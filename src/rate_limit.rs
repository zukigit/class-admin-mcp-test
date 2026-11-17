use governor::{DefaultKeyedRateLimiter, Quota, RateLimiter};

/// In-memory rate limiters, keyed by client IP or admin token id.
pub struct Limiters {
    pub public: DefaultKeyedRateLimiter<String>,
    pub booking: DefaultKeyedRateLimiter<String>,
    pub admin: DefaultKeyedRateLimiter<String>,
    pub auth_fail: DefaultKeyedRateLimiter<String>,
}

impl Limiters {
    pub fn new(public: Quota, booking: Quota, admin: Quota, auth_fail: Quota) -> Self {
        Self {
            public: RateLimiter::keyed(public),
            booking: RateLimiter::keyed(booking),
            admin: RateLimiter::keyed(admin),
            auth_fail: RateLimiter::keyed(auth_fail),
        }
    }

    /// Drops state for keys that are back to full quota, so memory stays small.
    pub fn cleanup(&self) {
        self.public.retain_recent();
        self.booking.retain_recent();
        self.admin.retain_recent();
        self.auth_fail.retain_recent();
    }
}

pub fn allow(limiter: &DefaultKeyedRateLimiter<String>, key: &str) -> bool {
    limiter.check_key(&key.to_string()).is_ok()
}
