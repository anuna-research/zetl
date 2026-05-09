//! Inbound feed fetcher per task-inbound-fetcher (REQ-3810, REQ-3811,
//! REQ-3812, NFR-3802, NFR-3806, T1..T8).
//!
//! Pure-core defences in this module:
//!
//!   * **SSRF allowlist** — [`is_public_ip`] / [`assert_public_target`]
//!     reject RFC 1918, RFC 6598, link-local, loopback, multicast,
//!     reserved, and CGNAT.
//!   * **Scheme allowlist** — [`assert_safe_scheme`] rejects `file://`,
//!     `data:`, `javascript:`, etc.
//!   * **XXE defence** — the parser entry-points reject DOCTYPE
//!     declarations and external entity references in the input
//!     stream.
//!   * **Decompression bound** — [`MAX_BODY_BYTES`] (1 MiB).
//!   * **Per-request timeout** — [`DEFAULT_TIMEOUT_SECS`] (10s).
//!   * **Per-fetch concurrency cap** — [`MAX_CONCURRENT_FETCHES`] (8).
//!
//! HTTP is abstracted behind [`HttpTransport`]; the shell wires whatever
//! HTTP library is available (`ureq`/`reqwest`) and passes its
//! responses through this module's safety layer.

use std::net::IpAddr;
use std::time::Duration;
use url::Url;

/// Cap on response body size after decompression per REQ-3811.
pub const MAX_BODY_BYTES: usize = 1024 * 1024;
/// Per-request timeout per NFR-3802.
pub const DEFAULT_TIMEOUT_SECS: u64 = 10;
/// Concurrent-fetch cap per NFR-3806.
pub const MAX_CONCURRENT_FETCHES: usize = 8;

/// User-Agent emitted on every outbound fetch per REQ-3811. Bare —
/// no telemetry, no operator email, no version detail beyond the
/// crate version, to minimise privacy leak.
pub fn user_agent() -> String {
    format!("zetl/{}", env!("CARGO_PKG_VERSION"))
}

/// Safety check: target URL must use http or https.
pub fn assert_safe_scheme(url: &Url) -> Result<(), FetchError> {
    match url.scheme() {
        "http" | "https" => Ok(()),
        other => Err(FetchError::UnsafeScheme(other.to_string())),
    }
}

/// Safety check: target host must resolve to a public IP. The shell
/// resolves the host (potentially multiple A/AAAA records) and passes
/// each address; if ANY is non-public the request is rejected.
pub fn assert_public_target(addresses: &[IpAddr]) -> Result<(), FetchError> {
    if addresses.is_empty() {
        return Err(FetchError::DnsEmpty);
    }
    for addr in addresses {
        if !is_public_ip(*addr) {
            return Err(FetchError::PrivateAddress(*addr));
        }
    }
    Ok(())
}

/// True iff the IP belongs to a publicly-routable range.
pub fn is_public_ip(addr: IpAddr) -> bool {
    match addr {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => is_public_v6(v6),
    }
}

fn is_public_v4(addr: std::net::Ipv4Addr) -> bool {
    let octets = addr.octets();
    let [a, b, _, _] = octets;
    if addr.is_loopback() || addr.is_private() || addr.is_link_local() {
        return false;
    }
    if addr.is_multicast() || addr.is_broadcast() || addr.is_unspecified() {
        return false;
    }
    if addr.is_documentation() {
        return false;
    }
    // RFC 6598 carrier-grade NAT.
    if a == 100 && (64..=127).contains(&b) {
        return false;
    }
    // 169.254.0.0/16 link-local.
    if a == 169 && b == 254 {
        return false;
    }
    // Reserved: 240.0.0.0/4.
    if a >= 240 {
        return false;
    }
    true
}

fn is_public_v6(addr: std::net::Ipv6Addr) -> bool {
    if addr.is_loopback() || addr.is_unspecified() || addr.is_multicast() {
        return false;
    }
    // IPv4-mapped (::ffff:a.b.c.d) addresses must be classified by the IPv4
    // rules — otherwise an attacker can wrap a private/loopback IPv4 in a v6
    // literal (e.g. http://[::ffff:127.0.0.1]/) to bypass the SSRF guard.
    if let Some(v4) = addr.to_ipv4_mapped() {
        return is_public_v4(v4);
    }
    // Deprecated IPv4-compatible form (::a.b.c.d, high 96 bits zero, not the
    // ::ffff: prefix). Refuse outright — it has no legitimate modern use and
    // could otherwise smuggle a private v4 past us.
    let segs = addr.segments();
    if segs[..6] == [0, 0, 0, 0, 0, 0] && (segs[6] != 0 || segs[7] != 0) {
        return false;
    }
    // fe80::/10 link-local.
    if (segs[0] & 0xffc0) == 0xfe80 {
        return false;
    }
    // fc00::/7 unique-local.
    if (segs[0] & 0xfe00) == 0xfc00 {
        return false;
    }
    // Documentation 2001:db8::/32.
    if segs[0] == 0x2001 && segs[1] == 0x0db8 {
        return false;
    }
    // 64:ff9b::/96 NAT64 — treat as public.
    true
}

/// Safety check on parsed XML stream: reject DOCTYPE declarations and
/// external-entity references. Pure: walks the byte slice once.
pub fn assert_no_xxe(body: &[u8]) -> Result<(), FetchError> {
    let s = std::str::from_utf8(body).unwrap_or("");
    if s.to_ascii_lowercase().contains("<!doctype") {
        return Err(FetchError::Xxe("DOCTYPE declaration present".to_string()));
    }
    if s.contains("<!ENTITY") {
        return Err(FetchError::Xxe("inline ENTITY declaration".to_string()));
    }
    if s.contains("SYSTEM \"") || s.contains("SYSTEM '") {
        return Err(FetchError::Xxe("SYSTEM external reference".to_string()));
    }
    if s.contains("PUBLIC \"") {
        return Err(FetchError::Xxe("PUBLIC external reference".to_string()));
    }
    Ok(())
}

/// Decompression bomb guard. The transport hands us the inflated body;
/// we refuse anything past [`MAX_BODY_BYTES`].
pub fn assert_under_size_cap(body_len: usize) -> Result<(), FetchError> {
    if body_len > MAX_BODY_BYTES {
        return Err(FetchError::TooLarge {
            size: body_len,
            limit: MAX_BODY_BYTES,
        });
    }
    Ok(())
}

/// Cross-origin redirect detection. Returns `true` iff `from` and `to`
/// have a different (scheme, host, port) tuple — used by
/// [`crate::feed::auth::strip_credentials_on_cross_origin_redirect`].
pub fn is_cross_origin(from: &Url, to: &Url) -> bool {
    let same = from.scheme() == to.scheme()
        && from.host_str() == to.host_str()
        && from.port_or_known_default() == to.port_or_known_default();
    !same
}

/// Header pair the fetcher emits before issuing a request. Returns
/// only If-Modified-Since / If-None-Match when the caller has a
/// previously-stored value (REQ-3811 conditional requests).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConditionalHeaders {
    pub if_modified_since: Option<String>,
    pub if_none_match: Option<String>,
}

/// Per-feed fetch state persisted across runs.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FetchState {
    /// Last `Last-Modified` header value (for IMS).
    pub last_modified: Option<String>,
    /// Last `ETag` value (for INM).
    pub etag: Option<String>,
    /// First-seen identity record set (REQ-3812). Each record is
    /// `(guid, canonical_link, content_fingerprint)`.
    pub seen_identities: Vec<SeenIdentity>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SeenIdentity {
    pub guid: Option<String>,
    pub canonical_link: Option<String>,
    pub content_fingerprint: Option<String>,
}

impl FetchState {
    /// REQ-3812 first-seen dedup: an item is duplicate iff ANY of its
    /// signals match a stored identity record.
    pub fn is_duplicate(&self, candidate: &SeenIdentity) -> bool {
        for stored in &self.seen_identities {
            if signals_match(&stored.guid, &candidate.guid) {
                return true;
            }
            if signals_match(&stored.canonical_link, &candidate.canonical_link) {
                return true;
            }
            if signals_match(&stored.content_fingerprint, &candidate.content_fingerprint) {
                return true;
            }
        }
        false
    }

    /// Append a new identity record. Caller calls only after
    /// confirming `is_duplicate(...) == false`.
    pub fn record(&mut self, identity: SeenIdentity) {
        self.seen_identities.push(identity);
    }
}

fn signals_match(a: &Option<String>, b: &Option<String>) -> bool {
    match (a, b) {
        (Some(x), Some(y)) if !x.is_empty() && !y.is_empty() => x == y,
        _ => false,
    }
}

/// Build conditional headers from stored state.
pub fn conditional_headers(state: &FetchState) -> ConditionalHeaders {
    ConditionalHeaders {
        if_modified_since: state.last_modified.clone(),
        if_none_match: state.etag.clone(),
    }
}

/// Abstract HTTP transport. Implementations go in the shell where
/// `ureq` / `reqwest` / etc. are available.
pub trait HttpTransport {
    /// Issue a GET. Returns the inflated body + the relevant headers.
    /// The transport MUST already enforce the timeout, follow at most
    /// the configured redirect chain, and re-validate every redirect
    /// hop's IP against [`assert_public_target`].
    fn fetch(&self, request: &FetchRequest) -> Result<FetchResponse, FetchError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchRequest {
    pub url: Url,
    pub user_agent: String,
    pub conditional: ConditionalHeaders,
    pub auth_header: Option<(String, String)>,
    pub timeout: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchResponse {
    pub status: u16,
    pub body: Vec<u8>,
    pub last_modified: Option<String>,
    pub etag: Option<String>,
    pub content_type: Option<String>,
    /// All `Link:` response headers, in receipt order. Used by SPEC-039
    /// webmention endpoint discovery (REQ-3908) where the header form
    /// takes precedence over `<link>` / `<a>` in HTML. Each value is the
    /// raw header string (multiple comma-separated entries inside one
    /// header are allowed and the parser handles them).
    pub link_headers: Vec<String>,
    /// The final URL after following any redirect chain; used by the
    /// auth layer to decide whether to drop credentials.
    pub final_url: Url,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FetchError {
    #[error("unsafe URL scheme: {0}")]
    UnsafeScheme(String),
    #[error("DNS resolution returned no addresses")]
    DnsEmpty,
    #[error("non-public address {0}")]
    PrivateAddress(IpAddr),
    #[error("SSRF block: {0}")]
    Ssrf(String),
    #[error("XXE block: {0}")]
    Xxe(String),
    #[error("response body {size} bytes exceeds {limit}-byte cap")]
    TooLarge { size: usize, limit: usize },
    #[error("transport: {0}")]
    Transport(String),
    #[error("timeout after {0:?}")]
    Timeout(Duration),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    #[test]
    fn ssrf_rejects_loopback_and_private() {
        for bad in [
            "127.0.0.1",
            "10.0.0.5",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.0.1",
            "100.64.0.1",
            "240.0.0.1",
        ] {
            let addr: Ipv4Addr = bad.parse().unwrap();
            assert!(!is_public_ip(addr.into()), "expected private: {bad}");
        }
        for good in ["8.8.8.8", "1.1.1.1", "203.0.0.1"] {
            let addr: Ipv4Addr = good.parse().unwrap();
            assert!(is_public_ip(addr.into()), "expected public: {good}");
        }
    }

    #[test]
    fn ssrf_rejects_ipv6_private_and_link_local() {
        for bad in ["::1", "fe80::1", "fc00::1", "fd12::1", "2001:db8::1"] {
            let addr: Ipv6Addr = bad.parse().unwrap();
            assert!(!is_public_ip(addr.into()), "expected private: {bad}");
        }
        let good: Ipv6Addr = "2606:4700:4700::1111".parse().unwrap();
        assert!(is_public_ip(good.into()));
    }

    #[test]
    fn ssrf_rejects_ipv4_mapped_private_v6() {
        // IPv4-mapped IPv6 wraps the v4 address in the low 32 bits of a v6
        // literal (::ffff:127.0.0.1). It must be classified by the v4 rules
        // so private/loopback v4 cannot bypass the guard.
        for bad in [
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            "::ffff:192.168.1.1",
            "::ffff:169.254.1.1",
            "::ffff:100.64.0.1",
        ] {
            let addr: Ipv6Addr = bad.parse().unwrap();
            assert!(!is_public_ip(addr.into()), "expected private: {bad}");
        }
        // Public v4 wrapped as v6-mapped is still public.
        let good: Ipv6Addr = "::ffff:8.8.8.8".parse().unwrap();
        assert!(is_public_ip(good.into()));
        // Deprecated IPv4-compatible form ::a.b.c.d is rejected outright.
        for bad in ["::127.0.0.1", "::8.8.8.8"] {
            let addr: Ipv6Addr = bad.parse().unwrap();
            assert!(!is_public_ip(addr.into()), "expected reject: {bad}");
        }
    }

    #[test]
    fn assert_public_target_rejects_when_any_addr_private() {
        let public: IpAddr = "8.8.8.8".parse().unwrap();
        let private: IpAddr = "10.0.0.1".parse().unwrap();
        assert!(assert_public_target(&[public, private]).is_err());
        assert!(assert_public_target(&[public]).is_ok());
        assert!(assert_public_target(&[]).is_err());
    }

    #[test]
    fn safe_scheme_rejects_file_data_javascript() {
        for s in [
            "file:///tmp/foo",
            "data:text/plain,hi",
            "javascript:alert(1)",
        ] {
            let url = Url::parse(s).unwrap();
            assert!(matches!(
                assert_safe_scheme(&url),
                Err(FetchError::UnsafeScheme(_))
            ));
        }
        for s in ["http://example.com", "https://example.com"] {
            let url = Url::parse(s).unwrap();
            assert_safe_scheme(&url).unwrap();
        }
    }

    #[test]
    fn xxe_rejects_doctype_and_entity_declarations() {
        let bad = b"<?xml version=\"1.0\"?><!DOCTYPE r [ <!ENTITY x SYSTEM \"file:///etc/passwd\"> ]><r>&x;</r>";
        assert!(assert_no_xxe(bad).is_err());
        let bad2 = b"<r><!ENTITY foo \"bar\">&foo;</r>";
        assert!(assert_no_xxe(bad2).is_err());
        let good = b"<r>plain</r>";
        assert!(assert_no_xxe(good).is_ok());
    }

    #[test]
    fn body_cap_enforced() {
        assert!(assert_under_size_cap(MAX_BODY_BYTES).is_ok());
        assert!(matches!(
            assert_under_size_cap(MAX_BODY_BYTES + 1),
            Err(FetchError::TooLarge { .. })
        ));
    }

    #[test]
    fn cross_origin_detection() {
        let a = Url::parse("https://example.com/feed.xml").unwrap();
        let b = Url::parse("https://other.example/feed.xml").unwrap();
        assert!(is_cross_origin(&a, &b));
        let c = Url::parse("https://example.com/different").unwrap();
        assert!(!is_cross_origin(&a, &c));
        let scheme_change = Url::parse("http://example.com/feed.xml").unwrap();
        assert!(is_cross_origin(&a, &scheme_change));
    }

    #[test]
    fn fetch_state_dedup_matches_any_signal() {
        let mut state = FetchState::default();
        state.record(SeenIdentity {
            guid: Some("g1".to_string()),
            canonical_link: Some("https://example.com/a".to_string()),
            content_fingerprint: Some("h1".to_string()),
        });
        // Same GUID -> duplicate.
        assert!(state.is_duplicate(&SeenIdentity {
            guid: Some("g1".to_string()),
            canonical_link: Some("https://example.com/different".to_string()),
            content_fingerprint: Some("different".to_string()),
        }));
        // Same canonical_link -> duplicate.
        assert!(state.is_duplicate(&SeenIdentity {
            guid: Some("g_other".to_string()),
            canonical_link: Some("https://example.com/a".to_string()),
            content_fingerprint: Some("h_other".to_string()),
        }));
        // None match -> not a duplicate.
        assert!(!state.is_duplicate(&SeenIdentity {
            guid: Some("g_new".to_string()),
            canonical_link: Some("https://example.com/new".to_string()),
            content_fingerprint: Some("h_new".to_string()),
        }));
    }

    #[test]
    fn user_agent_minimal() {
        let ua = user_agent();
        assert!(ua.starts_with("zetl/"));
        assert!(!ua.contains("(") && !ua.contains(";"));
    }
}
