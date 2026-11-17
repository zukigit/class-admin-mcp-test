use serde_json::json;
use sqlx::{FromRow, SqlitePool};

use super::audit::{self, Actor};
use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, FromRow)]
pub struct CourseRow {
    pub id: i64,
    pub code: String,
    pub name: String,
    pub description: Option<String>,
    pub level: Option<String>,
    pub status: String,
}

const COLS: &str = "id, code, name, description, level, status";

pub async fn list(pool: &SqlitePool, published_only: bool, level: Option<&str>) -> sqlx::Result<Vec<CourseRow>> {
    sqlx::query_as::<_, CourseRow>(sql!(
        "SELECT {COLS} FROM courses
         WHERE (? = 0 OR status = 'published')
           AND (? IS NULL OR lower(level) = lower(?))
         ORDER BY name"
    ))
    .bind(published_only)
    .bind(level)
    .bind(level)
    .fetch_all(pool)
    .await
}

pub async fn get(pool: &SqlitePool, code: &str, published_only: bool) -> AppResult<CourseRow> {
    sqlx::query_as::<_, CourseRow>(sql!(
        "SELECT {COLS} FROM courses WHERE code = ? AND (? = 0 OR status = 'published')"
    ))
    .bind(code)
    .bind(published_only)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::not_found("course"))
}

pub struct NewCourse {
    pub code: String,
    pub name: String,
    pub description: Option<String>,
    pub level: Option<String>,
    pub status: String,
}

pub async fn create(pool: &SqlitePool, actor: &Actor, c: NewCourse) -> AppResult<CourseRow> {
    let mut tx = pool.begin().await?;
    let res = sqlx::query("INSERT INTO courses (code, name, description, level, status) VALUES (?, ?, ?, ?, ?)")
        .bind(&c.code)
        .bind(&c.name)
        .bind(&c.description)
        .bind(&c.level)
        .bind(&c.status)
        .execute(&mut *tx)
        .await;
    if let Err(sqlx::Error::Database(e)) = &res
        && e.is_unique_violation()
    {
        return Err(AppError::conflict("course code already exists"));
    }
    res?;
    audit::record(
        &mut tx,
        actor,
        "create_course",
        None,
        None,
        json!({ "code": c.code, "name": c.name, "status": c.status }),
    )
    .await?;
    tx.commit().await?;
    get(pool, &c.code, false).await
}

pub struct CourseUpdate {
    pub name: Option<String>,
    pub description: Option<String>,
    pub level: Option<String>,
    pub status: Option<String>,
}

pub async fn update(pool: &SqlitePool, actor: &Actor, code: &str, u: CourseUpdate) -> AppResult<CourseRow> {
    let mut tx = pool.begin().await?;
    let old = sqlx::query_as::<_, CourseRow>(sql!("SELECT {COLS} FROM courses WHERE code = ?"))
        .bind(code)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| AppError::not_found("course"))?;
    sqlx::query(
        "UPDATE courses SET
            name = COALESCE(?, name),
            description = COALESCE(?, description),
            level = COALESCE(?, level),
            status = COALESCE(?, status),
            updated_at = strftime('%Y-%m-%dT%H:%M:%SZ','now')
         WHERE id = ?",
    )
    .bind(&u.name)
    .bind(&u.description)
    .bind(&u.level)
    .bind(&u.status)
    .bind(old.id)
    .execute(&mut *tx)
    .await?;
    audit::record(
        &mut tx,
        actor,
        "update_course",
        None,
        None,
        json!({
            "code": code,
            "old": { "name": old.name, "description": old.description, "level": old.level, "status": old.status },
            "new": { "name": u.name, "description": u.description, "level": u.level, "status": u.status },
        }),
    )
    .await?;
    tx.commit().await?;
    get(pool, code, false).await
}
