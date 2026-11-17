use serde_json::json;
use sqlx::{FromRow, SqlitePool};

use super::audit::{self, Actor};

#[derive(Debug, Clone, FromRow)]
pub struct RevokedToken {
    pub jti: String,
    pub sub: String,
    pub expires_at: i64,
    pub revoked_at: String,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
pub struct RevokedUser {
    pub sub: String,
    pub revoked_before: i64,
    pub reason: Option<String>,
}

/// True if the token (by `jti`, or by `sub` + `iat`) has been revoked.
pub async fn is_revoked(pool: &SqlitePool, jti: &str, sub: &str, iat: i64) -> sqlx::Result<bool> {
    let revoked: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM revoked_tokens WHERE jti = ?)
             OR EXISTS (SELECT 1 FROM revoked_users WHERE sub = ? AND ? < revoked_before)",
    )
    .bind(jti)
    .bind(sub)
    .bind(iat)
    .fetch_one(pool)
    .await?;
    Ok(revoked)
}

pub async fn revoke_token(
    pool: &SqlitePool,
    jti: &str,
    sub: &str,
    expires_at: i64,
    reason: Option<&str>,
) -> sqlx::Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("INSERT OR REPLACE INTO revoked_tokens (jti, sub, expires_at, reason) VALUES (?, ?, ?, ?)")
        .bind(jti)
        .bind(sub)
        .bind(expires_at)
        .bind(reason)
        .execute(&mut *tx)
        .await?;
    audit::record(
        &mut tx,
        &Actor::cli(),
        "revoke_token",
        None,
        None,
        json!({ "jti": jti, "sub": sub, "reason": reason }),
    )
    .await?;
    tx.commit().await
}

/// Revokes every token of `sub` issued up to now. Tokens issued later still work.
pub async fn revoke_user(pool: &SqlitePool, sub: &str, reason: Option<&str>) -> sqlx::Result<i64> {
    // +1: a token issued in this same second is revoked too.
    let before = chrono::Utc::now().timestamp() + 1;
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO revoked_users (sub, revoked_before, reason) VALUES (?, ?, ?)
         ON CONFLICT(sub) DO UPDATE SET revoked_before = excluded.revoked_before, reason = excluded.reason",
    )
    .bind(sub)
    .bind(before)
    .bind(reason)
    .execute(&mut *tx)
    .await?;
    audit::record(
        &mut tx,
        &Actor::cli(),
        "revoke_user",
        None,
        None,
        json!({ "sub": sub, "revoked_before": before, "reason": reason }),
    )
    .await?;
    tx.commit().await?;
    Ok(before)
}

pub async fn list(pool: &SqlitePool) -> sqlx::Result<(Vec<RevokedToken>, Vec<RevokedUser>)> {
    let tokens = sqlx::query_as::<_, RevokedToken>(
        "SELECT jti, sub, expires_at, revoked_at, reason FROM revoked_tokens ORDER BY revoked_at DESC",
    )
    .fetch_all(pool)
    .await?;
    let users = sqlx::query_as::<_, RevokedUser>("SELECT sub, revoked_before, reason FROM revoked_users ORDER BY sub")
        .fetch_all(pool)
        .await?;
    Ok((tokens, users))
}

/// Deletes revoked-token rows whose tokens have expired anyway, and user
/// revocations older than the longest possible token lifetime.
pub async fn prune(pool: &SqlitePool, max_token_days: i64) -> sqlx::Result<u64> {
    let now = chrono::Utc::now().timestamp();
    let a = sqlx::query("DELETE FROM revoked_tokens WHERE expires_at < ?")
        .bind(now)
        .execute(pool)
        .await?
        .rows_affected();
    let b = sqlx::query("DELETE FROM revoked_users WHERE revoked_before < ?")
        .bind(now - max_token_days * 86_400)
        .execute(pool)
        .await?
        .rows_affected();
    Ok(a + b)
}
