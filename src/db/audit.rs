use serde_json::Value;
use sqlx::SqliteConnection;

/// Who did something, for the audit log.
#[derive(Debug, Clone)]
pub struct Actor {
    /// Admin `sub`, or "public", "system", "cli".
    pub name: String,
    pub token_id: Option<String>,
    pub ip: Option<String>,
}

impl Actor {
    pub fn system() -> Self {
        Self {
            name: "system".into(),
            token_id: None,
            ip: None,
        }
    }
    pub fn cli() -> Self {
        Self {
            name: "cli".into(),
            token_id: None,
            ip: None,
        }
    }
}

/// Writes one audit row. `detail` must never contain emails or cancel codes.
pub async fn record(
    conn: &mut SqliteConnection,
    actor: &Actor,
    action: &str,
    class_id: Option<i64>,
    booking_id: Option<i64>,
    detail: Value,
) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO audit_log (actor, token_id, action, class_id, booking_id, detail_json, client_ip)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&actor.name)
    .bind(&actor.token_id)
    .bind(action)
    .bind(class_id)
    .bind(booking_id)
    .bind(detail.to_string())
    .bind(&actor.ip)
    .execute(conn)
    .await?;
    Ok(())
}
