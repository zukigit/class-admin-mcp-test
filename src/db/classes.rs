use chrono::NaiveDate;
use chrono_tz::Tz;
use serde_json::json;
use sqlx::{FromRow, SqliteConnection, SqlitePool};

use super::audit::{self, Actor};
use crate::error::{AppError, AppResult};
use crate::model::{BookingCounts, ClassOut, ClassSummary, Money, SessionOut, day_name, today_in};
use crate::validate::SessionInput;

#[derive(Debug, Clone, FromRow)]
pub struct ClassRow {
    pub id: i64,
    pub code: String,
    pub course_id: i64,
    pub course_code: String,
    pub course_name: String,
    pub course_status: String,
    pub teacher: Option<String>,
    pub term_start: String,
    pub term_end: String,
    pub timezone: String,
    pub capacity: i64,
    pub fee_cents: i64,
    pub status: String,
    /// Pending bookings that have not expired yet.
    pub pending: i64,
    pub confirmed: i64,
    pub rejected: i64,
    pub cancelled: i64,
    pub expired: i64,
}

#[derive(Debug, Clone, FromRow)]
pub struct SessionRow {
    pub day_of_week: i64,
    pub start_time: String,
    pub end_time: String,
}

/// Base query. Pending bookings past their `pending_until` already count as
/// free seats, even before the background job marks them expired.
const SELECT: &str = "
SELECT c.id, c.code, c.course_id, co.code AS course_code, co.name AS course_name,
       co.status AS course_status, c.teacher, c.term_start, c.term_end, c.timezone,
       c.capacity, c.fee_cents, c.status,
       (SELECT COUNT(*) FROM bookings b WHERE b.class_id = c.id AND b.status = 'pending'
          AND b.pending_until > strftime('%Y-%m-%dT%H:%M:%SZ','now')) AS pending,
       (SELECT COUNT(*) FROM bookings b WHERE b.class_id = c.id AND b.status = 'confirmed') AS confirmed,
       (SELECT COUNT(*) FROM bookings b WHERE b.class_id = c.id AND b.status = 'rejected') AS rejected,
       (SELECT COUNT(*) FROM bookings b WHERE b.class_id = c.id AND b.status = 'cancelled') AS cancelled,
       (SELECT COUNT(*) FROM bookings b WHERE b.class_id = c.id AND
          (b.status = 'expired' OR (b.status = 'pending'
           AND b.pending_until <= strftime('%Y-%m-%dT%H:%M:%SZ','now')))) AS expired
FROM classes c JOIN courses co ON co.id = c.course_id";

/// Public visibility in SQL: visible status, published course, and not clearly in the past.
/// (`-1 day` leaves room for time zones; the exact "upcoming" check is `is_upcoming`.)
const PUBLIC_FILTER: &str = "c.status IN ('open','closed') AND co.status = 'published'
    AND c.term_start >= date('now', '-1 day')";

impl ClassRow {
    pub fn tz(&self) -> Tz {
        self.timezone.parse().unwrap_or(Tz::UTC)
    }

    pub fn term_start_date(&self) -> Option<NaiveDate> {
        NaiveDate::parse_from_str(&self.term_start, "%Y-%m-%d").ok()
    }

    /// The class has not started yet, in its own time zone.
    pub fn is_upcoming(&self) -> bool {
        self.term_start_date().is_some_and(|start| today_in(self.tz()) < start)
    }

    pub fn is_public(&self) -> bool {
        matches!(self.status.as_str(), "open" | "closed") && self.course_status == "published" && self.is_upcoming()
    }

    pub fn active_bookings(&self) -> i64 {
        self.pending + self.confirmed
    }

    pub fn spots_left(&self) -> i64 {
        (self.capacity - self.active_bookings()).max(0)
    }

    pub fn bookable(&self) -> bool {
        self.status == "open" && self.is_public() && self.spots_left() > 0
    }

    pub fn summary(&self) -> ClassSummary {
        ClassSummary {
            code: self.code.clone(),
            course_name: self.course_name.clone(),
            term_start: self.term_start.clone(),
            term_end: self.term_end.clone(),
        }
    }

    pub fn to_out(&self, sessions: Vec<SessionRow>, admin: bool) -> ClassOut {
        ClassOut {
            code: self.code.clone(),
            course_code: self.course_code.clone(),
            course_name: self.course_name.clone(),
            teacher: self.teacher.clone(),
            term_start: self.term_start.clone(),
            term_end: self.term_end.clone(),
            timezone: self.timezone.clone(),
            sessions: sessions
                .into_iter()
                .map(|s| SessionOut {
                    day_of_week: s.day_of_week,
                    day: day_name(s.day_of_week),
                    start_time: s.start_time,
                    end_time: s.end_time,
                })
                .collect(),
            fee: Money::usd(self.fee_cents),
            capacity: self.capacity,
            spots_left: self.spots_left(),
            bookable: self.bookable(),
            status: self.status.clone(),
            booking_counts: admin.then_some(BookingCounts {
                pending: self.pending,
                confirmed: self.confirmed,
                rejected: self.rejected,
                cancelled: self.cancelled,
                expired: self.expired,
            }),
        }
    }
}

pub async fn sessions(pool: &SqlitePool, class_id: i64) -> sqlx::Result<Vec<SessionRow>> {
    sqlx::query_as::<_, SessionRow>(
        "SELECT day_of_week, start_time, end_time FROM class_sessions
         WHERE class_id = ? ORDER BY day_of_week, start_time",
    )
    .bind(class_id)
    .fetch_all(pool)
    .await
}

/// Rows turned into output, with their sessions.
pub async fn with_sessions(pool: &SqlitePool, rows: Vec<ClassRow>, admin: bool) -> sqlx::Result<Vec<ClassOut>> {
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let s = sessions(pool, row.id).await?;
        out.push(row.to_out(s, admin));
    }
    Ok(out)
}

/// Upcoming classes the public may see, ordered by start date.
pub async fn list_public(pool: &SqlitePool, course_code: Option<&str>) -> sqlx::Result<Vec<ClassRow>> {
    let rows = sqlx::query_as::<_, ClassRow>(sql!(
        "{SELECT} WHERE {PUBLIC_FILTER} AND (? IS NULL OR co.code = ?)
         ORDER BY c.term_start, c.code"
    ))
    .bind(course_code)
    .bind(course_code)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().filter(ClassRow::is_public).collect())
}

/// One class the public may see; anything else is "not found".
pub async fn get_public(pool: &SqlitePool, code: &str) -> AppResult<ClassRow> {
    sqlx::query_as::<_, ClassRow>(sql!("{SELECT} WHERE {PUBLIC_FILTER} AND c.code = ?"))
        .bind(code)
        .fetch_optional(pool)
        .await?
        .filter(ClassRow::is_public)
        .ok_or_else(|| AppError::not_found("class"))
}

pub async fn list_admin(
    pool: &SqlitePool,
    course_code: Option<&str>,
    status: Option<&str>,
    include_past: bool,
) -> sqlx::Result<Vec<ClassRow>> {
    let rows = sqlx::query_as::<_, ClassRow>(sql!(
        "{SELECT} WHERE (? IS NULL OR co.code = ?) AND (? IS NULL OR c.status = ?)
         ORDER BY c.term_start DESC, c.code"
    ))
    .bind(course_code)
    .bind(course_code)
    .bind(status)
    .bind(status)
    .fetch_all(pool)
    .await?;
    let today = chrono::Utc::now().date_naive();
    Ok(rows
        .into_iter()
        .filter(|r| include_past || NaiveDate::parse_from_str(&r.term_end, "%Y-%m-%d").is_ok_and(|end| end >= today))
        .collect())
}

pub async fn get_admin(pool: &SqlitePool, code: &str) -> AppResult<ClassRow> {
    get_admin_conn(&mut *pool.acquire().await?, code).await
}

pub async fn get_admin_conn(conn: &mut SqliteConnection, code: &str) -> AppResult<ClassRow> {
    sqlx::query_as::<_, ClassRow>(sql!("{SELECT} WHERE c.code = ?"))
        .bind(code)
        .fetch_optional(conn)
        .await?
        .ok_or_else(|| AppError::not_found("class"))
}

pub async fn get_by_id_conn(conn: &mut SqliteConnection, id: i64) -> AppResult<ClassRow> {
    sqlx::query_as::<_, ClassRow>(sql!("{SELECT} WHERE c.id = ?"))
        .bind(id)
        .fetch_optional(conn)
        .await?
        .ok_or_else(|| AppError::not_found("class"))
}

fn require_future_start(start: NaiveDate, tz: Tz) -> AppResult<()> {
    if start <= today_in(tz) {
        return Err(AppError::invalid(
            "an open or closed class must start in the future (use status draft for past classes)",
        ));
    }
    Ok(())
}

pub struct NewClass {
    pub course_code: String,
    pub code: String,
    pub teacher: Option<String>,
    pub term_start: NaiveDate,
    pub term_end: NaiveDate,
    pub timezone: Tz,
    pub capacity: i64,
    pub fee_cents: i64,
    pub status: String,
}

pub async fn create(pool: &SqlitePool, actor: &Actor, c: NewClass) -> AppResult<ClassRow> {
    if c.status == "open" || c.status == "closed" {
        require_future_start(c.term_start, c.timezone)?;
    }
    let mut tx = pool.begin().await?;
    let course_id: i64 = sqlx::query_scalar("SELECT id FROM courses WHERE code = ?")
        .bind(&c.course_code)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| AppError::not_found("course"))?;
    let res = sqlx::query(
        "INSERT INTO classes (course_id, code, teacher, term_start, term_end, timezone, capacity, fee_cents, status)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(course_id)
    .bind(&c.code)
    .bind(&c.teacher)
    .bind(c.term_start.to_string())
    .bind(c.term_end.to_string())
    .bind(c.timezone.name())
    .bind(c.capacity)
    .bind(c.fee_cents)
    .bind(&c.status)
    .execute(&mut *tx)
    .await;
    if let Err(sqlx::Error::Database(e)) = &res
        && e.is_unique_violation()
    {
        return Err(AppError::conflict("class code already exists"));
    }
    let class_id = res?.last_insert_rowid();
    audit::record(
        &mut tx,
        actor,
        "create_class",
        Some(class_id),
        None,
        json!({
            "code": c.code, "course": c.course_code, "term_start": c.term_start, "term_end": c.term_end,
            "timezone": c.timezone.name(), "capacity": c.capacity, "fee_cents": c.fee_cents, "status": c.status,
        }),
    )
    .await?;
    tx.commit().await?;
    get_admin(pool, &c.code).await
}

#[derive(Default)]
pub struct ClassUpdate {
    pub teacher: Option<String>,
    pub term_start: Option<NaiveDate>,
    pub term_end: Option<NaiveDate>,
    pub timezone: Option<Tz>,
    pub capacity: Option<i64>,
    pub status: Option<String>,
}

pub async fn update(pool: &SqlitePool, actor: &Actor, code: &str, u: ClassUpdate) -> AppResult<ClassRow> {
    if u.status.as_deref() == Some("cancelled") {
        return Err(AppError::invalid("use cancel_class to cancel a class"));
    }
    // IMMEDIATE: capacity is checked against bookings, which must not change meanwhile.
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let old = get_admin_conn(&mut tx, code).await?;
    if old.status == "cancelled" {
        return Err(AppError::conflict("class is cancelled and can't be changed"));
    }

    let start = u
        .term_start
        .or(old.term_start_date())
        .ok_or_else(|| AppError::invalid("bad term_start"))?;
    let end = match u.term_end {
        Some(d) => d,
        None => NaiveDate::parse_from_str(&old.term_end, "%Y-%m-%d").map_err(|_| AppError::invalid("bad term_end"))?,
    };
    crate::validate::term(start, end)?;
    let tz = u.timezone.unwrap_or(old.tz());
    let status = u.status.clone().unwrap_or(old.status.clone());
    let becomes_public = (status == "open" || status == "closed")
        && (u.status.is_some() || u.term_start.is_some() || u.timezone.is_some());
    if becomes_public {
        require_future_start(start, tz)?;
    }
    if let Some(cap) = u.capacity
        && cap < old.active_bookings()
    {
        return Err(AppError::conflict(format!(
            "capacity can't be lower than the {} active (pending + confirmed) bookings",
            old.active_bookings()
        )));
    }

    sqlx::query(
        "UPDATE classes SET teacher = COALESCE(?, teacher), term_start = ?, term_end = ?, timezone = ?,
            capacity = COALESCE(?, capacity), status = ?,
            updated_at = strftime('%Y-%m-%dT%H:%M:%SZ','now')
         WHERE id = ?",
    )
    .bind(&u.teacher)
    .bind(start.to_string())
    .bind(end.to_string())
    .bind(tz.name())
    .bind(u.capacity)
    .bind(&status)
    .bind(old.id)
    .execute(&mut *tx)
    .await?;
    audit::record(
        &mut tx,
        actor,
        "update_class",
        Some(old.id),
        None,
        json!({
            "code": code,
            "old": { "teacher": old.teacher, "term_start": old.term_start, "term_end": old.term_end,
                     "timezone": old.timezone, "capacity": old.capacity, "status": old.status },
            "new": { "teacher": u.teacher, "term_start": start, "term_end": end,
                     "timezone": tz.name(), "capacity": u.capacity, "status": status },
        }),
    )
    .await?;
    tx.commit().await?;
    get_admin(pool, code).await
}

pub async fn set_schedule(
    pool: &SqlitePool,
    actor: &Actor,
    code: &str,
    sessions: Vec<SessionInput>,
) -> AppResult<ClassRow> {
    let mut tx = pool.begin().await?;
    let class = get_admin_conn(&mut tx, code).await?;
    let old: Vec<SessionRow> = sqlx::query_as(
        "SELECT day_of_week, start_time, end_time FROM class_sessions WHERE class_id = ? ORDER BY day_of_week, start_time",
    )
    .bind(class.id)
    .fetch_all(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM class_sessions WHERE class_id = ?")
        .bind(class.id)
        .execute(&mut *tx)
        .await?;
    for s in &sessions {
        sqlx::query("INSERT INTO class_sessions (class_id, day_of_week, start_time, end_time) VALUES (?, ?, ?, ?)")
            .bind(class.id)
            .bind(s.day_of_week)
            .bind(s.start.format("%H:%M").to_string())
            .bind(s.end.format("%H:%M").to_string())
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("UPDATE classes SET updated_at = strftime('%Y-%m-%dT%H:%M:%SZ','now') WHERE id = ?")
        .bind(class.id)
        .execute(&mut *tx)
        .await?;
    let fmt = |d: i64, a: &str, b: &str| format!("{} {a}-{b}", day_name(d));
    audit::record(
        &mut tx,
        actor,
        "set_class_schedule",
        Some(class.id),
        None,
        json!({
            "code": code,
            "old": old.iter().map(|s| fmt(s.day_of_week, &s.start_time, &s.end_time)).collect::<Vec<_>>(),
            "new": sessions.iter().map(|s| fmt(s.day_of_week, &s.start.format("%H:%M").to_string(), &s.end.format("%H:%M").to_string())).collect::<Vec<_>>(),
        }),
    )
    .await?;
    tx.commit().await?;
    get_admin(pool, code).await
}

pub async fn set_fee(pool: &SqlitePool, actor: &Actor, code: &str, fee_cents: i64) -> AppResult<ClassRow> {
    let mut tx = pool.begin().await?;
    let class = get_admin_conn(&mut tx, code).await?;
    sqlx::query("UPDATE classes SET fee_cents = ?, updated_at = strftime('%Y-%m-%dT%H:%M:%SZ','now') WHERE id = ?")
        .bind(fee_cents)
        .bind(class.id)
        .execute(&mut *tx)
        .await?;
    audit::record(
        &mut tx,
        actor,
        "set_class_fee",
        Some(class.id),
        None,
        json!({ "code": code, "old_fee_cents": class.fee_cents, "new_fee_cents": fee_cents }),
    )
    .await?;
    tx.commit().await?;
    get_admin(pool, code).await
}

/// Cancels the class and all its active bookings. Returns the number of bookings cancelled.
pub async fn cancel(
    pool: &SqlitePool,
    actor: &Actor,
    code: &str,
    reason: Option<String>,
) -> AppResult<(ClassRow, u64)> {
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let class = get_admin_conn(&mut tx, code).await?;
    if class.status == "cancelled" {
        return Err(AppError::conflict("class is already cancelled"));
    }
    sqlx::query(
        "UPDATE classes SET status = 'cancelled', updated_at = strftime('%Y-%m-%dT%H:%M:%SZ','now') WHERE id = ?",
    )
    .bind(class.id)
    .execute(&mut *tx)
    .await?;
    let n = sqlx::query(
        "UPDATE bookings SET status = 'cancelled', decided_by = ?, decided_at = strftime('%Y-%m-%dT%H:%M:%SZ','now'),
            reason = COALESCE(?, 'class cancelled')
         WHERE class_id = ? AND status IN ('pending','confirmed')",
    )
    .bind(&actor.name)
    .bind(&reason)
    .bind(class.id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    audit::record(
        &mut tx,
        actor,
        "cancel_class",
        Some(class.id),
        None,
        json!({ "code": code, "old_status": class.status, "bookings_cancelled": n, "reason": reason }),
    )
    .await?;
    tx.commit().await?;
    Ok((get_admin(pool, code).await?, n))
}

pub async fn delete(pool: &SqlitePool, actor: &Actor, code: &str) -> AppResult<()> {
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let class = get_admin_conn(&mut tx, code).await?;
    let bookings: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM bookings WHERE class_id = ?")
        .bind(class.id)
        .fetch_one(&mut *tx)
        .await?;
    if bookings > 0 {
        return Err(AppError::conflict(
            "class has bookings and can't be deleted; use cancel_class instead",
        ));
    }
    sqlx::query("DELETE FROM classes WHERE id = ?")
        .bind(class.id)
        .execute(&mut *tx)
        .await?;
    audit::record(
        &mut tx,
        actor,
        "delete_class",
        Some(class.id),
        None,
        json!({ "code": code, "course": class.course_code, "term_start": class.term_start }),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}
