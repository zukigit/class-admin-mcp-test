//! HTTP layer: routes, per-request auth, rate limits.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::{ConnectInfo, Request, State};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use http::{HeaderValue, StatusCode, header};
use rmcp::transport::streamable_http_server::session::never::NeverSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use tokio_util::sync::CancellationToken;
use tower_http::timeout::TimeoutLayer;

use crate::auth::{AdminIdentity, Caller, jwt};
use crate::config::{MAX_BODY_BYTES, REQUEST_TIMEOUT};
use crate::db::revocations;
use crate::mcp::{AppState, McpServerFactory};
use crate::rate_limit;

#[derive(Clone)]
struct HttpState {
    app: Arc<AppState>,
    trusted_proxy: IpAddr,
}

pub struct HttpOptions {
    pub allowed_hosts: Vec<String>,
    pub allowed_origins: Vec<String>,
    pub trusted_proxy: IpAddr,
    pub cancel: CancellationToken,
}

pub fn router(app: Arc<AppState>, opts: HttpOptions) -> Router {
    let factory = McpServerFactory::new(app.clone());
    let mut config = StreamableHttpServerConfig::default()
        .with_allowed_hosts(opts.allowed_hosts)
        .with_legacy_session_mode(false) // stateless: auth is checked on every request
        .with_json_response(true)
        .with_max_request_body_bytes(MAX_BODY_BYTES)
        .with_cancellation_token(opts.cancel);
    config = if opts.allowed_origins.is_empty() {
        // No browser clients configured: reject every request that carries an Origin header.
        config.enforce_origin_validation()
    } else {
        config.with_allowed_origins(opts.allowed_origins)
    };
    let mcp = StreamableHttpService::new(
        move || Ok(factory.create()),
        Arc::new(NeverSessionManager::default()),
        config,
    );

    let state = HttpState {
        app,
        trusted_proxy: opts.trusted_proxy,
    };
    let public = Router::new()
        .route_service("/mcp", mcp.clone())
        .layer(middleware::from_fn_with_state(state.clone(), public_auth));
    let admin = Router::new()
        .route_service("/mcp/admin", mcp)
        .layer(middleware::from_fn_with_state(state, admin_auth));

    Router::new()
        .merge(public)
        .merge(admin)
        .route("/healthz", get(|| async { "ok" }))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            REQUEST_TIMEOUT,
        ))
}

/// Client IP: `X-Forwarded-For` is used only when the request comes from the trusted proxy.
fn client_ip(req: &Request, trusted_proxy: IpAddr) -> String {
    let peer = req.extensions().get::<ConnectInfo<SocketAddr>>().map(|c| c.0.ip());
    if peer == Some(trusted_proxy)
        && let Some(xff) = req.headers().get("x-forwarded-for").and_then(|v| v.to_str().ok())
        // The proxy appends the address it saw, so the last entry is the trustworthy one.
        && let Some(ip) = xff.rsplit(',').next().map(str::trim).and_then(|s| s.parse::<IpAddr>().ok())
    {
        return ip.to_string();
    }
    peer.map_or_else(|| "unknown".to_string(), |ip| ip.to_string())
}

fn bearer(req: &Request) -> Option<Result<String, ()>> {
    let value = req.headers().get(header::AUTHORIZATION)?;
    let parsed = value
        .to_str()
        .ok()
        .and_then(|v| v.strip_prefix("Bearer ").or_else(|| v.strip_prefix("bearer ")))
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty());
    Some(parsed.ok_or(()))
}

fn status(code: StatusCode, msg: &'static str) -> Response {
    (code, msg).into_response()
}

fn unauthorized(error: Option<&str>) -> Response {
    let challenge = match error {
        Some(e) => format!(r#"Bearer realm="class-admin", error="invalid_token", error_description="{e}""#),
        None => r#"Bearer realm="class-admin""#.to_string(),
    };
    let mut res = (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    if let Ok(v) = HeaderValue::from_str(&challenge) {
        res.headers_mut().insert(header::WWW_AUTHENTICATE, v);
    }
    res
}

/// Verifies an admin token: signature, claims and revocation.
// The error is the HTTP response to send; it is used right away, so its size does not matter.
#[allow(clippy::result_large_err)]
async fn check_token(app: &AppState, token: &str) -> Result<AdminIdentity, Response> {
    let Some(secret) = &app.jwt_secret else {
        return Err(status(
            StatusCode::SERVICE_UNAVAILABLE,
            "admin access is not configured on this server",
        ));
    };
    let claims = match jwt::verify(secret, token) {
        Ok(c) => c,
        Err(e) => return Err(unauthorized(Some(&e.to_string()))),
    };
    match revocations::is_revoked(&app.pool, &claims.jti, &claims.sub, claims.iat).await {
        Ok(false) => Ok(AdminIdentity {
            sub: claims.sub,
            jti: claims.jti,
        }),
        Ok(true) => Err(unauthorized(Some("token has been revoked"))),
        Err(e) => {
            tracing::error!(error = ?e, "revocation check failed");
            Err(status(StatusCode::INTERNAL_SERVER_ERROR, "internal error"))
        }
    }
}

async fn authenticate(state: &HttpState, mut req: Request, next: Next, require_token: bool) -> Response {
    let ip = client_ip(&req, state.trusted_proxy);
    let limiters = &state.app.limiters;

    let caller = match bearer(&req) {
        None if require_token => return unauthorized(None),
        None => {
            if !rate_limit::allow(&limiters.public, &ip) {
                return status(StatusCode::TOO_MANY_REQUESTS, "too many requests");
            }
            Caller::public(ip)
        }
        Some(token) => {
            // A token that is present but bad is always refused, never downgraded to public.
            let result = match token {
                Ok(t) => check_token(&state.app, &t).await,
                Err(()) => Err(unauthorized(Some("malformed Authorization header"))),
            };
            match result {
                Ok(admin) => {
                    if !rate_limit::allow(&limiters.admin, &admin.jti) {
                        return status(StatusCode::TOO_MANY_REQUESTS, "too many requests");
                    }
                    tracing::debug!(sub = %admin.sub, "admin request");
                    Caller { admin: Some(admin), ip }
                }
                Err(res) => {
                    if res.status() == StatusCode::UNAUTHORIZED {
                        tracing::warn!(%ip, "rejected admin token");
                        if !rate_limit::allow(&limiters.auth_fail, &ip) {
                            return status(StatusCode::TOO_MANY_REQUESTS, "too many failed logins");
                        }
                    }
                    return res;
                }
            }
        }
    };
    req.extensions_mut().insert(caller);
    next.run(req).await
}

async fn public_auth(State(state): State<HttpState>, req: Request<Body>, next: Next) -> Response {
    authenticate(&state, req, next, false).await
}

async fn admin_auth(State(state): State<HttpState>, req: Request<Body>, next: Next) -> Response {
    authenticate(&state, req, next, true).await
}
