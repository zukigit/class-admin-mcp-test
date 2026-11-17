mod common;

use class_admin::config::BookingArgs;
use class_admin::db::bookings::{self, AdminDecision, NewBooking};
use class_admin::db::{classes, courses};
use class_admin::error::AppError;
use common::*;

fn new_booking(class: &str, email: &str) -> NewBooking {
    NewBooking {
        class_code: class.into(),
        name: "Test Person".into(),
        email: email.into(),
        client_ip: None,
    }
}

/// Many people try to book the last seats at the same moment: exactly `capacity` succeed.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_bookings_never_overbook() {
    let db = test_db().await;
    course(&db.pool, "C1", "published").await;
    class(&db.pool, "C1", "C1-A", 10, 10, "open").await;
    let settings = BookingArgs::default();

    let mut handles = Vec::new();
    for i in 0..50 {
        let pool = db.pool.clone();
        let settings = settings.clone();
        handles.push(tokio::spawn(async move {
            bookings::book(&pool, &settings, new_booking("C1-A", &format!("p{i}@example.com"))).await
        }));
    }
    let mut ok = 0;
    let mut full = 0;
    for h in handles {
        match h.await.unwrap() {
            Ok(_) => ok += 1,
            Err(AppError::Conflict(m)) if m == "class is full" => full += 1,
            Err(e) => panic!("unexpected error: {e:?}"),
        }
    }
    assert_eq!((ok, full), (10, 40));
    let c = classes::get_admin(&db.pool, "C1-A").await.unwrap();
    assert_eq!(c.spots_left(), 0);
    assert!(!c.bookable());
}

#[tokio::test]
async fn one_active_booking_per_email_and_rebook_after_cancel() {
    let db = test_db().await;
    course(&db.pool, "C1", "published").await;
    class(&db.pool, "C1", "C1-A", 10, 5, "open").await;
    let s = BookingArgs::default();

    let first = bookings::book(&db.pool, &s, new_booking("C1-A", "a@example.com"))
        .await
        .unwrap();
    let dup = bookings::book(&db.pool, &s, new_booking("C1-A", "a@example.com")).await;
    assert!(matches!(dup, Err(AppError::Conflict(_))));

    bookings::cancel_by_booker(&db.pool, &first.booking.reference, &first.cancel_code, None)
        .await
        .unwrap();
    bookings::book(&db.pool, &s, new_booking("C1-A", "a@example.com"))
        .await
        .unwrap();
}

#[tokio::test]
async fn pending_limit_per_email() {
    let db = test_db().await;
    course(&db.pool, "C1", "published").await;
    for c in ["C1-A", "C1-B", "C1-C", "C1-D"] {
        class(&db.pool, "C1", c, 10, 5, "open").await;
    }
    let s = BookingArgs {
        max_pending_per_email: 3,
        ..Default::default()
    };
    for c in ["C1-A", "C1-B", "C1-C"] {
        bookings::book(&db.pool, &s, new_booking(c, "spam@example.com"))
            .await
            .unwrap();
    }
    let fourth = bookings::book(&db.pool, &s, new_booking("C1-D", "spam@example.com")).await;
    assert!(matches!(fourth, Err(AppError::Conflict(_))));
}

#[tokio::test]
async fn expired_pending_frees_seat_and_cannot_be_approved() {
    let db = test_db().await;
    course(&db.pool, "C1", "published").await;
    class(&db.pool, "C1", "C1-A", 10, 1, "open").await;
    let s = BookingArgs::default();

    let b = bookings::book(&db.pool, &s, new_booking("C1-A", "a@example.com"))
        .await
        .unwrap();
    assert!(matches!(
        bookings::book(&db.pool, &s, new_booking("C1-A", "b@example.com")).await,
        Err(AppError::Conflict(m)) if m == "class is full"
    ));

    sqlx::query("UPDATE bookings SET pending_until = '2000-01-01T00:00:00Z'")
        .execute(&db.pool)
        .await
        .unwrap();
    // Seat counts as free immediately, even before the job runs.
    assert_eq!(classes::get_admin(&db.pool, "C1-A").await.unwrap().spots_left(), 1);
    let approve = bookings::decide(&db.pool, &admin(), &b.booking.reference, AdminDecision::Approve, None).await;
    assert!(matches!(approve, Err(AppError::Conflict(_))));

    assert_eq!(bookings::expire_due(&db.pool).await.unwrap(), 1);
    let row = bookings::get_by_reference(&db.pool, &b.booking.reference)
        .await
        .unwrap();
    assert_eq!(row.status, "expired");
    bookings::book(&db.pool, &s, new_booking("C1-A", "b@example.com"))
        .await
        .unwrap();
}

#[tokio::test]
async fn approve_reject_and_capacity_rules() {
    let db = test_db().await;
    course(&db.pool, "C1", "published").await;
    class(&db.pool, "C1", "C1-A", 10, 3, "open").await;
    let s = BookingArgs::default();
    let a = bookings::book(&db.pool, &s, new_booking("C1-A", "a@example.com"))
        .await
        .unwrap();
    let b = bookings::book(&db.pool, &s, new_booking("C1-A", "b@example.com"))
        .await
        .unwrap();

    let a2 = bookings::decide(&db.pool, &admin(), &a.booking.reference, AdminDecision::Approve, None)
        .await
        .unwrap();
    assert_eq!(a2.status, "confirmed");
    let b2 = bookings::decide(
        &db.pool,
        &admin(),
        &b.booking.reference,
        AdminDecision::Reject,
        Some("full".into()),
    )
    .await
    .unwrap();
    assert_eq!(b2.status, "rejected");
    // Rejecting a confirmed booking is not allowed; cancelling is.
    assert!(
        bookings::decide(&db.pool, &admin(), &a.booking.reference, AdminDecision::Reject, None)
            .await
            .is_err()
    );

    // 1 active booking: capacity 1 is OK, 0 is impossible anyway; lower than active is refused.
    let c = bookings::book(&db.pool, &s, new_booking("C1-A", "c@example.com"))
        .await
        .unwrap();
    let too_low = classes::update(
        &db.pool,
        &admin(),
        "C1-A",
        classes::ClassUpdate {
            capacity: Some(1),
            ..Default::default()
        },
    )
    .await;
    assert!(matches!(too_low, Err(AppError::Conflict(_))));
    classes::update(
        &db.pool,
        &admin(),
        "C1-A",
        classes::ClassUpdate {
            capacity: Some(2),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let _ = c;
}

#[tokio::test]
async fn self_cancel_needs_right_code_and_closes_when_class_starts() {
    let db = test_db().await;
    course(&db.pool, "C1", "published").await;
    class(&db.pool, "C1", "C1-A", 10, 3, "open").await;
    let s = BookingArgs::default();
    let b = bookings::book(&db.pool, &s, new_booking("C1-A", "a@example.com"))
        .await
        .unwrap();

    let wrong = bookings::cancel_by_booker(&db.pool, &b.booking.reference, "wrong", None).await;
    assert!(matches!(wrong, Err(AppError::NotFound(m)) if m == "booking not found"));
    let unknown = bookings::get_for_booker(&db.pool, "BK-XXXXXXXX", &b.cancel_code).await;
    assert!(matches!(unknown, Err(AppError::NotFound(m)) if m == "booking not found"));

    sqlx::query("UPDATE classes SET term_start = date('now', '-1 day')")
        .execute(&db.pool)
        .await
        .unwrap();
    let started = bookings::cancel_by_booker(&db.pool, &b.booking.reference, &b.cancel_code, None).await;
    assert!(matches!(started, Err(AppError::Conflict(_))));
}

#[tokio::test]
async fn public_visibility() {
    let db = test_db().await;
    course(&db.pool, "PUB", "published").await;
    course(&db.pool, "DRAFT", "draft").await;
    class(&db.pool, "PUB", "OPEN", 10, 5, "open").await;
    class(&db.pool, "PUB", "CLOSED", 10, 5, "closed").await;
    class(&db.pool, "PUB", "HIDDEN", 10, 5, "draft").await;
    class(&db.pool, "PUB", "STARTED", -1, 5, "draft").await;
    class(&db.pool, "DRAFT", "OF-DRAFT", 10, 5, "open").await;
    class(&db.pool, "PUB", "WILL-CANCEL", 10, 5, "open").await;
    classes::cancel(&db.pool, &admin(), "WILL-CANCEL", None).await.unwrap();
    // A class that was open but has started since.
    sqlx::query("UPDATE classes SET status = 'open' WHERE code = 'STARTED'")
        .execute(&db.pool)
        .await
        .unwrap();

    let visible: Vec<_> = classes::list_public(&db.pool, None)
        .await
        .unwrap()
        .into_iter()
        .map(|c| c.code)
        .collect();
    assert_eq!(visible, vec!["CLOSED", "OPEN"]);
    for code in ["HIDDEN", "STARTED", "OF-DRAFT", "WILL-CANCEL"] {
        assert!(
            classes::get_public(&db.pool, code).await.is_err(),
            "{code} must be hidden"
        );
    }
    assert!(!classes::get_public(&db.pool, "CLOSED").await.unwrap().bookable());
    let closed = bookings::book(
        &db.pool,
        &BookingArgs::default(),
        new_booking("CLOSED", "a@example.com"),
    )
    .await;
    assert!(matches!(closed, Err(AppError::Conflict(_))));
    let hidden = bookings::book(
        &db.pool,
        &BookingArgs::default(),
        new_booking("HIDDEN", "a@example.com"),
    )
    .await;
    assert!(matches!(hidden, Err(AppError::NotFound(_))));

    let courses: Vec<_> = courses::list(&db.pool, true, None)
        .await
        .unwrap()
        .into_iter()
        .map(|c| c.code)
        .collect();
    assert_eq!(courses, vec!["PUB"]);
}

#[tokio::test]
async fn cancel_class_cancels_bookings_and_delete_needs_no_bookings() {
    let db = test_db().await;
    course(&db.pool, "C1", "published").await;
    class(&db.pool, "C1", "C1-A", 10, 3, "open").await;
    class(&db.pool, "C1", "C1-B", 10, 3, "open").await;
    let s = BookingArgs::default();
    bookings::book(&db.pool, &s, new_booking("C1-A", "a@example.com"))
        .await
        .unwrap();

    assert!(matches!(
        classes::delete(&db.pool, &admin(), "C1-A").await,
        Err(AppError::Conflict(_))
    ));
    let (_, n) = classes::cancel(&db.pool, &admin(), "C1-A", Some("teacher ill".into()))
        .await
        .unwrap();
    assert_eq!(n, 1);
    classes::delete(&db.pool, &admin(), "C1-B").await.unwrap();
    assert!(classes::get_admin(&db.pool, "C1-B").await.is_err());
}

#[tokio::test]
async fn retention_erases_personal_data_only_after_period() {
    let db = test_db().await;
    course(&db.pool, "C1", "published").await;
    class(&db.pool, "C1", "C1-A", 10, 3, "open").await;
    let b = bookings::book(&db.pool, &BookingArgs::default(), new_booking("C1-A", "a@example.com"))
        .await
        .unwrap();

    assert_eq!(bookings::purge_personal_data(&db.pool, 365).await.unwrap(), 0);
    sqlx::query("UPDATE classes SET term_start = date('now','-400 days'), term_end = date('now','-366 days')")
        .execute(&db.pool)
        .await
        .unwrap();
    assert_eq!(bookings::purge_personal_data(&db.pool, 365).await.unwrap(), 1);
    let row = bookings::get_by_reference(&db.pool, &b.booking.reference)
        .await
        .unwrap();
    assert_eq!((row.name, row.email), (None, None));
    assert_eq!(row.status, "pending"); // statuses stay for statistics
}

#[tokio::test]
async fn audit_log_has_no_emails_or_codes() {
    let db = test_db().await;
    course(&db.pool, "C1", "published").await;
    class(&db.pool, "C1", "C1-A", 10, 3, "open").await;
    let b = bookings::book(
        &db.pool,
        &BookingArgs::default(),
        new_booking("C1-A", "secret.person@example.com"),
    )
    .await
    .unwrap();
    bookings::cancel_by_booker(&db.pool, &b.booking.reference, &b.cancel_code, None)
        .await
        .unwrap();
    let rows: Vec<String> = sqlx::query_scalar("SELECT detail_json FROM audit_log")
        .fetch_all(&db.pool)
        .await
        .unwrap();
    assert!(rows.len() >= 3);
    for r in rows {
        assert!(!r.contains("secret.person"), "email leaked into audit log: {r}");
        assert!(!r.contains(&b.cancel_code), "cancel code leaked into audit log: {r}");
    }
}
