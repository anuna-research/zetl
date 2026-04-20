//! Capability-URL rendering + parsing (SPEC-034 CON-3401).
//!
//! Three URL shapes share one path layout (`/c/<path-cap>/<slug>.html`)
//! but differ in whether they carry key material in the fragment:
//!
//! - **Delegated-URL (default)** — `#k=<43-char-base64url>` carries a
//!   256-bit `priv_A` share that the shim binds to a passkey on first
//!   click (TOFU).
//! - **Hardened (WebAuthn-PRF-only)** — no fragment; readers self-enrol
//!   at `/enroll.html` to derive a cohort-scoped X25519 identity.
//! - **Split-key (opt-in)** — `#k1=<43-char-base64url>` carries only
//!   half of `priv_A`; the other half is conveyed separately (QR /
//!   spoken phrase — see REQ-3430).
//!
//! The parser is strict: anything that is not one of these three
//! shapes fails with a typed error. In particular, query strings,
//! trailing slashes on the slug, and unknown fragment prefixes are all
//! rejected so operators cannot accidentally mint URLs that look like
//! legitimate invites but silently fail in the shim.
//!
//! This module is pure: no I/O, no randomness, no wall clock. All
//! randomness that *produces* fragment keys lives in the effectful
//! shell (`cap::invite::keygen_deterministic` + OsRng); the pure core
//! only validates and renders.

use std::fmt;

/// Path segment that marks capability-mode URLs. CON-3401 fixes `/c/`
/// as the first segment after the host; changing this is a wire-format
/// break (existing bookmarks die).
pub const CAP_PATH_PREFIX: &str = "/c/";

/// Fragment prefix for delegated-URL mode. CON-3401: `#k=<priv_A>`.
pub const FRAGMENT_PREFIX_DELEGATED: &str = "k=";

/// Fragment prefix for split-key mode (REQ-3430). `#k1=<half1>`; the
/// complementary share is conveyed through a separate channel.
pub const FRAGMENT_PREFIX_SPLIT_KEY: &str = "k1=";

/// `priv_A` (or `half1`) is 256 bits → 32 bytes → 43 base64url chars
/// (unpadded). Pinned here so parsing can reject short/long fragments
/// without reconstructing the decoded byte count.
pub const FRAGMENT_KEY_B64URL_LEN: usize = 43;

/// Minimum path-cap length in Crockford-base32 chars (48 bits).
pub const PATH_CAP_MIN_CHARS: usize = 10;
/// Maximum path-cap length in Crockford-base32 chars (128 bits).
pub const PATH_CAP_MAX_CHARS: usize = 26;

/// The `.html` suffix every cap URL must carry on its slug. CON-3401
/// fixes the extension so hosts serving static files route correctly
/// (no `index.html` magic, no content-negotiation).
pub const SLUG_SUFFIX: &str = ".html";

/// Parsed capability URL. Carries only the capability-relevant bits:
/// the scheme/host are echoed back from the input so renderers can
/// reconstruct the URL without re-parsing, but parsing does not
/// interpret them beyond a "looks like a URL" smoke check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapUrl {
    pub scheme: String,
    pub host: String,
    pub path_cap: String,
    pub slug: String,
    pub mode: CapUrlMode,
}

/// Three distinguishable shapes per CON-3401 / REQ-3430.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapUrlMode {
    /// Delegated-URL mode: fragment carries `priv_A`.
    Delegated { priv_a_b64url: String },
    /// Hardened mode: no fragment.
    Hardened,
    /// Split-key mode: fragment carries `half1`; second factor
    /// conveyed out-of-band (not in the URL).
    SplitKey { half1_b64url: String },
}

/// Errors returned by `CapUrl::parse`.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("URL has no scheme://host authority")]
    MissingAuthority,
    #[error("path does not start with `{CAP_PATH_PREFIX}`")]
    MissingCapPrefix,
    #[error("path has fewer segments than `/c/<path-cap>/<slug>.html`")]
    PathTooShort,
    #[error("path-cap segment is empty")]
    PathCapEmpty,
    #[error(
        "path-cap length {0} chars is outside the [{min}, {max}] range",
        min = PATH_CAP_MIN_CHARS,
        max = PATH_CAP_MAX_CHARS
    )]
    PathCapLength(usize),
    #[error("path-cap contains non-Crockford character {0:?}")]
    PathCapCharset(char),
    #[error("slug does not end in `{SLUG_SUFFIX}`")]
    SlugMissingSuffix,
    #[error("slug is empty")]
    SlugEmpty,
    #[error("slug contains `?`, `#`, or control chars")]
    SlugContainsReserved,
    #[error("query strings are not allowed on capability URLs")]
    QueryNotAllowed,
    #[error("fragment prefix `{0}` is not one of `k=` / `k1=`")]
    FragmentPrefix(String),
    #[error(
        "fragment key is {0} chars, expected exactly {n}",
        n = FRAGMENT_KEY_B64URL_LEN
    )]
    FragmentKeyLength(usize),
    #[error("fragment contains non-base64url character {0:?}")]
    FragmentKeyCharset(char),
}

impl CapUrl {
    /// Parse a cap URL string. Strict: no query strings, exact slug
    /// extension, exact fragment prefix.
    pub fn parse(s: &str) -> Result<Self, ParseError> {
        let (scheme, rest) = s.split_once("://").ok_or(ParseError::MissingAuthority)?;
        if scheme.is_empty() || !scheme.chars().all(is_scheme_char) {
            return Err(ParseError::MissingAuthority);
        }

        // Split the fragment off first so we don't accidentally
        // consume `?` or `#` characters as part of the path.
        let (before_frag, fragment) = match rest.split_once('#') {
            Some((a, b)) => (a, Some(b)),
            None => (rest, None),
        };
        if before_frag.contains('?') {
            return Err(ParseError::QueryNotAllowed);
        }

        let (host, path) = match before_frag.split_once('/') {
            Some((h, p)) => (h, format!("/{p}")),
            None => return Err(ParseError::PathTooShort),
        };
        if host.is_empty() {
            return Err(ParseError::MissingAuthority);
        }

        if !path.starts_with(CAP_PATH_PREFIX) {
            return Err(ParseError::MissingCapPrefix);
        }
        let after_prefix = &path[CAP_PATH_PREFIX.len()..];

        // path-cap is the first `/`-delimited segment; everything
        // after is the slug (which may itself contain `/` — wikis
        // support nested paths like `team/infra/runbook.html`).
        let (path_cap, slug_with_ext) = after_prefix
            .split_once('/')
            .ok_or(ParseError::PathTooShort)?;

        validate_path_cap(path_cap)?;
        let slug = validate_slug(slug_with_ext)?;

        let mode = match fragment {
            None | Some("") => CapUrlMode::Hardened,
            Some(f) => parse_fragment(f)?,
        };

        Ok(Self {
            scheme: scheme.to_string(),
            host: host.to_string(),
            path_cap: path_cap.to_string(),
            slug,
            mode,
        })
    }

    /// Render a delegated-URL cap URL (`#k=<priv_A>`).
    pub fn render_delegated(
        scheme: &str,
        host: &str,
        path_cap: &str,
        slug: &str,
        priv_a_b64url: &str,
    ) -> Result<String, ParseError> {
        validate_scheme(scheme)?;
        validate_host(host)?;
        validate_path_cap(path_cap)?;
        let slug_clean = validate_slug_component(slug)?;
        validate_fragment_key(priv_a_b64url)?;
        Ok(format!(
            "{scheme}://{host}{CAP_PATH_PREFIX}{path_cap}/{slug_clean}{SLUG_SUFFIX}#{FRAGMENT_PREFIX_DELEGATED}{priv_a_b64url}"
        ))
    }

    /// Render a hardened-mode cap URL (no fragment).
    pub fn render_hardened(
        scheme: &str,
        host: &str,
        path_cap: &str,
        slug: &str,
    ) -> Result<String, ParseError> {
        validate_scheme(scheme)?;
        validate_host(host)?;
        validate_path_cap(path_cap)?;
        let slug_clean = validate_slug_component(slug)?;
        Ok(format!(
            "{scheme}://{host}{CAP_PATH_PREFIX}{path_cap}/{slug_clean}{SLUG_SUFFIX}"
        ))
    }

    /// Render a split-key cap URL (`#k1=<half1>`). REQ-3430.
    pub fn render_split_key(
        scheme: &str,
        host: &str,
        path_cap: &str,
        slug: &str,
        half1_b64url: &str,
    ) -> Result<String, ParseError> {
        validate_scheme(scheme)?;
        validate_host(host)?;
        validate_path_cap(path_cap)?;
        let slug_clean = validate_slug_component(slug)?;
        validate_fragment_key(half1_b64url)?;
        Ok(format!(
            "{scheme}://{host}{CAP_PATH_PREFIX}{path_cap}/{slug_clean}{SLUG_SUFFIX}#{FRAGMENT_PREFIX_SPLIT_KEY}{half1_b64url}"
        ))
    }
}

impl fmt::Display for CapUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}://{}{CAP_PATH_PREFIX}{}/{}{SLUG_SUFFIX}",
            self.scheme, self.host, self.path_cap, self.slug
        )?;
        match &self.mode {
            CapUrlMode::Hardened => Ok(()),
            CapUrlMode::Delegated { priv_a_b64url } => {
                write!(f, "#{FRAGMENT_PREFIX_DELEGATED}{priv_a_b64url}")
            }
            CapUrlMode::SplitKey { half1_b64url } => {
                write!(f, "#{FRAGMENT_PREFIX_SPLIT_KEY}{half1_b64url}")
            }
        }
    }
}

fn validate_path_cap(s: &str) -> Result<(), ParseError> {
    let n = s.chars().count();
    if !(PATH_CAP_MIN_CHARS..=PATH_CAP_MAX_CHARS).contains(&n) {
        return Err(ParseError::PathCapLength(n));
    }
    for c in s.chars() {
        if !is_crockford(c) {
            return Err(ParseError::PathCapCharset(c));
        }
    }
    Ok(())
}

fn validate_slug(slug_with_ext: &str) -> Result<String, ParseError> {
    if !slug_with_ext.ends_with(SLUG_SUFFIX) {
        return Err(ParseError::SlugMissingSuffix);
    }
    let slug = &slug_with_ext[..slug_with_ext.len() - SLUG_SUFFIX.len()];
    if slug.is_empty() {
        return Err(ParseError::SlugEmpty);
    }
    if slug.contains(['?', '#']) || slug.chars().any(|c| c.is_control()) {
        return Err(ParseError::SlugContainsReserved);
    }
    Ok(slug.to_string())
}

fn validate_slug_component(slug: &str) -> Result<String, ParseError> {
    if slug.is_empty() {
        return Err(ParseError::SlugEmpty);
    }
    if slug.contains(['?', '#']) || slug.chars().any(|c| c.is_control()) {
        return Err(ParseError::SlugContainsReserved);
    }
    if slug.starts_with('/') || slug.ends_with('/') || slug.contains("//") {
        return Err(ParseError::SlugContainsReserved);
    }
    // Callers pass a bare slug; accept it idempotently if they
    // already appended the `.html` suffix (common in CLI pipelines).
    Ok(slug.strip_suffix(SLUG_SUFFIX).unwrap_or(slug).to_string())
}

fn validate_scheme(s: &str) -> Result<(), ParseError> {
    if s.is_empty() || !s.chars().all(is_scheme_char) {
        Err(ParseError::MissingAuthority)
    } else {
        Ok(())
    }
}

fn validate_host(s: &str) -> Result<(), ParseError> {
    if s.is_empty() || s.contains('/') || s.chars().any(|c| c.is_control() || c == ' ') {
        Err(ParseError::MissingAuthority)
    } else {
        Ok(())
    }
}

fn parse_fragment(f: &str) -> Result<CapUrlMode, ParseError> {
    // Order matters: `k1=` must be checked before `k=` or the shorter
    // prefix would swallow split-key fragments.
    if let Some(rest) = f.strip_prefix(FRAGMENT_PREFIX_SPLIT_KEY) {
        validate_fragment_key(rest)?;
        return Ok(CapUrlMode::SplitKey {
            half1_b64url: rest.to_string(),
        });
    }
    if let Some(rest) = f.strip_prefix(FRAGMENT_PREFIX_DELEGATED) {
        validate_fragment_key(rest)?;
        return Ok(CapUrlMode::Delegated {
            priv_a_b64url: rest.to_string(),
        });
    }
    let prefix = f.split('=').next().unwrap_or("").to_string();
    Err(ParseError::FragmentPrefix(format!("{prefix}=")))
}

fn validate_fragment_key(s: &str) -> Result<(), ParseError> {
    let n = s.chars().count();
    if n != FRAGMENT_KEY_B64URL_LEN {
        return Err(ParseError::FragmentKeyLength(n));
    }
    for c in s.chars() {
        if !is_base64url(c) {
            return Err(ParseError::FragmentKeyCharset(c));
        }
    }
    Ok(())
}

/// Crockford base32 alphabet accepted on input. Uppercase only — the
/// encoder emits uppercase and we don't want case collisions in the
/// wire format.
fn is_crockford(c: char) -> bool {
    matches!(
        c,
        '0'..='9'
            | 'A'..='H'
            | 'J'..='K'
            | 'M'..='N'
            | 'P'..='T'
            | 'V'..='Z'
    )
}

/// RFC 4648 base64url alphabet (no padding): `A-Z`, `a-z`, `0-9`, `-`,
/// `_`. Matches what `base64::engine::general_purpose::URL_SAFE_NO_PAD`
/// emits, which is what CLI invite code will feed in.
fn is_base64url(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_'
}

/// RFC 3986 scheme chars: ALPHA / DIGIT / "+" / "-" / ".". First-char
/// constraint (must be ALPHA) is not enforced here — the host-facing
/// URL libraries will reject malformed schemes downstream.
fn is_scheme_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.'
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_KEY: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

    #[test]
    fn render_and_parse_delegated_roundtrip() {
        let url = CapUrl::render_delegated(
            "https",
            "wiki.example",
            "1234567890ABC",
            "pages/intro",
            VALID_KEY,
        )
        .unwrap();
        assert_eq!(
            url,
            format!("https://wiki.example/c/1234567890ABC/pages/intro.html#k={VALID_KEY}")
        );
        let parsed = CapUrl::parse(&url).unwrap();
        assert_eq!(parsed.scheme, "https");
        assert_eq!(parsed.host, "wiki.example");
        assert_eq!(parsed.path_cap, "1234567890ABC");
        assert_eq!(parsed.slug, "pages/intro");
        assert_eq!(
            parsed.mode,
            CapUrlMode::Delegated {
                priv_a_b64url: VALID_KEY.to_string()
            }
        );
    }

    #[test]
    fn render_and_parse_hardened_roundtrip() {
        let url =
            CapUrl::render_hardened("https", "wiki.example", "1234567890ABC", "welcome").unwrap();
        assert_eq!(url, "https://wiki.example/c/1234567890ABC/welcome.html");
        let parsed = CapUrl::parse(&url).unwrap();
        assert_eq!(parsed.mode, CapUrlMode::Hardened);
        assert_eq!(parsed.slug, "welcome");
    }

    #[test]
    fn render_and_parse_split_key_roundtrip() {
        let url = CapUrl::render_split_key(
            "https",
            "wiki.example",
            "1234567890ABC",
            "onboarding",
            VALID_KEY,
        )
        .unwrap();
        let parsed = CapUrl::parse(&url).unwrap();
        assert_eq!(
            parsed.mode,
            CapUrlMode::SplitKey {
                half1_b64url: VALID_KEY.to_string()
            }
        );
    }

    #[test]
    fn display_matches_render() {
        let u = CapUrl::parse(&format!("https://x/c/1234567890ABC/a.html#k={VALID_KEY}")).unwrap();
        assert_eq!(
            u.to_string(),
            format!("https://x/c/1234567890ABC/a.html#k={VALID_KEY}")
        );
    }

    #[test]
    fn reject_query_string() {
        assert_eq!(
            CapUrl::parse("https://x/c/1234567890ABC/a.html?q=1"),
            Err(ParseError::QueryNotAllowed)
        );
    }

    #[test]
    fn reject_missing_prefix() {
        assert_eq!(
            CapUrl::parse("https://x/notc/1234567890ABC/a.html"),
            Err(ParseError::MissingCapPrefix)
        );
    }

    #[test]
    fn reject_missing_scheme() {
        assert_eq!(
            CapUrl::parse("x/c/1234567890ABC/a.html"),
            Err(ParseError::MissingAuthority)
        );
    }

    #[test]
    fn reject_path_cap_length() {
        // 9 chars — too short
        assert!(matches!(
            CapUrl::parse("https://x/c/123456789/a.html"),
            Err(ParseError::PathCapLength(9))
        ));
        // 27 chars — too long
        let long = "1".repeat(27);
        assert!(matches!(
            CapUrl::parse(&format!("https://x/c/{long}/a.html")),
            Err(ParseError::PathCapLength(27))
        ));
    }

    #[test]
    fn reject_path_cap_bad_char() {
        // lowercase letter is not in Crockford alphabet
        assert!(matches!(
            CapUrl::parse("https://x/c/1234567890aBC/a.html"),
            Err(ParseError::PathCapCharset('a'))
        ));
        // `L` is excluded from Crockford
        assert!(matches!(
            CapUrl::parse("https://x/c/1234567890LBC/a.html"),
            Err(ParseError::PathCapCharset('L'))
        ));
    }

    #[test]
    fn reject_slug_missing_suffix() {
        assert_eq!(
            CapUrl::parse("https://x/c/1234567890ABC/intro"),
            Err(ParseError::SlugMissingSuffix)
        );
    }

    #[test]
    fn reject_empty_slug() {
        assert_eq!(
            CapUrl::parse("https://x/c/1234567890ABC/.html"),
            Err(ParseError::SlugEmpty)
        );
    }

    #[test]
    fn reject_fragment_key_wrong_length() {
        let short = "A".repeat(FRAGMENT_KEY_B64URL_LEN - 1);
        assert!(matches!(
            CapUrl::parse(&format!(
                "https://x/c/1234567890ABC/a.html#k={short}"
            )),
            Err(ParseError::FragmentKeyLength(n)) if n == FRAGMENT_KEY_B64URL_LEN - 1
        ));
    }

    #[test]
    fn reject_fragment_key_bad_char() {
        let bad = format!("{}!", "A".repeat(FRAGMENT_KEY_B64URL_LEN - 1));
        assert!(matches!(
            CapUrl::parse(&format!("https://x/c/1234567890ABC/a.html#k={bad}")),
            Err(ParseError::FragmentKeyCharset('!'))
        ));
    }

    #[test]
    fn reject_unknown_fragment_prefix() {
        assert!(matches!(
            CapUrl::parse("https://x/c/1234567890ABC/a.html#key=abc"),
            Err(ParseError::FragmentPrefix(p)) if p == "key="
        ));
    }

    #[test]
    fn parse_multi_segment_slugs() {
        // Slugs may contain `/` — they are the vault-relative path.
        let parsed = CapUrl::parse("https://x/c/1234567890ABC/some/nested/page.html").unwrap();
        assert_eq!(parsed.slug, "some/nested/page");
    }

    #[test]
    fn split_key_prefix_precedes_delegated_check() {
        // `k1=` must match before `k=` even though they share the `k`.
        let url = format!("https://x/c/1234567890ABC/a.html#k1={VALID_KEY}");
        let parsed = CapUrl::parse(&url).unwrap();
        assert!(matches!(parsed.mode, CapUrlMode::SplitKey { .. }));
    }

    #[test]
    fn empty_fragment_is_hardened() {
        // `#` with nothing after it is tolerated as equivalent to no
        // fragment; some channel-rewriters strip the fragment body but
        // leave the `#`.
        let parsed = CapUrl::parse("https://x/c/1234567890ABC/a.html#").unwrap();
        assert_eq!(parsed.mode, CapUrlMode::Hardened);
    }

    #[test]
    fn render_rejects_invalid_slug() {
        assert_eq!(
            CapUrl::render_delegated("https", "x", "1234567890ABC", "bad?slug", VALID_KEY),
            Err(ParseError::SlugContainsReserved)
        );
    }

    #[test]
    fn render_strips_duplicate_html_suffix() {
        let url = CapUrl::render_hardened("https", "x", "1234567890ABC", "intro.html").unwrap();
        assert_eq!(url, "https://x/c/1234567890ABC/intro.html");
    }
}
