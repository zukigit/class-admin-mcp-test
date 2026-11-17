mod common;

use class_admin::auth::jwt;
use class_admin::db::revocations;
use common::*;
use serde_json::json;

const ADMIN_TOOLS: &[&str] = &[
    "create_course",
    "update_course",
    "list_all_classes",
    "create_class",
    "update_class",
    "set_class_schedule",
    "set_class_fee",
    "cancel_class",
    "delete_class",
    "list_bookings",
    "approve_booking",
    "reject_booking",
    "admin_cancel_booking",
];

async fn seeded(limits: Limits) -> TestServer {
    let s = server(limits).await;
    course(&s.pool, "PY", "published").await;
    class(&s.pool, "PY", "PY-A", 10, 5, "open").await;
    s
}

#[tokio::test]
async fn public_sees_only_public_tools_and_cannot_call_admin_tools() {
    let s = seeded(Limits::default()).await;
    let names = s.tool_names("/mcp", None).await;
    assert!(names.contains(&"book_class".to_string()));
    for t in ADMIN_TOOLS {
        assert!(!names.contains(&t.to_string()), "{t} visible to public");
    }

    for (tool, args) in [
        ("set_class_fee", json!({ "code": "PY-A", "fee_cents": 1 })),
        ("cancel_class", json!({ "code": "PY-A" })),
        ("create_course", json!({ "code": "X", "name": "X" })),
        ("list_bookings", json!({})),
    ] {
        let r = s.call("/mcp", tool, args, None).await;
        assert!(r.is_tool_error(), "{tool}: {:?}", r.body);
        assert_eq!(r.tool()["error"], "permission denied: this action requires an admin");
    }
    let fee: i64 = sqlx::query_scalar("SELECT fee_cents FROM classes WHERE code = 'PY-A'")
        .fetch_one(&s.pool)
        .await
        .unwrap();
    assert_eq!(fee, 12000, "DB must be unchanged");
    let status: String = sqlx::query_scalar("SELECT status FROM classes WHERE code = 'PY-A'")
        .fetch_one(&s.pool)
        .await
        .unwrap();
    assert_eq!(status, "open");
}

#[tokio::test]
async fn admin_endpoint_and_bad_tokens() {
    let s = seeded(Limits::default()).await;
    let r = s.rpc("/mcp/admin", "tools/list", json!({}), None, &[]).await;
    assert_eq!(r.status, 401);
    assert!(r.headers.get("www-authenticate").is_some());

    for bad in [
        "garbage",
        &jwt::issue(b"some-other-secret-some-other-secret", "alice", 1)
            .unwrap()
            .0,
    ] {
        assert_eq!(s.rpc("/mcp", "tools/list", json!({}), Some(bad), &[]).await.status, 401);
        assert_eq!(
            s.rpc("/mcp/admin", "tools/list", json!({}), Some(bad), &[])
                .await
                .status,
            401
        );
    }

    let token = admin_token();
    let names = s.tool_names("/mcp/admin", Some(&token)).await;
    for t in ADMIN_TOOLS {
        assert!(names.contains(&t.to_string()), "{t} missing for admin");
    }
    let who = s.call("/mcp/admin", "whoami", json!({}), Some(&token)).await;
    assert_eq!(who.tool()["caller"], "admin");
}

#[tokio::test]
async fn revoked_tokens_are_refused_immediately() {
    let s = seeded(Limits::default()).await;
    let (token, claims) = jwt::issue(SECRET, "alice", 1).unwrap();
    assert_eq!(
        s.rpc("/mcp/admin", "tools/list", json!({}), Some(&token), &[])
            .await
            .status,
        200
    );
    revocations::revoke_token(&s.pool, &claims.jti, &claims.sub, claims.exp, None)
        .await
        .unwrap();
    assert_eq!(
        s.rpc("/mcp/admin", "tools/list", json!({}), Some(&token), &[])
            .await
            .status,
        401
    );

    let (t2, _) = jwt::issue(SECRET, "bob", 1).unwrap();
    revocations::revoke_user(&s.pool, "bob", None).await.unwrap();
    assert_eq!(
        s.rpc("/mcp/admin", "tools/list", json!({}), Some(&t2), &[])
            .await
            .status,
        401
    );
    // A token issued after the revocation works (iat must be later than revoked_before).
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    let (t3, _) = jwt::issue(SECRET, "bob", 1).unwrap();
    assert_eq!(
        s.rpc("/mcp/admin", "tools/list", json!({}), Some(&t3), &[])
            .await
            .status,
        200
    );
}

#[tokio::test]
async fn full_booking_flow_over_http() {
    let s = seeded(Limits::default()).await;
    let r = s
        .call("/mcp", "list_upcoming_classes", json!({ "only_available": true }), None)
        .await;
    assert_eq!(r.tool()["total"], 1);
    assert_eq!(r.tool()["classes"][0]["spots_left"], 5);

    let booked = s
        .call(
            "/mcp",
            "book_class",
            json!({ "class_code": "py-a", "name": "Ada", "email": "ada@example.com" }),
            None,
        )
        .await;
    assert!(!booked.is_tool_error(), "{:?}", booked.body);
    let reference = booked.tool()["reference"].as_str().unwrap().to_string();
    let code = booked.tool()["cancel_code"].as_str().unwrap().to_string();

    let check = s
        .call(
            "/mcp",
            "get_booking",
            json!({ "reference": reference, "cancel_code": code }),
            None,
        )
        .await;
    assert_eq!(check.tool()["status"], "pending");
    let text = check.body.to_string();
    assert!(
        !text.contains("ada@example.com") && !text.contains("\"Ada\""),
        "booker data leaked: {text}"
    );

    let token = admin_token();
    let list = s
        .call(
            "/mcp/admin",
            "list_bookings",
            json!({ "status": "pending" }),
            Some(&token),
        )
        .await;
    assert_eq!(
        list.tool()["bookings"][0]["booker_untrusted"]["email"],
        "ada@example.com"
    );
    let ok = s
        .call(
            "/mcp/admin",
            "approve_booking",
            json!({ "reference": reference }),
            Some(&token),
        )
        .await;
    assert_eq!(ok.tool()["status"], "confirmed");

    let check = s
        .call(
            "/mcp",
            "get_booking",
            json!({ "reference": reference, "cancel_code": code }),
            None,
        )
        .await;
    assert_eq!(check.tool()["status"], "confirmed");
    let cancel = s
        .call(
            "/mcp",
            "cancel_booking",
            json!({ "reference": reference, "cancel_code": code }),
            None,
        )
        .await;
    assert_eq!(cancel.tool()["status"], "cancelled");

    let audit: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM audit_log WHERE action IN ('book_class','approve_booking','cancel_booking')",
    )
    .fetch_one(&s.pool)
    .await
    .unwrap();
    assert_eq!(audit, 3);
    let actor: String = sqlx::query_scalar("SELECT actor FROM audit_log WHERE action = 'approve_booking'")
        .fetch_one(&s.pool)
        .await
        .unwrap();
    assert_eq!(actor, "alice");
}

#[tokio::test]
async fn admin_manages_classes_over_http() {
    let s = seeded(Limits::default()).await;
    let t = admin_token();
    let r = s
        .call(
            "/mcp/admin",
            "set_class_schedule",
            json!({ "code": "PY-A", "sessions": [
                { "day_of_week": 1, "start_time": "18:00", "end_time": "20:00" },
                { "day_of_week": 3, "start_time": "18:00", "end_time": "20:00" }
            ]}),
            Some(&t),
        )
        .await;
    assert_eq!(r.tool()["sessions"].as_array().unwrap().len(), 2, "{:?}", r.body);
    let overlap = s
        .call(
            "/mcp/admin",
            "set_class_schedule",
            json!({ "code": "PY-A", "sessions": [
                { "day_of_week": 1, "start_time": "18:00", "end_time": "20:00" },
                { "day_of_week": 1, "start_time": "19:00", "end_time": "21:00" }
            ]}),
            Some(&t),
        )
        .await;
    assert!(overlap.is_tool_error());

    let fee = s
        .call(
            "/mcp/admin",
            "set_class_fee",
            json!({ "code": "PY-A", "fee_cents": 15050 }),
            Some(&t),
        )
        .await;
    assert_eq!(fee.tool()["fee"]["display"], "$150.50");
    let public = s.call("/mcp", "get_class", json!({ "code": "PY-A" }), None).await;
    assert_eq!(public.tool()["fee"]["cents"], 15050);
    assert!(
        public.tool().get("booking_counts").is_none(),
        "admin-only field shown to public"
    );
}

#[tokio::test]
async fn origin_check_rejects_unknown_browser_origins() {
    let s = seeded(Limits::default()).await;
    let r = s
        .rpc(
            "/mcp",
            "tools/list",
            json!({}),
            None,
            &[("origin", "https://evil.example")],
        )
        .await;
    assert_eq!(r.status, 403);
}

#[tokio::test]
async fn public_rate_limit() {
    let s = seeded(Limits {
        public: "3/min",
        ..Default::default()
    })
    .await;
    for _ in 0..3 {
        assert_eq!(s.rpc("/mcp", "tools/list", json!({}), None, &[]).await.status, 200);
    }
    assert_eq!(s.rpc("/mcp", "tools/list", json!({}), None, &[]).await.status, 429);
}

#[tokio::test]
async fn booking_rate_limit_cannot_be_bypassed_with_fake_forwarded_for() {
    // The test client connects from 127.0.0.1, which is NOT the trusted proxy here,
    // so X-Forwarded-For must be ignored.
    let s = seeded(Limits {
        booking: "2/hour",
        trusted_proxy: "10.0.0.1",
        ..Default::default()
    })
    .await;
    for i in 0..3 {
        let fake_ip = format!("203.0.113.{i}");
        let r = s
            .rpc(
                "/mcp",
                "tools/call",
                json!({ "name": "book_class", "arguments": { "class_code": "PY-A", "name": "X", "email": format!("x{i}@example.com") } }),
                None,
                &[("x-forwarded-for", fake_ip.as_str())],
            )
            .await;
        if i < 2 {
            assert!(!r.is_tool_error(), "{:?}", r.body);
        } else {
            assert_eq!(r.tool()["error"], "too many requests, please try again later");
        }
    }
    let ips: Vec<String> = sqlx::query_scalar("SELECT DISTINCT client_ip FROM bookings")
        .fetch_all(&s.pool)
        .await
        .unwrap();
    assert_eq!(ips, vec!["127.0.0.1"]);
}

#[tokio::test]
async fn errors_do_not_leak_internals() {
    let s = seeded(Limits::default()).await;
    let r = s.call("/mcp", "get_class", json!({ "code": "NOPE" }), None).await;
    assert_eq!(r.tool()["error"], "class not found");
    let r = s
        .call(
            "/mcp",
            "book_class",
            json!({ "class_code": "PY-A", "name": "", "email": "bad" }),
            None,
        )
        .await;
    let text = r.body.to_string().to_lowercase();
    assert!(r.is_tool_error());
    for word in ["sqlite", "select", "panicked", "/src/", "constraint"] {
        assert!(!text.contains(word), "internal detail '{word}' in {text}");
    }
}
