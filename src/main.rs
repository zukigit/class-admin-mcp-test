use std::sync::Arc;

use anyhow::Context;
use clap::{Parser, Subcommand};
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

use class_admin::auth::{self, jwt};
use class_admin::config::{BookingArgs, DbArgs, SecretArgs, ServeArgs};
use class_admin::db::{self, bookings, revocations};
use class_admin::http::{self, HttpOptions};
use class_admin::mcp::AppState;
use class_admin::rate_limit::Limiters;
use class_admin::{jobs, seed};

#[derive(Parser)]
#[command(name = "class-admin", version, about = "MCP server for browsing and booking classes")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the HTTP MCP server.
    Serve(ServeArgs),
    /// Signing secret commands.
    #[command(subcommand)]
    Secret(SecretCmd),
    /// Admin token commands.
    #[command(subcommand)]
    Token(TokenCmd),
    /// Revocation list commands.
    #[command(subcommand)]
    Revoke(RevokeCmd),
    /// Booking maintenance commands.
    #[command(subcommand)]
    Bookings(BookingsCmd),
    /// Add sample courses and classes for development.
    Seed(DbArgs),
}

#[derive(Subcommand)]
enum SecretCmd {
    /// Print a new random signing secret.
    Generate,
}

#[derive(Subcommand)]
enum TokenCmd {
    /// Issue an admin token.
    Issue {
        /// Admin name, written to the audit log.
        #[arg(long)]
        user: String,
        /// Lifetime in days (1-90).
        #[arg(long, default_value_t = jwt::DEFAULT_TOKEN_DAYS)]
        days: i64,
        #[command(flatten)]
        secret: SecretArgs,
    },
    /// Show a token's claims and whether it is valid.
    Inspect {
        token: String,
        #[command(flatten)]
        secret: SecretArgs,
        #[command(flatten)]
        db: DbArgs,
    },
    /// Revoke one token (by jti or by the token itself) or all tokens of a user.
    Revoke {
        /// Token id (printed by `token issue`).
        #[arg(long, conflicts_with_all = ["user", "token"])]
        jti: Option<String>,
        /// Revoke all tokens of this admin issued up to now.
        #[arg(long, conflicts_with = "token")]
        user: Option<String>,
        /// The token itself.
        #[arg(long)]
        token: Option<String>,
        #[arg(long)]
        reason: Option<String>,
        #[command(flatten)]
        secret: SecretArgs,
        #[command(flatten)]
        db: DbArgs,
    },
}

#[derive(Subcommand)]
enum RevokeCmd {
    /// Show revoked tokens and users.
    List(DbArgs),
    /// Delete revocation rows that are no longer needed.
    Prune(DbArgs),
}

#[derive(Subcommand)]
enum BookingsCmd {
    /// Expire overdue pending bookings now.
    Expire(DbArgs),
    /// Erase booker names and emails after the retention period now.
    PurgePersonalData {
        #[command(flatten)]
        db: DbArgs,
        #[command(flatten)]
        booking: BookingArgs,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,rmcp=warn")))
        .with_writer(std::io::stderr)
        .init();

    match Cli::parse().command {
        Command::Serve(args) => serve(args).await,
        Command::Secret(SecretCmd::Generate) => {
            println!("{}", auth::generate_secret());
            Ok(())
        }
        Command::Token(cmd) => token(cmd).await,
        Command::Revoke(RevokeCmd::List(d)) => {
            let pool = db::connect(&d.db).await?;
            let (tokens, users) = revocations::list(&pool).await?;
            println!("Revoked tokens: {}", tokens.len());
            for t in tokens {
                println!(
                    "  jti={} user={} revoked_at={} reason={}",
                    t.jti,
                    t.sub,
                    t.revoked_at,
                    t.reason.unwrap_or_default()
                );
            }
            println!("Revoked users: {}", users.len());
            for u in users {
                let before = chrono::DateTime::from_timestamp(u.revoked_before, 0)
                    .map(|t| t.to_rfc3339())
                    .unwrap_or_default();
                println!(
                    "  user={} tokens issued before {} reason={}",
                    u.sub,
                    before,
                    u.reason.unwrap_or_default()
                );
            }
            Ok(())
        }
        Command::Revoke(RevokeCmd::Prune(d)) => {
            let pool = db::connect(&d.db).await?;
            println!("pruned {} rows", revocations::prune(&pool, jwt::MAX_TOKEN_DAYS).await?);
            Ok(())
        }
        Command::Bookings(BookingsCmd::Expire(d)) => {
            let pool = db::connect(&d.db).await?;
            println!("expired {} bookings", bookings::expire_due(&pool).await?);
            Ok(())
        }
        Command::Bookings(BookingsCmd::PurgePersonalData { db: d, booking }) => {
            let pool = db::connect(&d.db).await?;
            let n = bookings::purge_personal_data(&pool, booking.booking_retention_days).await?;
            println!("erased personal data of {n} bookings");
            Ok(())
        }
        Command::Seed(d) => {
            let pool = db::connect(&d.db).await?;
            seed::run(&pool).await
        }
    }
}

async fn serve(args: ServeArgs) -> anyhow::Result<()> {
    let pool = db::connect(&args.db.db).await?;
    let jwt_secret = args.secret.checked()?;
    if jwt_secret.is_none() {
        tracing::warn!("CLASS_ADMIN_JWT_SECRET is not set: admin access is off, /mcp is public only");
    }
    let app = Arc::new(AppState {
        pool,
        booking: args.booking.clone(),
        limiters: Limiters::new(
            args.rate_limit_public,
            args.rate_limit_booking,
            args.rate_limit_admin,
            args.rate_limit_auth_fail,
        ),
        jwt_secret,
    });

    let cancel = CancellationToken::new();
    jobs::spawn(app.clone(), cancel.clone());
    let router = http::router(
        app,
        HttpOptions {
            allowed_hosts: args.allowed_hosts()?,
            allowed_origins: args.origins(),
            trusted_proxy: args.trusted_proxy,
            cancel: cancel.clone(),
        },
    );

    let listener = tokio::net::TcpListener::bind(args.bind)
        .await
        .with_context(|| format!("binding {}", args.bind))?;
    tracing::info!("listening on http://{} (public: /mcp, admin: /mcp/admin)", args.bind);
    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        let _ = tokio::signal::ctrl_c().await;
        tracing::info!("shutting down");
        cancel.cancel();
    })
    .await?;
    Ok(())
}

async fn token(cmd: TokenCmd) -> anyhow::Result<()> {
    match cmd {
        TokenCmd::Issue { user, days, secret } => {
            let (token, claims) = jwt::issue(&secret.required()?, &user, days)?;
            let exp = chrono::DateTime::from_timestamp(claims.exp, 0)
                .map(|t| t.to_rfc3339())
                .unwrap_or_default();
            eprintln!("Admin token for {} (jti {}), expires {exp}.", claims.sub, claims.jti);
            eprintln!(
                "Keep it secret. Revoke it with: class-admin token revoke --jti {}",
                claims.jti
            );
            println!("{token}");
        }
        TokenCmd::Inspect { token, secret, db: d } => {
            let secret = secret.required()?;
            let claims = jwt::inspect(&secret, &token).context("token is invalid (bad signature or claims)")?;
            let pool = db::connect(&d.db).await?;
            let revoked = revocations::is_revoked(&pool, &claims.jti, &claims.sub, claims.iat).await?;
            let state = match jwt::verify(&secret, &token) {
                Ok(_) if revoked => "revoked",
                Ok(_) => "valid",
                Err(jwt::TokenError::Expired) => "expired",
                Err(_) => "invalid",
            };
            let fmt = |t: i64| {
                chrono::DateTime::from_timestamp(t, 0)
                    .map(|t| t.to_rfc3339())
                    .unwrap_or_default()
            };
            println!("status:  {state}");
            println!("user:    {}", claims.sub);
            println!("role:    {}", claims.role);
            println!("jti:     {}", claims.jti);
            println!("issued:  {}", fmt(claims.iat));
            println!("expires: {}", fmt(claims.exp));
        }
        TokenCmd::Revoke {
            jti,
            user,
            token,
            reason,
            secret,
            db: d,
        } => {
            let pool = db::connect(&d.db).await?;
            if let Some(user) = user {
                revocations::revoke_user(&pool, &user, reason.as_deref()).await?;
                println!("revoked all tokens of {user} issued up to now");
            } else if let Some(token) = token {
                let claims = jwt::inspect(&secret.required()?, &token).context("token is invalid")?;
                revocations::revoke_token(&pool, &claims.jti, &claims.sub, claims.exp, reason.as_deref()).await?;
                println!("revoked token {} of {}", claims.jti, claims.sub);
            } else if let Some(jti) = jti {
                // Without the token we don't know its expiry; keep the row for the longest lifetime.
                let exp = chrono::Utc::now().timestamp() + jwt::MAX_TOKEN_DAYS * 86_400;
                revocations::revoke_token(&pool, &jti, "unknown", exp, reason.as_deref()).await?;
                println!("revoked token {jti}");
            } else {
                anyhow::bail!("give --jti, --token or --user");
            }
        }
    }
    Ok(())
}
