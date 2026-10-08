//! Proving you may open a code.
//!
//! The secret arrives once, in the URL fragment, which no browser sends to a server. It
//! is exchanged for a cookie scoped to that one code's path, so every later request —
//! pages, API calls and image loads alike — carries it without the secret ever reaching
//! an access log, a `Referer` or a proxy.
//!
//! Everything a code owns therefore lives under `/c/{code}`, including its slice of the
//! API. A cookie with `Path=/c/ABC` is not sent to `/api/v1/codes/ABC`, and the
//! alternative — one cookie at `Path=/` per code — grows a header that is sent on every
//! request to everything.

use std::collections::HashMap;

use axum::{
    RequestExt as _,
    extract::{FromRequestParts, Path, Request},
    http::{StatusCode, request::Parts},
    middleware::Next,
    response::{IntoResponse, Response},
};
use qr_core::{Binding, Code, CodeId, Secret, store};

use crate::state::AppState;

/// The cookie that holds a code's capability.
pub fn cookie_name(code: &CodeId) -> String {
    format!("qc_{code}")
}

/// Scoped to the one code, so two boxes open in two tabs do not leak into each other
/// and the jar does not grow a header sent everywhere.
pub fn cookie_path(code: &CodeId) -> String {
    format!("/c/{code}")
}

/// Renders the Set-Cookie value for a redeemed code.
pub fn set_cookie(code: &CodeId, secret: &Secret, max_age_days: i64, secure: bool) -> String {
    let mut cookie = format!(
        "{}={}; Path={}; Max-Age={}; HttpOnly; SameSite=Lax",
        cookie_name(code),
        secret.expose(),
        cookie_path(code),
        max_age_days * 24 * 60 * 60,
    );
    if secure {
        cookie.push_str("; Secure");
    }
    cookie
}

/// Clears it again.
pub fn clear_cookie(code: &CodeId) -> String {
    format!(
        "{}=; Path={}; Max-Age=0; HttpOnly; SameSite=Lax",
        cookie_name(code),
        cookie_path(code),
    )
}

/// What the layer proved, and what every guarded route reads.
#[derive(Debug, Clone)]
pub struct CodeAccess {
    pub code: Code,
    /// The binding this code currently has, if it has been claimed.
    pub binding: Option<Binding>,
}

/// The guarded request's code. Infallible once the layer has run.
#[derive(Debug, Clone)]
pub struct CodeCtx(pub CodeAccess);

impl<S: Send + Sync> FromRequestParts<S> for CodeCtx {
    /// A missing extension means the route was mounted outside the guarded subtree,
    /// which is a bug here rather than anything a caller did.
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<CodeAccess>()
            .cloned()
            .map(Self)
            .ok_or((
                StatusCode::INTERNAL_SERVER_ERROR,
                "this route was mounted without the capability layer\n",
            ))
    }
}

/// One generic refusal for every way a request can fail to prove itself.
///
/// Not 401 or 403: either confirms the code exists, which turns the id space into an
/// enumerable inventory of how many boxes there are. The body says the same thing
/// whether the code is unknown, the secret is wrong or the cookie has expired.
fn refuse() -> Response {
    (
        StatusCode::NOT_FOUND,
        [("content-type", "text/plain; charset=utf-8")],
        "this link is not valid. Scan the sticker on the box.\n",
    )
        .into_response()
}

/// Reads the capability cookie and looks the code up.
///
/// Shared by the middleware and by the landing page, which cannot sit behind the
/// middleware because it has to answer before a cookie exists.
pub async fn resolve(
    state: &AppState,
    code_id: &CodeId,
    headers: &axum::http::HeaderMap,
) -> Option<CodeAccess> {
    let secret = header_cookie(headers, &cookie_name(code_id))
        .as_deref()
        .and_then(|v| Secret::parse(v).ok())?;

    let code = match store::redeem(state.store().pool(), code_id, &secret).await {
        Ok(code) => code?,
        Err(error) => {
            tracing::error!(%error, "a capability check could not reach the database");
            return None;
        }
    };

    let binding = match store::active_binding(state.store().pool(), code_id).await {
        Ok(binding) => binding,
        Err(error) => {
            tracing::error!(%error, "a binding lookup failed");
            return None;
        }
    };

    Some(CodeAccess { code, binding })
}

/// Reads one cookie out of a header map.
fn header_cookie(headers: &axum::http::HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(axum::http::header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value.to_owned())
}

/// Checks the capability cookie, and hands the code and its binding to what follows.
///
/// Mounted with `route_layer` rather than `layer`, so a path that matches nothing in
/// the subtree 404s in the router without paying for a database lookup, and a miss is
/// never reported as an authorisation failure.
pub async fn require_capability(
    axum::extract::State(state): axum::extract::State<AppState>,
    mut request: Request,
    next: Next,
) -> Response {
    let Ok(Path(params)) = request
        .extract_parts::<Path<HashMap<String, String>>>()
        .await
    else {
        return refuse();
    };
    let Some(raw) = params.get("code") else {
        return refuse();
    };
    let Ok(code_id) = CodeId::parse(raw) else {
        return refuse();
    };

    let Some(access) = resolve(&state, &code_id, request.headers()).await else {
        return refuse();
    };

    request.extensions_mut().insert(access);
    next.run(request).await
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;
    use qr_core::ids::{DEFAULT_CODE_ID_LEN, DEFAULT_SECRET_LEN};

    #[test]
    fn the_cookie_is_scoped_to_the_one_code() {
        let code = CodeId::generate(DEFAULT_CODE_ID_LEN).expect("id");
        let secret = Secret::generate(DEFAULT_SECRET_LEN).expect("secret");
        let cookie = set_cookie(&code, &secret, 365, true);

        assert!(cookie.starts_with(&format!("qc_{code}=")));
        assert!(cookie.contains(&format!("Path=/c/{code}")));
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("SameSite=Lax"));
        assert!(cookie.contains("Secure"));
        assert!(cookie.contains("Max-Age=31536000"));
    }

    #[test]
    fn dropping_secure_is_something_that_has_to_be_asked_for() {
        let code = CodeId::generate(DEFAULT_CODE_ID_LEN).expect("id");
        let secret = Secret::generate(DEFAULT_SECRET_LEN).expect("secret");
        assert!(!set_cookie(&code, &secret, 1, false).contains("Secure"));
    }

    #[test]
    fn clearing_uses_the_same_name_and_path_so_the_browser_matches_it() {
        let code = CodeId::generate(DEFAULT_CODE_ID_LEN).expect("id");
        let secret = Secret::generate(DEFAULT_SECRET_LEN).expect("secret");
        let set = set_cookie(&code, &secret, 1, true);
        let clear = clear_cookie(&code);
        assert!(clear.starts_with(&format!("qc_{code}=")));
        assert!(set.contains(&format!("Path=/c/{code}")));
        assert!(clear.contains(&format!("Path=/c/{code}")));
        assert!(clear.contains("Max-Age=0"));
    }

    #[test]
    fn two_codes_do_not_share_a_cookie() {
        let a = CodeId::parse("ABCD123456").expect("a");
        let b = CodeId::parse("ZYXW987654").expect("b");
        assert_ne!(cookie_name(&a), cookie_name(&b));
        assert_ne!(cookie_path(&a), cookie_path(&b));
    }
}
