//! Admin JWTs: HS256, signed with the server secret.

use chrono::Utc;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};

pub const ISSUER: &str = "class-admin";
pub const AUDIENCE: &str = "class-admin-mcp";
pub const ROLE_ADMIN: &str = "admin";
pub const DEFAULT_TOKEN_DAYS: i64 = 30;
/// Longest allowed token lifetime. Also bounds how long user revocations must be kept.
pub const MAX_TOKEN_DAYS: i64 = 90;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Claims {
    pub sub: String,
    pub role: String,
    pub iat: i64,
    pub exp: i64,
    pub jti: String,
    pub iss: String,
    pub aud: String,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TokenError {
    #[error("token is invalid")]
    Invalid,
    #[error("token has expired")]
    Expired,
    #[error("token does not have the admin role")]
    NotAdmin,
}

pub fn issue(secret: &[u8], sub: &str, days: i64) -> anyhow::Result<(String, Claims)> {
    anyhow::ensure!(
        (1..=MAX_TOKEN_DAYS).contains(&days),
        "token lifetime must be 1-{MAX_TOKEN_DAYS} days"
    );
    let sub = sub.trim();
    anyhow::ensure!(
        !sub.is_empty() && sub.len() <= 64 && sub.chars().all(|c| c.is_ascii_alphanumeric() || "._-@".contains(c)),
        "user name must be 1-64 characters of letters, digits and . _ - @"
    );
    let now = Utc::now().timestamp();
    let claims = Claims {
        sub: sub.to_string(),
        role: ROLE_ADMIN.to_string(),
        iat: now,
        exp: now + days * 86_400,
        jti: uuid::Uuid::new_v4().to_string(),
        iss: ISSUER.to_string(),
        aud: AUDIENCE.to_string(),
    };
    let token = encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(secret)?,
    )?;
    Ok((token, claims))
}

fn validation(check_exp: bool) -> Validation {
    // Only HS256 is accepted; "none" and every other algorithm are rejected.
    let mut v = Validation::new(Algorithm::HS256);
    v.leeway = 30;
    v.validate_exp = check_exp;
    v.set_issuer(&[ISSUER]);
    v.set_audience(&[AUDIENCE]);
    v.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
    v
}

/// Checks signature, algorithm, expiry, issuer, audience and role.
/// Revocation is checked separately (it needs the database).
pub fn verify(secret: &[u8], token: &str) -> Result<Claims, TokenError> {
    let key = DecodingKey::from_secret(secret).map_err(|_| TokenError::Invalid)?;
    let data = decode::<Claims>(token, &key, &validation(true)).map_err(|e| match e.kind() {
        jsonwebtoken::errors::ErrorKind::ExpiredSignature => TokenError::Expired,
        _ => TokenError::Invalid,
    })?;
    if data.claims.role != ROLE_ADMIN {
        return Err(TokenError::NotAdmin);
    }
    Ok(data.claims)
}

/// Decodes a token with a valid signature even if it has expired (for `token inspect`).
pub fn inspect(secret: &[u8], token: &str) -> Result<Claims, TokenError> {
    let key = DecodingKey::from_secret(secret).map_err(|_| TokenError::Invalid)?;
    decode::<Claims>(token, &key, &validation(false))
        .map(|d| d.claims)
        .map_err(|_| TokenError::Invalid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64_helpers::*;

    const SECRET: &[u8] = b"0123456789abcdef0123456789abcdef-test";

    mod base64_helpers {
        /// URL-safe base64 without padding, enough for building test tokens.
        pub fn b64(data: &[u8]) -> String {
            const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
            let mut out = String::new();
            for chunk in data.chunks(3) {
                let n = chunk
                    .iter()
                    .enumerate()
                    .fold(0u32, |acc, (i, b)| acc | (*b as u32) << (16 - 8 * i));
                for i in 0..=chunk.len() {
                    out.push(T[((n >> (18 - 6 * i)) & 63) as usize] as char);
                }
            }
            out
        }
    }

    fn claims(role: &str, exp_offset: i64, iss: &str, aud: &str) -> Claims {
        let now = Utc::now().timestamp();
        Claims {
            sub: "alice".into(),
            role: role.into(),
            iat: now,
            exp: now + exp_offset,
            jti: "j1".into(),
            iss: iss.into(),
            aud: aud.into(),
        }
    }

    fn sign(c: &Claims, secret: &[u8]) -> String {
        encode(
            &Header::new(Algorithm::HS256),
            c,
            &EncodingKey::from_secret(secret).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn issue_and_verify() {
        let (token, c) = issue(SECRET, "alice", 30).unwrap();
        assert_eq!(verify(SECRET, &token).unwrap(), c);
        assert!(issue(SECRET, "alice", 0).is_err());
        assert!(issue(SECRET, "alice", MAX_TOKEN_DAYS + 1).is_err());
        assert!(issue(SECRET, "bad name", 1).is_err());
    }

    #[test]
    fn rejects_bad_tokens() {
        let (token, _) = issue(SECRET, "alice", 1).unwrap();
        assert_eq!(
            verify(b"another-secret-another-secret-xx", &token),
            Err(TokenError::Invalid)
        );
        assert_eq!(verify(SECRET, "garbage"), Err(TokenError::Invalid));
        assert_eq!(
            verify(SECRET, &sign(&claims("admin", -3600, ISSUER, AUDIENCE), SECRET)),
            Err(TokenError::Expired)
        );
        assert_eq!(
            verify(SECRET, &sign(&claims("admin", 3600, "evil", AUDIENCE), SECRET)),
            Err(TokenError::Invalid)
        );
        assert_eq!(
            verify(SECRET, &sign(&claims("admin", 3600, ISSUER, "other"), SECRET)),
            Err(TokenError::Invalid)
        );
        assert_eq!(
            verify(SECRET, &sign(&claims("viewer", 3600, ISSUER, AUDIENCE), SECRET)),
            Err(TokenError::NotAdmin)
        );
    }

    #[test]
    fn rejects_alg_none() {
        let header = b64(br#"{"alg":"none","typ":"JWT"}"#);
        let payload = b64(serde_json::to_string(&claims("admin", 3600, ISSUER, AUDIENCE))
            .unwrap()
            .as_bytes());
        let token = format!("{header}.{payload}.");
        assert_eq!(verify(SECRET, &token), Err(TokenError::Invalid));
    }

    #[test]
    fn rejects_other_algorithm() {
        let token = encode(
            &Header::new(Algorithm::HS512),
            &claims("admin", 3600, ISSUER, AUDIENCE),
            &EncodingKey::from_secret(SECRET).unwrap(),
        )
        .unwrap();
        assert_eq!(verify(SECRET, &token), Err(TokenError::Invalid));
    }

    #[test]
    fn inspect_expired() {
        let token = sign(&claims("admin", -3600, ISSUER, AUDIENCE), SECRET);
        assert!(inspect(SECRET, &token).is_ok());
    }
}
