use thiserror::Error;

/// Errors that tools return to callers.
///
/// Everything except `Internal` carries a message that is safe to show to
/// anyone. `Internal` is logged on the server and shown only as a generic text,
/// so SQL, file paths and other internals never leak.
#[derive(Debug, Error)]
pub enum AppError {
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Conflict(String),
    #[error("permission denied: this action requires an admin")]
    Forbidden,
    #[error("too many requests, please try again later")]
    RateLimited,
    #[error("internal error")]
    Internal(#[source] anyhow::Error),
}

impl AppError {
    pub fn not_found(what: &str) -> Self {
        Self::NotFound(format!("{what} not found"))
    }
    pub fn invalid(msg: impl Into<String>) -> Self {
        Self::Invalid(msg.into())
    }
    pub fn conflict(msg: impl Into<String>) -> Self {
        Self::Conflict(msg.into())
    }

    /// Message that is safe to send to the caller.
    pub fn public_message(&self) -> String {
        if let Self::Internal(err) = self {
            tracing::error!(error = ?err, "internal error");
        }
        self.to_string()
    }
}

impl From<sqlx::Error> for AppError {
    fn from(err: sqlx::Error) -> Self {
        Self::Internal(err.into())
    }
}

impl From<anyhow::Error> for AppError {
    fn from(err: anyhow::Error) -> Self {
        Self::Internal(err)
    }
}

pub type AppResult<T> = Result<T, AppError>;
