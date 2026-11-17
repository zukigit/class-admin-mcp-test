use rmcp::handler::server::tool::Extension;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::{ErrorData, schemars, tool, tool_router};
use serde::Deserialize;
use serde_json::json;

use super::{McpServer, reply};
use crate::db::{bookings, classes, courses};
use crate::error::{AppError, AppResult};
use crate::rate_limit;
use crate::validate;

type Parts = http::request::Parts;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListCoursesArgs {
    /// Only courses of this level, e.g. "beginner".
    pub level: Option<String>,
    /// Max results (1-100, default 50).
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CodeArgs {
    /// Course or class code, e.g. "PY-BASICS".
    pub code: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListUpcomingArgs {
    /// Only classes of this course code.
    pub course_code: Option<String>,
    /// Only classes that meet on this weekday: 1 = Monday ... 7 = Sunday.
    pub day_of_week: Option<i64>,
    /// Only classes starting on or after this date (YYYY-MM-DD).
    pub from_date: Option<String>,
    /// Only classes starting on or before this date (YYYY-MM-DD).
    pub to_date: Option<String>,
    /// Only classes that can be booked right now (open, with spots left).
    pub only_available: Option<bool>,
    /// Only classes with a fee up to this amount, in USD cents (12000 = $120.00).
    pub max_fee_cents: Option<i64>,
    /// Max results (1-100, default 20).
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct BookArgs {
    /// Code of the class to book, e.g. "PY-BASICS-2027-01".
    pub class_code: String,
    /// Full name of the person attending (the person's own name).
    pub name: String,
    /// Email of the person attending (the person's own email).
    pub email: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct BookingLookupArgs {
    /// Booking reference, e.g. "BK-7F3K9Q2M".
    pub reference: String,
    /// Secret cancel code returned once by book_class.
    pub cancel_code: String,
}

#[tool_router(router = public_tools, vis = "pub(crate)")]
impl McpServer {
    #[tool(
        description = "List the courses the school teaches (published courses only), with how many upcoming classes each has.",
        annotations(title = "List courses", read_only_hint = true, open_world_hint = false)
    )]
    async fn list_courses(&self, Parameters(a): Parameters<ListCoursesArgs>) -> Result<CallToolResult, ErrorData> {
        reply(self.list_courses_impl(a).await)
    }

    #[tool(
        description = "Get one published course and its upcoming classes (schedule, fee in USD, spots left).",
        annotations(title = "Get course", read_only_hint = true, open_world_hint = false)
    )]
    async fn get_course(&self, Parameters(a): Parameters<CodeArgs>) -> Result<CallToolResult, ErrorData> {
        reply(self.get_course_impl(a).await)
    }

    #[tool(
        description = "List upcoming classes (not started yet): course, teacher, term dates, weekly sessions, fee in USD, capacity, spots_left and whether it can be booked now.",
        annotations(title = "List upcoming classes", read_only_hint = true, open_world_hint = false)
    )]
    async fn list_upcoming_classes(
        &self,
        Parameters(a): Parameters<ListUpcomingArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        reply(self.list_upcoming_impl(a).await)
    }

    #[tool(
        description = "Get one upcoming class by its code: term dates, weekly sessions, fee in USD, spots_left, bookable.",
        annotations(title = "Get class", read_only_hint = true, open_world_hint = false)
    )]
    async fn get_class(&self, Parameters(a): Parameters<CodeArgs>) -> Result<CallToolResult, ErrorData> {
        let pool = &self.state.pool;
        let res = async {
            let row = classes::get_public(pool, &validate::code("code", &a.code)?).await?;
            let s = classes::sessions(pool, row.id).await?;
            Ok(row.to_out(s, false))
        }
        .await;
        reply(res)
    }

    #[tool(
        description = "Book a seat in an upcoming class for the whole term. Only book for a real person who asked for it, \
using their own name and email. The booking is pending until the school approves it. \
The reply contains a booking reference and a secret cancel code that is shown only once: \
give both to the person and tell them to keep them; they are needed to check or cancel the booking.",
        annotations(
            title = "Book a class",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn book_class(
        &self,
        Parameters(a): Parameters<BookArgs>,
        Extension(parts): Extension<Parts>,
    ) -> Result<CallToolResult, ErrorData> {
        let caller = Self::caller(&parts);
        let res = async {
            if !caller.is_admin() && !rate_limit::allow(&self.state.limiters.booking, &caller.ip) {
                return Err(AppError::RateLimited);
            }
            let new = bookings::NewBooking {
                class_code: validate::code("class_code", &a.class_code)?,
                name: validate::person_name(&a.name)?,
                email: validate::email(&a.email)?,
                client_ip: Some(caller.ip.clone()),
            };
            let booked = bookings::book(&self.state.pool, &self.state.booking, new).await?;
            Ok(json!({
                "reference": booked.booking.reference,
                "cancel_code": booked.cancel_code,
                "status": "pending",
                "pending_until": booked.booking.pending_until,
                "class": booked.class.summary(),
                "fee": crate::model::Money::usd(booked.class.fee_cents),
                "message": "Booking received and waiting for approval by the school. \
Keep the reference and cancel code: they are shown only once and are needed to check or cancel this booking. \
If it is not approved before pending_until, it expires.",
            }))
        }
        .await;
        reply(res)
    }

    #[tool(
        description = "Check the status of your own booking (pending, confirmed, rejected, cancelled, expired) with its reference and cancel code.",
        annotations(title = "Check booking", read_only_hint = true, open_world_hint = false)
    )]
    async fn get_booking(&self, Parameters(a): Parameters<BookingLookupArgs>) -> Result<CallToolResult, ErrorData> {
        let res = async {
            let (b, class) = bookings::get_for_booker(&self.state.pool, &a.reference, &a.cancel_code).await?;
            Ok(b.to_public(&class))
        }
        .await;
        reply(res)
    }

    #[tool(
        description = "Cancel your own booking with its reference and cancel code. Works for pending or confirmed bookings until the class starts.",
        annotations(
            title = "Cancel booking",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn cancel_booking(
        &self,
        Parameters(a): Parameters<BookingLookupArgs>,
        Extension(parts): Extension<Parts>,
    ) -> Result<CallToolResult, ErrorData> {
        let caller = Self::caller(&parts);
        let res = async {
            let (b, class) =
                bookings::cancel_by_booker(&self.state.pool, &a.reference, &a.cancel_code, Some(caller.ip.clone()))
                    .await?;
            Ok(b.to_public(&class))
        }
        .await;
        reply(res)
    }

    #[tool(
        description = "Show whether you are connected as the public or as an admin.",
        annotations(title = "Who am I", read_only_hint = true, open_world_hint = false)
    )]
    async fn whoami(&self, Extension(parts): Extension<Parts>) -> Result<CallToolResult, ErrorData> {
        let caller = Self::caller(&parts);
        reply::<serde_json::Value>(Ok(match &caller.admin {
            Some(a) => json!({ "caller": "admin", "user": a.sub }),
            None => json!({ "caller": "public", "user": null }),
        }))
    }
}

impl McpServer {
    async fn list_courses_impl(&self, a: ListCoursesArgs) -> AppResult<serde_json::Value> {
        let pool = &self.state.pool;
        let rows = courses::list(pool, true, a.level.as_deref()).await?;
        let upcoming = classes::list_public(pool, None).await?;
        let limit = validate::limit(a.limit, 50) as usize;
        let offset = validate::offset(a.offset) as usize;
        let total = rows.len();
        let items: Vec<_> = rows
            .into_iter()
            .skip(offset)
            .take(limit)
            .map(|c| crate::model::CourseOut {
                upcoming_classes: upcoming.iter().filter(|u| u.course_id == c.id).count(),
                code: c.code,
                name: c.name,
                description: c.description,
                level: c.level,
                status: None,
            })
            .collect();
        Ok(json!({ "courses": items, "total": total }))
    }

    async fn get_course_impl(&self, a: CodeArgs) -> AppResult<serde_json::Value> {
        let pool = &self.state.pool;
        let course = courses::get(pool, &validate::code("code", &a.code)?, true).await?;
        let rows = classes::list_public(pool, Some(&course.code)).await?;
        let upcoming = classes::with_sessions(pool, rows, false).await?;
        Ok(json!({
            "course": crate::model::CourseOut {
                code: course.code,
                name: course.name,
                description: course.description,
                level: course.level,
                status: None,
                upcoming_classes: upcoming.len(),
            },
            "upcoming_classes": upcoming,
        }))
    }

    async fn list_upcoming_impl(&self, a: ListUpcomingArgs) -> AppResult<serde_json::Value> {
        let pool = &self.state.pool;
        let course = a
            .course_code
            .as_deref()
            .map(|c| validate::code("course_code", c))
            .transpose()?;
        let day = a.day_of_week.map(validate::day_of_week).transpose()?;
        let from = a
            .from_date
            .as_deref()
            .map(|d| validate::date("from_date", d))
            .transpose()?;
        let to = a.to_date.as_deref().map(|d| validate::date("to_date", d)).transpose()?;
        let limit = validate::limit(a.limit, 20) as usize;
        let offset = validate::offset(a.offset) as usize;

        let rows = classes::list_public(pool, course.as_deref()).await?;
        let mut items = classes::with_sessions(pool, rows, false).await?;
        items.retain(|c| {
            let start = chrono::NaiveDate::parse_from_str(&c.term_start, "%Y-%m-%d").ok();
            day.is_none_or(|d| c.sessions.iter().any(|s| s.day_of_week == d))
                && from.is_none_or(|f| start.is_some_and(|s| s >= f))
                && to.is_none_or(|t| start.is_some_and(|s| s <= t))
                && (!a.only_available.unwrap_or(false) || c.bookable)
                && a.max_fee_cents.is_none_or(|m| c.fee.cents <= m)
        });
        let total = items.len();
        let items: Vec<_> = items.into_iter().skip(offset).take(limit).collect();
        Ok(json!({ "classes": items, "total": total }))
    }
}
