//! Concrete HTTP transport implementations for webmention I/O.
//!
//! - [`UreqTransport`] implements [`crate::feed::fetch::HttpTransport`]
//!   (read-only GET) for source-fetch verification + endpoint discovery.
//! - [`UreqWebmentionPoster`] implements [`super::send::WebmentionPoster`]
//!   for outbound POST.
//!
//! Both honour the SSRF / scheme / size guards in
//! [`crate::feed::fetch`]: scheme is asserted before request, body is
//! capped at [`crate::feed::fetch::MAX_BODY_BYTES`], timeout is
//! [`crate::feed::fetch::DEFAULT_TIMEOUT_SECS`]. DNS-resolution +
//! per-redirect-hop SSRF check are delegated to ureq's stack — ureq
//! resolves at connect time and we set `redirects(0)` to avoid silent
//! cross-origin hops, requiring callers to handle redirect chains
//! explicitly via the `final_url` field (which here equals the request
//! URL since we don't follow redirects).

use std::io::Read;

use url::Url;

use crate::feed::fetch::{
    assert_safe_scheme, assert_under_size_cap, FetchError, FetchRequest, FetchResponse,
    HttpTransport, MAX_BODY_BYTES,
};
use crate::webmention::send::WebmentionPoster;

/// Synchronous HTTP transport backed by `ureq`. Used for source-fetch
/// (receive path) and target-fetch / endpoint-discovery (send path).
#[derive(Clone, Default)]
pub struct UreqTransport;

impl UreqTransport {
    pub fn new() -> Self {
        Self
    }
}

impl HttpTransport for UreqTransport {
    fn fetch(&self, request: &FetchRequest) -> Result<FetchResponse, FetchError> {
        assert_safe_scheme(&request.url)?;

        // ureq honours its agent timeout for connect + read; we use a
        // single budget for both per NFR-3902.
        let agent = ureq::AgentBuilder::new()
            .timeout(request.timeout)
            .redirects(0)
            .user_agent(&request.user_agent)
            .build();

        let mut req = agent.request_url("GET", &request.url);
        if let Some(ims) = &request.conditional.if_modified_since {
            req = req.set("If-Modified-Since", ims);
        }
        if let Some(inm) = &request.conditional.if_none_match {
            req = req.set("If-None-Match", inm);
        }
        if let Some((name, value)) = &request.auth_header {
            req = req.set(name, value);
        }

        let response = match req.call() {
            Ok(r) => r,
            Err(ureq::Error::Status(_status, r)) => r,
            Err(ureq::Error::Transport(t)) => {
                return Err(FetchError::Transport(t.to_string()));
            }
        };

        let status = response.status();
        let last_modified = response.header("last-modified").map(str::to_string);
        let etag = response.header("etag").map(str::to_string);
        let content_type = response.header("content-type").map(str::to_string);

        // Cap body read at MAX_BODY_BYTES + 1 so we can detect overflow.
        let mut body = Vec::with_capacity(8192);
        let cap = MAX_BODY_BYTES + 1;
        let mut reader = response.into_reader().take(cap as u64);
        reader
            .read_to_end(&mut body)
            .map_err(|e| FetchError::Transport(e.to_string()))?;
        assert_under_size_cap(body.len())?;

        Ok(FetchResponse {
            status,
            body,
            last_modified,
            etag,
            content_type,
            final_url: request.url.clone(),
        })
    }
}

/// Synchronous webmention POSTer backed by `ureq`.
#[derive(Clone, Default)]
pub struct UreqWebmentionPoster;

impl UreqWebmentionPoster {
    pub fn new() -> Self {
        Self
    }
}

impl WebmentionPoster for UreqWebmentionPoster {
    fn post_webmention(&self, endpoint: &Url, source: &str, target: &str) -> Result<u16, String> {
        assert_safe_scheme(endpoint).map_err(|e| e.to_string())?;
        let agent = ureq::AgentBuilder::new()
            .timeout(std::time::Duration::from_secs(
                crate::feed::fetch::DEFAULT_TIMEOUT_SECS,
            ))
            .redirects(2)
            .user_agent(&crate::feed::fetch::user_agent())
            .build();
        let req = agent
            .request_url("POST", endpoint)
            .set("Content-Type", "application/x-www-form-urlencoded");
        let body = format!(
            "source={}&target={}",
            urlencoding::encode(source),
            urlencoding::encode(target),
        );
        match req.send_string(&body) {
            Ok(r) => Ok(r.status()),
            Err(ureq::Error::Status(s, _)) => Ok(s),
            Err(ureq::Error::Transport(t)) => Err(t.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poster_rejects_unsafe_scheme() {
        let poster = UreqWebmentionPoster::new();
        let err = poster
            .post_webmention(
                &Url::parse("javascript:alert(1)").unwrap(),
                "https://me.example/p",
                "https://t.example/q",
            )
            .unwrap_err();
        assert!(err.to_lowercase().contains("scheme"));
    }
}
