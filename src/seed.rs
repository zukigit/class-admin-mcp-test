//! Sample data for development.

use chrono::{Duration, Utc};
use sqlx::SqlitePool;

use crate::db::audit::Actor;
use crate::db::{classes, courses};
use crate::validate;

pub async fn run(pool: &SqlitePool) -> anyhow::Result<()> {
    let actor = Actor::cli();
    let today = Utc::now().date_naive();
    let samples = [
        (
            "PY-BASICS",
            "Python Basics",
            "Learn programming from zero with Python.",
            "beginner",
        ),
        (
            "WEB-DEV",
            "Web Development",
            "HTML, CSS and JavaScript to build your first website.",
            "beginner",
        ),
        (
            "DATA-SQL",
            "Data Analysis with SQL",
            "Query and analyse data with SQL.",
            "intermediate",
        ),
    ];
    for (code, name, desc, level) in samples {
        if courses::get(pool, code, false).await.is_ok() {
            println!("course {code} already exists, skipping");
            continue;
        }
        courses::create(
            pool,
            &actor,
            courses::NewCourse {
                code: code.into(),
                name: name.into(),
                description: Some(desc.into()),
                level: Some(level.into()),
                status: "published".into(),
            },
        )
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        println!("created course {code}");
    }

    let runs = [
        (
            "PY-BASICS",
            "PY-BASICS-A",
            "Ada",
            14,
            70,
            10,
            12000,
            vec![(1, "18:00", "20:00"), (3, "18:00", "20:00")],
        ),
        (
            "PY-BASICS",
            "PY-BASICS-B",
            "Grace",
            45,
            100,
            3,
            12000,
            vec![(6, "10:00", "13:00")],
        ),
        (
            "WEB-DEV",
            "WEB-DEV-A",
            "Tim",
            21,
            90,
            12,
            15000,
            vec![(2, "19:00", "21:00"), (4, "19:00", "21:00")],
        ),
        (
            "DATA-SQL",
            "DATA-SQL-A",
            "Edgar",
            30,
            60,
            8,
            18000,
            vec![(5, "18:30", "21:00")],
        ),
    ];
    for (course, code, teacher, start_in, length, capacity, fee, sessions) in runs {
        if classes::get_admin(pool, code).await.is_ok() {
            println!("class {code} already exists, skipping");
            continue;
        }
        let start = today + Duration::days(start_in);
        classes::create(
            pool,
            &actor,
            classes::NewClass {
                course_code: course.into(),
                code: code.into(),
                teacher: Some(teacher.into()),
                term_start: start,
                term_end: start + Duration::days(length),
                timezone: chrono_tz::Tz::UTC,
                capacity,
                fee_cents: fee,
                status: "open".into(),
            },
        )
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        let items: Vec<_> = sessions
            .into_iter()
            .map(|(d, a, b)| (d, a.to_string(), b.to_string()))
            .collect();
        let s = validate::sessions(&items).map_err(|e| anyhow::anyhow!(e.to_string()))?;
        classes::set_schedule(pool, &actor, code, s)
            .await
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        println!("created class {code}");
    }
    Ok(())
}
