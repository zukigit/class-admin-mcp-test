//! Background maintenance while the server runs.

use std::sync::Arc;
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use crate::auth::jwt::MAX_TOKEN_DAYS;
use crate::db::{bookings, revocations};
use crate::mcp::AppState;

/// Every minute: expire overdue pending bookings and trim rate-limit memory.
/// Every hour: erase old booker data and prune old revocations.
pub fn spawn(app: Arc<AppState>, cancel: CancellationToken) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(60));
        let mut n: u64 = 0;
        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = tick.tick() => {}
            }
            match bookings::expire_due(&app.pool).await {
                Ok(0) => {}
                Ok(k) => tracing::info!(expired = k, "expired pending bookings"),
                Err(e) => tracing::error!(error = ?e, "booking expiry failed"),
            }
            app.limiters.cleanup();
            if n.is_multiple_of(60) {
                match bookings::purge_personal_data(&app.pool, app.booking.booking_retention_days).await {
                    Ok(0) => {}
                    Ok(k) => tracing::info!(bookings = k, "erased booker personal data after retention period"),
                    Err(e) => tracing::error!(error = ?e, "personal data clean-up failed"),
                }
                if let Err(e) = revocations::prune(&app.pool, MAX_TOKEN_DAYS).await {
                    tracing::error!(error = ?e, "revocation prune failed");
                }
            }
            n += 1;
        }
    });
}
