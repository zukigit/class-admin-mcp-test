use std::net::{IpAddr, SocketAddr};
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, bail};
use clap::Args;
use governor::Quota;

/// Minimum length of the JWT signing secret.
pub const MIN_SECRET_LEN: usize = 32;

#[derive(Debug, Clone, Args)]
pub struct DbArgs {
    /// SQLite database file.
    #[arg(long, env = "CLASS_ADMIN_DB", default_value = "./data/class_admin.db")]
    pub db: PathBuf,
}

#[derive(Debug, Clone, Args)]
pub struct SecretArgs {
    /// Admin token signing secret (server only, at least 32 characters).
    #[arg(long, env = "CLASS_ADMIN_JWT_SECRET", hide_env_values = true)]
    pub jwt_secret: Option<String>,
}

impl SecretArgs {
    /// Returns the secret if it is set and long enough.
    pub fn checked(&self) -> anyhow::Result<Option<Vec<u8>>> {
        match &self.jwt_secret {
            None => Ok(None),
            Some(s) if s.trim().is_empty() => Ok(None),
            Some(s) if s.len() < MIN_SECRET_LEN => bail!(
                "CLASS_ADMIN_JWT_SECRET must be at least {MIN_SECRET_LEN} characters; \
                 create one with `class-admin secret generate`"
            ),
            Some(s) => Ok(Some(s.as_bytes().to_vec())),
        }
    }

    pub fn required(&self) -> anyhow::Result<Vec<u8>> {
        self.checked()?
            .context("CLASS_ADMIN_JWT_SECRET is not set; create one with `class-admin secret generate`")
    }
}

#[derive(Debug, Clone, Args)]
pub struct ServeArgs {
    #[command(flatten)]
    pub db: DbArgs,

    #[command(flatten)]
    pub secret: SecretArgs,

    /// Address to listen on. Put an HTTPS proxy in front when it is on the internet.
    #[arg(long, env = "CLASS_ADMIN_BIND", default_value = "127.0.0.1:8080")]
    pub bind: SocketAddr,

    /// Public URL of the server, e.g. https://classes.example.com.
    /// Its host is added to the allowed Host headers.
    #[arg(long, env = "CLASS_ADMIN_PUBLIC_URL")]
    pub public_url: Option<String>,

    /// Allowed browser Origin values, comma separated (e.g. https://app.example.com:443).
    #[arg(long, env = "CLASS_ADMIN_ALLOWED_ORIGINS", value_delimiter = ',', default_value = "")]
    pub allowed_origins: Vec<String>,

    /// Only this proxy's X-Forwarded-For header is trusted for client IPs.
    #[arg(long, env = "CLASS_ADMIN_TRUSTED_PROXY", default_value = "127.0.0.1")]
    pub trusted_proxy: IpAddr,

    /// Per-IP limit for all public calls, e.g. 60/min.
    #[arg(long, env = "CLASS_ADMIN_RATE_LIMIT_PUBLIC", default_value = "60/min", value_parser = parse_quota)]
    pub rate_limit_public: Quota,

    /// Per-IP limit for book_class, e.g. 5/hour.
    #[arg(long, env = "CLASS_ADMIN_RATE_LIMIT_BOOKING", default_value = "5/hour", value_parser = parse_quota)]
    pub rate_limit_booking: Quota,

    /// Per-token limit for admins, e.g. 300/min.
    #[arg(long, env = "CLASS_ADMIN_RATE_LIMIT_ADMIN", default_value = "300/min", value_parser = parse_quota)]
    pub rate_limit_admin: Quota,

    /// Per-IP limit for failed token checks, e.g. 10/min.
    #[arg(long, env = "CLASS_ADMIN_RATE_LIMIT_AUTH_FAIL", default_value = "10/min", value_parser = parse_quota)]
    pub rate_limit_auth_fail: Quota,

    #[command(flatten)]
    pub booking: BookingArgs,
}

#[derive(Debug, Clone, Args)]
pub struct BookingArgs {
    /// Max pending bookings per email, across all classes.
    #[arg(long, env = "CLASS_ADMIN_MAX_PENDING_PER_EMAIL", default_value_t = 3)]
    pub max_pending_per_email: i64,

    /// Pending bookings expire after this many hours.
    #[arg(long, env = "CLASS_ADMIN_PENDING_TTL_HOURS", default_value_t = 72)]
    pub pending_ttl_hours: i64,

    /// Days after a class's term end before booker names and emails are erased.
    #[arg(long, env = "CLASS_ADMIN_BOOKING_RETENTION_DAYS", default_value_t = 365)]
    pub booking_retention_days: i64,
}

impl Default for BookingArgs {
    fn default() -> Self {
        Self {
            max_pending_per_email: 3,
            pending_ttl_hours: 72,
            booking_retention_days: 365,
        }
    }
}

impl ServeArgs {
    /// Hosts accepted in the Host header (protection against DNS rebinding).
    pub fn allowed_hosts(&self) -> anyhow::Result<Vec<String>> {
        let mut hosts = vec!["localhost".to_string(), "127.0.0.1".to_string(), "::1".to_string()];
        if let Some(url) = &self.public_url {
            let uri: http::Uri = url.parse().context("CLASS_ADMIN_PUBLIC_URL is not a valid URL")?;
            let host = uri.host().context("CLASS_ADMIN_PUBLIC_URL has no host")?;
            hosts.push(host.to_string());
        }
        Ok(hosts)
    }

    pub fn origins(&self) -> Vec<String> {
        self.allowed_origins
            .iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }
}

pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
pub const MAX_BODY_BYTES: usize = 1024 * 1024;

/// Parses limits such as `60/min`, `5/hour`, `10/sec`.
pub fn parse_quota(s: &str) -> Result<Quota, String> {
    let (n, unit) = s.split_once('/').ok_or("expected a limit like 60/min")?;
    let n: u32 = n.trim().parse().map_err(|_| "limit count must be a number")?;
    let n = NonZeroU32::new(n).ok_or("limit count must be greater than 0")?;
    match unit.trim() {
        "s" | "sec" | "second" => Ok(Quota::per_second(n)),
        "m" | "min" | "minute" => Ok(Quota::per_minute(n)),
        "h" | "hour" => Ok(Quota::per_hour(n)),
        _ => Err("limit unit must be sec, min or hour".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotas() {
        assert!(parse_quota("60/min").is_ok());
        assert!(parse_quota("5/hour").is_ok());
        assert!(parse_quota("0/min").is_err());
        assert!(parse_quota("5/day").is_err());
        assert!(parse_quota("five/min").is_err());
    }
}
