pub mod jwt;

use crate::db::audit::Actor;
use crate::error::{AppError, AppResult};

/// Who is calling, worked out per HTTP request by the auth middleware.
#[derive(Debug, Clone)]
pub struct Caller {
    pub admin: Option<AdminIdentity>,
    pub ip: String,
}

#[derive(Debug, Clone)]
pub struct AdminIdentity {
    pub sub: String,
    pub jti: String,
}

impl Caller {
    pub fn public(ip: String) -> Self {
        Self { admin: None, ip }
    }

    pub fn is_admin(&self) -> bool {
        self.admin.is_some()
    }

    pub fn actor(&self) -> Actor {
        match &self.admin {
            Some(a) => Actor {
                name: a.sub.clone(),
                token_id: Some(a.jti.clone()),
                ip: Some(self.ip.clone()),
            },
            None => Actor {
                name: "public".into(),
                token_id: None,
                ip: Some(self.ip.clone()),
            },
        }
    }
}

/// The real guard for admin tools: every admin tool calls this first.
pub fn require_admin(caller: &Caller) -> AppResult<Actor> {
    if caller.is_admin() {
        Ok(caller.actor())
    } else {
        Err(AppError::Forbidden)
    }
}

/// Creates a random signing secret (64 hex characters = 32 random bytes).
pub fn generate_secret() -> String {
    use rand::Rng;
    let bytes: [u8; 32] = rand::rng().random();
    hex::encode(bytes)
}
