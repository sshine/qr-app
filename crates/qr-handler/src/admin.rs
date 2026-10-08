//! Who is allowed to change things.
//!
//! Three ways in, because they answer different questions. A browser arrives through an
//! auth proxy, which injects a header; the service turns that into a signed cookie of
//! its own so pages outside the proxy's path can still tell. A script has no browser to
//! run an OIDC redirect in, so it carries either a token the realm issued or a static
//! one this deployment was given.

use std::{sync::Arc, time::SystemTime};

use async_trait::async_trait;
use axum::{
    extract::{FromRef, FromRequestParts},
    http::{StatusCode, header, request::Parts},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD as B64};
use subtle::ConstantTimeEq as _;

/// Where an admin identity came from. Logged, so a surprising write can be traced to
/// the door it came through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdminSource {
    /// A header an authenticating proxy injected.
    Proxy,
    /// This service's own session cookie, issued after a proxy visit.
    Cookie,
    /// A bearer token the identity provider issued.
    Oidc,
    /// The static bearer token this deployment was given.
    Token,
}

#[derive(Debug, Clone)]
pub struct AdminIdentity {
    pub subject: String,
    pub source: AdminSource,
}

/// Verifies a bearer token against an identity provider.
///
/// A trait rather than a dependency, so this crate needs neither an HTTP client nor a
/// JWT library; the binary supplies one.
#[async_trait]
pub trait TokenVerifier: Send + Sync + 'static {
    /// Returns the subject the token belongs to.
    async fn verify(&self, token: &str) -> Result<String, String>;
}

/// The name of the session cookie. Path `/`, because the forwarding handler shows its
/// editor at the same URL an anonymous visitor is redirected from, and that URL is
/// under `/c/`.
pub const COOKIE: &str = "qr_admin";

#[derive(Clone)]
pub struct AdminConfig {
    /// The header an authenticating proxy injects, and strips from anything arriving
    /// with it, so a request that went round the proxy carries none.
    pub header: String,
    /// Signs the session cookie. Generated on first start and kept in the database.
    pub key: [u8; 32],
    pub session_secs: u64,
    /// A bearer token for scripted calls. Absent leaves that door shut rather than open.
    pub static_token: Option<String>,
    pub verifier: Option<Arc<dyn TokenVerifier>>,
}

impl std::fmt::Debug for AdminConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdminConfig")
            .field("header", &self.header)
            .field("session_secs", &self.session_secs)
            .field("static_token", &self.static_token.as_ref().map(|_| "<set>"))
            .finish_non_exhaustive()
    }
}

impl AdminConfig {
    /// Mints a session cookie value for `subject`.
    ///
    /// `subject|expiry|mac`, all base64url. Signed rather than random-and-stored
    /// because there is no session table to look it up in, and adding one to hold what
    /// a MAC already proves would be a second thing to expire.
    pub fn issue(&self, subject: &str) -> String {
        let expires = now() + self.session_secs;
        let payload = format!("{}|{}", B64.encode(subject), expires);
        let mac = blake3::keyed_hash(&self.key, payload.as_bytes());
        format!("{payload}|{}", B64.encode(mac.as_bytes()))
    }

    /// Reads a session cookie back, or `None` if it was tampered with or has expired.
    pub fn read(&self, value: &str) -> Option<AdminIdentity> {
        let (payload, mac) = value.rsplit_once('|')?;
        let (subject, expires) = payload.split_once('|')?;

        let expected = blake3::keyed_hash(&self.key, payload.as_bytes());
        let given = B64.decode(mac).ok()?;
        if !bool::from(expected.as_bytes().ct_eq(given.as_slice())) {
            return None;
        }

        if expires.parse::<u64>().ok()? <= now() {
            return None;
        }

        Some(AdminIdentity {
            subject: String::from_utf8(B64.decode(subject).ok()?).ok()?,
            source: AdminSource::Cookie,
        })
    }

    /// Resolves whoever this request claims to be, trying each door in turn.
    pub async fn identify(&self, parts: &Parts) -> Option<AdminIdentity> {
        if let Some(subject) = parts
            .headers
            .get(&self.header)
            .and_then(|v| v.to_str().ok())
            .filter(|v| !v.is_empty())
        {
            return Some(AdminIdentity {
                subject: subject.to_owned(),
                source: AdminSource::Proxy,
            });
        }

        if let Some(identity) = cookie_value(parts, COOKIE).and_then(|v| self.read(&v)) {
            return Some(identity);
        }

        let bearer = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .map(str::trim)
            .filter(|v| !v.is_empty())?;

        // Constant time, so a wrong token cannot be narrowed down a byte at a time.
        if let Some(expected) = &self.static_token
            && bool::from(expected.as_bytes().ct_eq(bearer.as_bytes()))
        {
            return Some(AdminIdentity {
                subject: "static-token".to_owned(),
                source: AdminSource::Token,
            });
        }

        let verifier = self.verifier.as_ref()?;
        match verifier.verify(bearer).await {
            Ok(subject) => Some(AdminIdentity {
                subject,
                source: AdminSource::Oidc,
            }),
            Err(reason) => {
                tracing::debug!(reason, "a bearer token was refused");
                None
            }
        }
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Reads one cookie out of the request without parsing them all into a jar.
pub fn cookie_value(parts: &Parts, name: &str) -> Option<String> {
    parts
        .headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value.to_owned())
}

/// An admin identity if there is one, and no rejection if there is not.
///
/// What a page uses to decide whether to show the buttons. The endpoints behind those
/// buttons take `RequireAdmin` instead, so hiding one is cosmetic and not the control.
#[derive(Debug, Clone)]
pub struct MaybeAdmin(pub Option<AdminIdentity>);

/// An admin identity, or 403.
#[derive(Debug, Clone)]
pub struct RequireAdmin(pub AdminIdentity);

impl<S> FromRequestParts<S> for MaybeAdmin
where
    S: Send + Sync,
    AdminConfig: FromRef<S>,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let config = AdminConfig::from_ref(state);
        Ok(Self(config.identify(parts).await))
    }
}

impl<S> FromRequestParts<S> for RequireAdmin
where
    S: Send + Sync,
    AdminConfig: FromRef<S>,
{
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let config = AdminConfig::from_ref(state);
        match config.identify(parts).await {
            Some(identity) => Ok(Self(identity)),
            None => Err((StatusCode::FORBIDDEN, "not authorised\n")),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;

    fn config() -> AdminConfig {
        AdminConfig {
            header: "X-Forwarded-Email".to_owned(),
            key: [7u8; 32],
            session_secs: 3600,
            static_token: Some("a-static-token".to_owned()),
            verifier: None,
        }
    }

    #[test]
    fn a_session_cookie_round_trips() {
        let config = config();
        let value = config.issue("someone@example.com");
        let identity = config.read(&value).expect("should read back");
        assert_eq!(identity.subject, "someone@example.com");
        assert_eq!(identity.source, AdminSource::Cookie);
    }

    #[test]
    fn a_tampered_cookie_is_refused() {
        let config = config();
        let value = config.issue("someone@example.com");

        // Swapping the subject for another keeps the shape and breaks the signature.
        let (_, rest) = value.split_once('|').expect("shape");
        let forged = format!("{}|{rest}", B64.encode("admin@example.com"));
        assert!(config.read(&forged).is_none());

        // So does flipping a byte of the signature.
        let mut bytes = value.into_bytes();
        let last = bytes.len() - 1;
        bytes[last] ^= 0x01;
        let flipped = String::from_utf8(bytes).expect("utf8");
        assert!(config.read(&flipped).is_none());
    }

    #[test]
    fn a_cookie_signed_with_another_key_is_refused() {
        let value = config().issue("someone@example.com");
        let other = AdminConfig {
            key: [9u8; 32],
            ..config()
        };
        assert!(other.read(&value).is_none());
    }

    #[test]
    fn an_expired_cookie_is_refused() {
        let config = AdminConfig {
            session_secs: 0,
            ..config()
        };
        let value = config.issue("someone@example.com");
        assert!(config.read(&value).is_none());
    }

    #[test]
    fn rubbish_is_refused_rather_than_panicking() {
        let config = config();
        for bad in ["", "|", "a|b", "a|b|c", "....|1|....", "a|notanumber|b"] {
            assert!(config.read(bad).is_none(), "{bad} should not read");
        }
    }

    #[test]
    fn one_cookie_is_read_out_of_a_header_holding_several() {
        let request = axum::http::Request::builder()
            .header(header::COOKIE, "qc_ABC=one; qr_admin=two; other=three")
            .body(())
            .expect("request");
        let (parts, ()) = request.into_parts();
        assert_eq!(cookie_value(&parts, COOKIE).as_deref(), Some("two"));
        assert_eq!(cookie_value(&parts, "qc_ABC").as_deref(), Some("one"));
        assert_eq!(cookie_value(&parts, "absent"), None);
    }
}
