use chrono::{Duration, NaiveTime, TimeZone, Utc};
use rand::Rng;
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::{FromRow, SqliteConnection, SqlitePool};
use subtle::ConstantTimeEq;

use super::audit::{self, Actor};
use super::classes::{self, ClassRow};
use super::{now_ts, ts};
use crate::config::BookingArgs;
use crate::error::{AppError, AppResult};
use crate::model::{BookingAdminOut, BookingPublicOut, UntrustedBooker};

#[derive(Debug, Clone, FromRow)]
pub struct BookingRow {
    pub id: i64,
    pub reference: String,
    pub cancel_code_hash: String,
    pub class_id: i64,
    pub class_code: String,
    pub name: Option<String>,
    pub email: Option<String>,
    pub status: String,
    pub pending_until: String,
    pub decided_by: Option<String>,
    pub decided_at: Option<String>,
    pub reason: Option<String>,
    pub created_at: String,
}

const SELECT: &str = "SELECT b.id, b.reference, b.cancel_code_hash, b.class_id, c.code AS class_code,
    b.name, b.email, b.status, b.pending_until, b.decided_by, b.decided_at, b.reason, b.created_at
    FROM bookings b JOIN classes c ON c.id = b.class_id";

impl BookingRow {
    /// A pending booking whose time ran out counts as expired, even before the job runs.
    pub fn effective_status(&self) -> &str {
        if self.status == "pending" && self.pending_until <= now_ts() {
            "expired"
        } else {
            &self.status
        }
    }

    pub fn to_public(&self, class: &ClassRow) -> BookingPublicOut {
        let status = self.effective_status().to_string();
        BookingPublicOut {
            reference: self.reference.clone(),
            pending_until: (status == "pending").then(|| self.pending_until.clone()),
            status,
            class: class.summary(),
            reason: self.reason.clone(),
        }
    }

    pub fn to_admin(&self) -> BookingAdminOut {
        BookingAdminOut {
            reference: self.reference.clone(),
            class_code: self.class_code.clone(),
            status: self.effective_status().to_string(),
            booker_untrusted: UntrustedBooker {
                name: self.name.clone(),
                email: self.email.clone(),
            },
            pending_until: self.pending_until.clone(),
            created_at: self.created_at.clone(),
            decided_by: self.decided_by.clone(),
            decided_at: self.decided_at.clone(),
            reason: self.reason.clone(),
        }
    }
}

/// Unambiguous characters (no 0/O, 1/I/L) for references people may read out.
const REF_ALPHABET: &[u8] = b"23456789ABCDEFGHJKMNPQRSTUVWXYZ";

fn new_reference() -> String {
    let mut rng = rand::rng();
    let s: String = (0..8)
        .map(|_| REF_ALPHABET[rng.random_range(0..REF_ALPHABET.len())] as char)
        .collect();
    format!("BK-{s}")
}

/// 128-bit random cancel code, shown once to the booker.
fn new_cancel_code() -> String {
    let bytes: [u8; 16] = rand::rng().random();
    hex::encode(bytes)
}

pub fn hash_code(code: &str) -> String {
    hex::encode(Sha256::digest(code.trim().to_ascii_lowercase().as_bytes()))
}

fn code_matches(code: &str, stored_hash: &str) -> bool {
    hash_code(code).as_bytes().ct_eq(stored_hash.as_bytes()).into()
}

pub struct NewBooking {
    pub class_code: String,
    pub name: String,
    pub email: String,
    pub client_ip: Option<String>,
}

pub struct Booked {
    pub booking: BookingRow,
    pub class: ClassRow,
    pub cancel_code: String,
}

/// Books a seat without ever overbooking.
///
/// `BEGIN IMMEDIATE` takes SQLite's write lock up front, so two bookings can't
/// both read "1 seat left" and both insert.
pub async fn book(pool: &SqlitePool, settings: &BookingArgs, b: NewBooking) -> AppResult<Booked> {
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    expire_due_conn(&mut tx).await?;

    let class = match classes::get_admin_conn(&mut tx, &b.class_code).await {
        Ok(c) if c.is_public() => c,
        _ => return Err(AppError::not_found("class")),
    };
    if class.status != "open" {
        return Err(AppError::conflict("this class is not taking bookings"));
    }
    if class.spots_left() <= 0 {
        return Err(AppError::conflict("class is full"));
    }

    let pending_for_email: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM bookings WHERE email = ? AND status = 'pending'")
            .bind(&b.email)
            .fetch_one(&mut *tx)
            .await?;
    if pending_for_email >= settings.max_pending_per_email {
        return Err(AppError::conflict(
            "this email already has the maximum number of bookings waiting for approval",
        ));
    }

    // Pending until the TTL runs out or the class starts, whichever is first.
    let ttl_end = Utc::now() + Duration::hours(settings.pending_ttl_hours);
    let class_start = class
        .term_start_date()
        .and_then(|d| class.tz().from_local_datetime(&d.and_time(NaiveTime::MIN)).earliest())
        .map(|t| t.with_timezone(&Utc));
    let pending_until = ts(class_start.map_or(ttl_end, |s| s.min(ttl_end)));

    let cancel_code = new_cancel_code();
    let code_hash = hash_code(&cancel_code);
    let mut inserted = None;
    for _ in 0..5 {
        let reference = new_reference();
        let res = sqlx::query(
            "INSERT INTO bookings (reference, cancel_code_hash, class_id, name, email, status, pending_until, client_ip)
             VALUES (?, ?, ?, ?, ?, 'pending', ?, ?)",
        )
        .bind(&reference)
        .bind(&code_hash)
        .bind(class.id)
        .bind(&b.name)
        .bind(&b.email)
        .bind(&pending_until)
        .bind(&b.client_ip)
        .execute(&mut *tx)
        .await;
        match res {
            Ok(r) => {
                inserted = Some((r.last_insert_rowid(), reference));
                break;
            }
            Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
                if e.message().contains("bookings.reference") {
                    continue; // reference collision: try another one
                }
                return Err(AppError::conflict("this email already has a booking for this class"));
            }
            Err(e) => return Err(e.into()),
        }
    }
    let (booking_id, reference) =
        inserted.ok_or_else(|| AppError::Internal(anyhow::anyhow!("could not create a unique booking reference")))?;

    let actor = Actor {
        name: "public".into(),
        token_id: None,
        ip: b.client_ip.clone(),
    };
    audit::record(
        &mut tx,
        &actor,
        "book_class",
        Some(class.id),
        Some(booking_id),
        json!({ "reference": reference, "class": class.code, "pending_until": pending_until }),
    )
    .await?;
    tx.commit().await?;

    let booking = get_by_reference(pool, &reference).await?;
    let class = classes::get_admin(pool, &class.code).await?;
    Ok(Booked {
        booking,
        class,
        cancel_code,
    })
}

async fn get_by_reference_conn(conn: &mut SqliteConnection, reference: &str) -> AppResult<BookingRow> {
    sqlx::query_as::<_, BookingRow>(sql!("{SELECT} WHERE b.reference = ?"))
        .bind(reference.trim().to_ascii_uppercase())
        .fetch_optional(conn)
        .await?
        .ok_or_else(|| AppError::not_found("booking"))
}

pub async fn get_by_reference(pool: &SqlitePool, reference: &str) -> AppResult<BookingRow> {
    get_by_reference_conn(&mut *pool.acquire().await?, reference).await
}

/// Looks up a booking for its booker. A wrong reference and a wrong code give the
/// same "booking not found", so nobody can probe which references exist.
pub async fn get_for_booker(pool: &SqlitePool, reference: &str, code: &str) -> AppResult<(BookingRow, ClassRow)> {
    let mut conn = pool.acquire().await?;
    let booking = match get_by_reference_conn(&mut conn, reference).await {
        Ok(b) if code_matches(code, &b.cancel_code_hash) => b,
        _ => return Err(AppError::not_found("booking")),
    };
    let class = classes::get_by_id_conn(&mut conn, booking.class_id).await?;
    Ok((booking, class))
}

/// Booker cancels their own booking, before the class starts.
pub async fn cancel_by_booker(
    pool: &SqlitePool,
    reference: &str,
    code: &str,
    ip: Option<String>,
) -> AppResult<(BookingRow, ClassRow)> {
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let booking = match get_by_reference_conn(&mut tx, reference).await {
        Ok(b) if code_matches(code, &b.cancel_code_hash) => b,
        _ => return Err(AppError::not_found("booking")),
    };
    let class = classes::get_by_id_conn(&mut tx, booking.class_id).await?;
    if !class.is_upcoming() {
        return Err(AppError::conflict(
            "the class has already started; contact the school to cancel",
        ));
    }
    if !matches!(booking.effective_status(), "pending" | "confirmed") {
        return Err(AppError::conflict(format!(
            "booking is already {}",
            booking.effective_status()
        )));
    }
    set_status(&mut tx, booking.id, "cancelled", "public", None).await?;
    let actor = Actor {
        name: "public".into(),
        token_id: None,
        ip,
    };
    audit::record(
        &mut tx,
        &actor,
        "cancel_booking",
        Some(class.id),
        Some(booking.id),
        json!({ "reference": booking.reference, "old_status": booking.status }),
    )
    .await?;
    tx.commit().await?;
    Ok((get_by_reference(pool, reference).await?, class))
}

async fn set_status(
    conn: &mut SqliteConnection,
    id: i64,
    status: &str,
    by: &str,
    reason: Option<&str>,
) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE bookings SET status = ?, decided_by = ?, decided_at = strftime('%Y-%m-%dT%H:%M:%SZ','now'),
            reason = COALESCE(?, reason) WHERE id = ?",
    )
    .bind(status)
    .bind(by)
    .bind(reason)
    .bind(id)
    .execute(conn)
    .await?;
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AdminDecision {
    Approve,
    Reject,
    Cancel,
}

/// Admin approves, rejects or cancels one booking.
pub async fn decide(
    pool: &SqlitePool,
    actor: &Actor,
    reference: &str,
    decision: AdminDecision,
    reason: Option<String>,
) -> AppResult<BookingRow> {
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let booking = get_by_reference_conn(&mut tx, reference).await?;
    let current = booking.effective_status().to_string();
    let (allowed, new_status, action) = match decision {
        AdminDecision::Approve => (current == "pending", "confirmed", "approve_booking"),
        AdminDecision::Reject => (current == "pending", "rejected", "reject_booking"),
        AdminDecision::Cancel => (
            matches!(current.as_str(), "pending" | "confirmed"),
            "cancelled",
            "admin_cancel_booking",
        ),
    };
    if !allowed {
        return Err(AppError::conflict(format!(
            "booking is {current} and can't be changed this way"
        )));
    }
    set_status(&mut tx, booking.id, new_status, &actor.name, reason.as_deref()).await?;
    audit::record(
        &mut tx,
        actor,
        action,
        Some(booking.class_id),
        Some(booking.id),
        json!({ "reference": booking.reference, "old_status": current, "new_status": new_status, "reason": reason }),
    )
    .await?;
    tx.commit().await?;
    get_by_reference(pool, reference).await
}

pub async fn list(
    pool: &SqlitePool,
    class_code: Option<&str>,
    status: Option<&str>,
    limit: i64,
    offset: i64,
) -> sqlx::Result<Vec<BookingRow>> {
    expire_due(pool).await?;
    sqlx::query_as::<_, BookingRow>(sql!(
        "{SELECT} WHERE (? IS NULL OR c.code = ?) AND (? IS NULL OR b.status = ?)
         ORDER BY b.created_at DESC, b.id DESC LIMIT ? OFFSET ?"
    ))
    .bind(class_code)
    .bind(class_code)
    .bind(status)
    .bind(status)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await
}

async fn expire_due_conn(conn: &mut SqliteConnection) -> sqlx::Result<u64> {
    let due: Vec<(i64, i64, String)> = sqlx::query_as(
        "SELECT id, class_id, reference FROM bookings
         WHERE status = 'pending' AND pending_until <= strftime('%Y-%m-%dT%H:%M:%SZ','now')",
    )
    .fetch_all(&mut *conn)
    .await?;
    for (id, class_id, reference) in &due {
        set_status(conn, *id, "expired", "system", Some("not approved in time")).await?;
        audit::record(
            conn,
            &Actor::system(),
            "expire_booking",
            Some(*class_id),
            Some(*id),
            json!({ "reference": reference }),
        )
        .await?;
    }
    Ok(due.len() as u64)
}

/// Marks overdue pending bookings as expired. Returns how many.
pub async fn expire_due(pool: &SqlitePool) -> sqlx::Result<u64> {
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let n = expire_due_conn(&mut tx).await?;
    tx.commit().await?;
    Ok(n)
}

/// Erases booker names and emails `retention_days` after the class's term ends.
pub async fn purge_personal_data(pool: &SqlitePool, retention_days: i64) -> sqlx::Result<u64> {
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let n = sqlx::query(
        "UPDATE bookings SET name = NULL, email = NULL
         WHERE (name IS NOT NULL OR email IS NOT NULL)
           AND class_id IN (SELECT id FROM classes WHERE date(term_end, '+' || ? || ' days') < date('now'))",
    )
    .bind(retention_days)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if n > 0 {
        audit::record(
            &mut tx,
            &Actor::system(),
            "purge_personal_data",
            None,
            None,
            json!({ "bookings": n, "retention_days": retention_days }),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn references_and_codes() {
        let r = new_reference();
        assert!(r.starts_with("BK-") && r.len() == 11);
        let code = new_cancel_code();
        assert_eq!(code.len(), 32);
        let h = hash_code(&code);
        assert!(code_matches(&code, &h));
        assert!(code_matches(&format!(" {} ", code.to_uppercase()), &h));
        assert!(!code_matches("wrong", &h));
    }
}
