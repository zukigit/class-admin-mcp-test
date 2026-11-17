#![allow(dead_code)]

use std::net::SocketAddr;
use std::sync::Arc;

use chrono::{Duration, Utc};
use class_admin::config::{BookingArgs, parse_quota};
use class_admin::db::{self, audit::Actor, classes, courses};
use class_admin::http::{self, HttpOptions};
use class_admin::mcp::AppState;
use class_admin::rate_limit::Limiters;
use serde_json::{Value, json};
use sqlx::SqlitePool;
use tokio_util::sync::CancellationToken;

pub const SECRET: &[u8] = b"test-secret-test-secret-test-secret-0123";

pub struct TestDb {
    pub pool: SqlitePool,
    _dir: tempfile::TempDir,
}

/// File-based database (so several connections can race, like in production).
pub async fn test_db() -> TestDb {
    let dir = tempfile::tempdir().unwrap();
    let pool = db::connect(&dir.path().join("test.db")).await.unwrap();
    TestDb { pool, _dir: dir }
}

pub fn admin() -> Actor {
    Actor {
        name: "tester".into(),
        token_id: Some("t".into()),
        ip: None,
    }
}

pub async fn course(pool: &SqlitePool, code: &str, status: &str) {
    courses::create(
        pool,
        &admin(),
        courses::NewCourse {
            code: code.into(),
            name: format!("Course {code}"),
            description: None,
            level: Some("beginner".into()),
            status: status.into(),
        },
    )
    .await
    .unwrap();
}

/// A class starting `start_in_days` from today (UTC).
pub async fn class(pool: &SqlitePool, course: &str, code: &str, start_in_days: i64, capacity: i64, status: &str) {
    let start = Utc::now().date_naive() + Duration::days(start_in_days);
    classes::create(
        pool,
        &admin(),
        classes::NewClass {
            course_code: course.into(),
            code: code.into(),
            teacher: Some("T".into()),
            term_start: start,
            term_end: start + Duration::days(30),
            timezone: chrono_tz::Tz::UTC,
            capacity,
            fee_cents: 12000,
            status: status.into(),
        },
    )
    .await
    .unwrap();
}

pub struct TestServer {
    pub base: String,
    pub pool: SqlitePool,
    pub client: reqwest::Client,
    cancel: CancellationToken,
    _db: TestDb,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

pub struct Limits {
    pub public: &'static str,
    pub booking: &'static str,
    pub trusted_proxy: &'static str,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            public: "1000/min",
            booking: "1000/min",
            trusted_proxy: "127.0.0.1",
        }
    }
}

pub async fn server(limits: Limits) -> TestServer {
    let db = test_db().await;
    let app = Arc::new(AppState {
        pool: db.pool.clone(),
        booking: BookingArgs::default(),
        limiters: Limiters::new(
            parse_quota(limits.public).unwrap(),
            parse_quota(limits.booking).unwrap(),
            parse_quota("1000/min").unwrap(),
            parse_quota("1000/min").unwrap(),
        ),
        jwt_secret: Some(SECRET.to_vec()),
    });
    let cancel = CancellationToken::new();
    let router = http::router(
        app,
        HttpOptions {
            allowed_hosts: vec!["127.0.0.1".into(), "localhost".into()],
            allowed_origins: vec![],
            trusted_proxy: limits.trusted_proxy.parse().unwrap(),
            cancel: cancel.clone(),
        },
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let ct = cancel.clone();
    tokio::spawn(async move {
        axum::serve(listener, router.into_make_service_with_connect_info::<SocketAddr>())
            .with_graceful_shutdown(async move { ct.cancelled().await })
            .await
            .unwrap();
    });
    TestServer {
        base: format!("http://{addr}"),
        pool: db.pool.clone(),
        client: reqwest::Client::new(),
        cancel,
        _db: db,
    }
}

pub struct Reply {
    pub status: u16,
    pub headers: reqwest::header::HeaderMap,
    pub body: Value,
}

impl Reply {
    /// Structured tool result.
    pub fn tool(&self) -> &Value {
        &self.body["result"]["structuredContent"]
    }
    pub fn is_tool_error(&self) -> bool {
        self.body["result"]["isError"] == json!(true)
    }
}

impl TestServer {
    pub async fn rpc(
        &self,
        path: &str,
        method: &str,
        params: Value,
        token: Option<&str>,
        extra: &[(&str, &str)],
    ) -> Reply {
        let mut req = self
            .client
            .post(format!("{}{path}", self.base))
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .header("mcp-protocol-version", "2025-06-18")
            .json(&json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }));
        if let Some(t) = token {
            req = req.bearer_auth(t);
        }
        for (k, v) in extra {
            req = req.header(*k, *v);
        }
        let res = req.send().await.unwrap();
        let status = res.status().as_u16();
        let headers = res.headers().clone();
        let text = res.text().await.unwrap();
        let body = serde_json::from_str(&text).unwrap_or(Value::String(text));
        Reply { status, headers, body }
    }

    pub async fn call(&self, path: &str, tool: &str, args: Value, token: Option<&str>) -> Reply {
        self.rpc(
            path,
            "tools/call",
            json!({ "name": tool, "arguments": args }),
            token,
            &[],
        )
        .await
    }

    pub async fn tool_names(&self, path: &str, token: Option<&str>) -> Vec<String> {
        let r = self.rpc(path, "tools/list", json!({}), token, &[]).await;
        r.body["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect()
    }
}

pub fn admin_token() -> String {
    class_admin::auth::jwt::issue(SECRET, "alice", 1).unwrap().0
}
