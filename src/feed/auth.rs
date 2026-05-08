//! Inbound authentication per REQ-3824..REQ-3828 + OBS-3808.
//!
//! Composes with [`crate::feed::fetch`]: the fetcher hands the request
//! to [`apply_auth`] before issuing it; on each redirect hop the
//! transport calls [`strip_credentials_on_cross_origin_redirect`].

use crate::feed::credentials::Credential;
use crate::feed::fetch::{is_cross_origin, FetchRequest};
use base64::Engine;
use url::Url;

/// Build the request's auth header (and possibly URL substitution)
/// from a credential record. Returns the modified request.
pub fn apply_auth(mut request: FetchRequest, cred: &Credential) -> FetchRequest {
    match cred {
        Credential::Basic { username, password } => {
            let raw = format!("{username}:{password}");
            let encoded = base64::engine::general_purpose::STANDARD.encode(raw.as_bytes());
            request.auth_header = Some(("Authorization".to_string(), format!("Basic {encoded}")));
        }
        Credential::Bearer { token } => {
            request.auth_header = Some(("Authorization".to_string(), format!("Bearer {token}")));
        }
        Credential::QueryParam {
            url_with_token,
            token_param,
            token_value,
        } => {
            if let Some(full) = url_with_token {
                if let Ok(parsed) = Url::parse(full) {
                    request.url = parsed;
                }
            } else if let (Some(param), Some(value)) = (token_param, token_value) {
                request.url.query_pairs_mut().append_pair(param, value);
            }
        }
    }
    request
}

/// REQ-3826 cross-origin redirect credential drop. Returns a *new*
/// `FetchRequest` with credentials stripped if the redirect is
/// cross-origin; otherwise returns the input unchanged.
pub fn strip_credentials_on_cross_origin_redirect(
    request: FetchRequest,
    redirect_to: &Url,
    cred: &Credential,
) -> FetchRequest {
    if !is_cross_origin(&request.url, redirect_to) {
        // Same-origin: keep the auth_header (Basic/Bearer) attached and
        // re-splice any QueryParam token onto the new URL — `redirect_to`
        // is the server's `Location` and won't carry the token we
        // appended in `apply_auth`.
        let mut same_origin = FetchRequest {
            url: redirect_to.clone(),
            ..request
        };
        if let Credential::QueryParam {
            token_param: Some(param),
            token_value: Some(value),
            ..
        } = cred
        {
            same_origin.url.query_pairs_mut().append_pair(param, value);
        }
        return same_origin;
    }
    let mut stripped = FetchRequest {
        url: redirect_to.clone(),
        ..request
    };
    stripped.auth_header = None;
    // For query-param credentials, also remove the token parameter
    // from the URL so the redirected request carries no auth.
    if let Credential::QueryParam {
        token_param: Some(param),
        ..
    } = cred
    {
        let scrubbed = drop_query_param(&stripped.url, param);
        stripped.url = scrubbed;
    }
    stripped
}

fn drop_query_param(url: &Url, name: &str) -> Url {
    let pairs: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(k, _)| k != name)
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    let mut out = url.clone();
    out.set_query(None);
    if !pairs.is_empty() {
        let mut q = out.query_pairs_mut();
        for (k, v) in &pairs {
            q.append_pair(k, v);
        }
    }
    out
}

/// REQ-3827 logging hygiene: redact secret-shaped query parameters
/// from a URL before it appears in any log line. Replace the value
/// with the literal string `<redacted>` for `token`/`api_key`/`key`/
/// `auth`/`access_token` parameters; pass other parameters through.
pub fn redact_url_for_log(url: &Url) -> String {
    let mut clone = url.clone();
    let pairs: Vec<(String, String)> = clone
        .query_pairs()
        .map(|(k, v)| {
            let key = k.into_owned();
            let value = if SECRET_PARAM_KEYS
                .iter()
                .any(|k| key.eq_ignore_ascii_case(k))
            {
                "<redacted>".to_string()
            } else {
                v.into_owned()
            };
            (key, value)
        })
        .collect();
    clone.set_query(None);
    if !pairs.is_empty() {
        let mut q = clone.query_pairs_mut();
        for (k, v) in &pairs {
            q.append_pair(k, v);
        }
    }
    clone.to_string()
}

const SECRET_PARAM_KEYS: &[&str] = &[
    "token",
    "api_key",
    "apikey",
    "key",
    "auth",
    "access_token",
    "secret",
];

/// REQ-3828 retry-policy state. After a 401, the fetcher tries exactly
/// one re-read of credentials + retry; persistent 401/403 enters
/// `Suspended` so subsequent calls produce a structured error before
/// any network activity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthState {
    Healthy,
    /// Just hit a 401; the fetcher will try one more time after
    /// re-reading the credential file.
    OneRetryAllowed,
    /// Persistent 401/403; manual operator action required.
    Suspended,
}

impl AuthState {
    /// Transition on observing a response status.
    pub fn observe(self, status: u16) -> AuthState {
        match (self, status) {
            (AuthState::Suspended, _) => AuthState::Suspended,
            (_, 200..=399) => AuthState::Healthy,
            (AuthState::Healthy, 401) => AuthState::OneRetryAllowed,
            (AuthState::OneRetryAllowed, 401 | 403) => AuthState::Suspended,
            (state, _) => state,
        }
    }

    /// True iff the fetcher should produce a structured error before
    /// attempting any network activity.
    pub fn is_locked(self) -> bool {
        matches!(self, AuthState::Suspended)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(url: &str) -> FetchRequest {
        FetchRequest {
            url: Url::parse(url).unwrap(),
            user_agent: "zetl/test".to_string(),
            conditional: Default::default(),
            auth_header: None,
            timeout: std::time::Duration::from_secs(10),
        }
    }

    #[test]
    fn basic_auth_emits_authorization_header() {
        let cred = Credential::Basic {
            username: "alice".to_string(),
            password: "hunter2".to_string(),
        };
        let r = apply_auth(req("https://example.com/feed"), &cred);
        let (k, v) = r.auth_header.unwrap();
        assert_eq!(k, "Authorization");
        assert!(v.starts_with("Basic "));
    }

    #[test]
    fn bearer_emits_authorization_bearer() {
        let cred = Credential::Bearer {
            token: "abc".to_string(),
        };
        let r = apply_auth(req("https://example.com/feed"), &cred);
        assert_eq!(r.auth_header.unwrap().1, "Bearer abc");
    }

    #[test]
    fn query_param_url_form_replaces_url() {
        let cred = Credential::QueryParam {
            url_with_token: Some("https://example.com/feed?api_key=secret".to_string()),
            token_param: None,
            token_value: None,
        };
        let r = apply_auth(req("https://example.com/different"), &cred);
        assert!(r.url.query().unwrap().contains("api_key=secret"));
        assert!(r.auth_header.is_none());
    }

    #[test]
    fn query_param_pair_form_appends_to_url() {
        let cred = Credential::QueryParam {
            url_with_token: None,
            token_param: Some("api_key".to_string()),
            token_value: Some("secret".to_string()),
        };
        let r = apply_auth(req("https://example.com/feed"), &cred);
        assert!(r.url.query().unwrap().contains("api_key=secret"));
    }

    #[test]
    fn cross_origin_redirect_drops_basic_header() {
        let cred = Credential::Basic {
            username: "u".to_string(),
            password: "p".to_string(),
        };
        let original = apply_auth(req("https://a.example/feed"), &cred);
        let redirect = Url::parse("https://b.example/feed").unwrap();
        let stripped =
            strip_credentials_on_cross_origin_redirect(original.clone(), &redirect, &cred);
        assert!(stripped.auth_header.is_none());
        assert_eq!(stripped.url, redirect);
    }

    #[test]
    fn same_origin_redirect_keeps_credentials() {
        let cred = Credential::Bearer {
            token: "t".to_string(),
        };
        let original = apply_auth(req("https://example.com/feed"), &cred);
        let redirect = Url::parse("https://example.com/feed/v2").unwrap();
        let stripped =
            strip_credentials_on_cross_origin_redirect(original.clone(), &redirect, &cred);
        assert!(stripped.auth_header.is_some());
    }

    #[test]
    fn same_origin_redirect_re_splices_query_param_token() {
        // Without re-splicing, redirect_to (the server's Location)
        // would lack the token, and the next request would 401.
        let cred = Credential::QueryParam {
            url_with_token: None,
            token_param: Some("api_key".to_string()),
            token_value: Some("secret".to_string()),
        };
        let original = apply_auth(req("https://example.com/feed"), &cred);
        assert!(original.url.query().unwrap().contains("api_key=secret"));
        let redirect = Url::parse("https://example.com/feed/v2").unwrap();
        let resumed = strip_credentials_on_cross_origin_redirect(original, &redirect, &cred);
        assert_eq!(resumed.url.host_str(), Some("example.com"));
        assert!(resumed.url.path().starts_with("/feed/v2"));
        assert!(
            resumed.url.query().unwrap_or("").contains("api_key=secret"),
            "expected token re-spliced, got url {}",
            resumed.url
        );
    }

    #[test]
    fn cross_origin_redirect_drops_query_param_token() {
        let cred = Credential::QueryParam {
            url_with_token: None,
            token_param: Some("api_key".to_string()),
            token_value: Some("secret".to_string()),
        };
        let original = apply_auth(req("https://a.example/feed"), &cred);
        let redirect = Url::parse("https://b.example/feed?api_key=secret").unwrap();
        let stripped =
            strip_credentials_on_cross_origin_redirect(original.clone(), &redirect, &cred);
        assert!(!stripped.url.query().unwrap_or("").contains("api_key"));
    }

    #[test]
    fn redact_url_for_log_replaces_secret_params() {
        let url = Url::parse("https://example.com/feed?api_key=secret&q=hello").unwrap();
        let s = redact_url_for_log(&url);
        assert!(s.contains("api_key=%3Credacted%3E") || s.contains("api_key=<redacted>"));
        assert!(s.contains("q=hello"));
    }

    #[test]
    fn auth_state_lockout_after_double_401() {
        let s0 = AuthState::Healthy;
        let s1 = s0.observe(401);
        assert_eq!(s1, AuthState::OneRetryAllowed);
        let s2 = s1.observe(401);
        assert_eq!(s2, AuthState::Suspended);
        assert!(s2.is_locked());
    }

    #[test]
    fn auth_state_recovers_on_success() {
        let s = AuthState::OneRetryAllowed.observe(200);
        assert_eq!(s, AuthState::Healthy);
    }

    #[test]
    fn auth_state_suspended_stays_suspended() {
        let s = AuthState::Suspended.observe(200);
        assert_eq!(s, AuthState::Suspended);
    }
}
