//! In-memory rate limiter for authentication endpoints (REQ-020, CON-020).
//!
//! Provides per-key (user ID or IP address) sliding-window counters with
//! configurable limits and window durations. Returns 429 Too Many Requests
//! when the limit is exceeded.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Configuration for a rate limiter instance.
#[derive(Clone, Debug)]
pub struct RateLimitConfig {
    /// Maximum number of requests allowed within the window.
    pub max_requests: u32,
    /// Duration of the sliding window.
    pub window: Duration,
}

/// A single counter tracking requests from one key.
struct Counter {
    /// Timestamps of requests within the current window.
    timestamps: Vec<Instant>,
}

impl Counter {
    fn new() -> Self {
        Self {
            timestamps: Vec::new(),
        }
    }

    /// Prune timestamps older than the window and return the current count.
    fn prune_and_count(&mut self, now: Instant, window: Duration) -> u32 {
        self.timestamps.retain(|&t| now.duration_since(t) < window);
        self.timestamps.len() as u32
    }

    /// Record a new request timestamp.
    fn record(&mut self, now: Instant) {
        self.timestamps.push(now);
    }
}

/// Maximum number of tracked keys before forced eviction.
const MAX_TRACKED_KEYS: usize = 10_000;

/// Thread-safe in-memory rate limiter.
pub struct RateLimiter {
    config: RateLimitConfig,
    counters: Mutex<HashMap<String, Counter>>,
}

impl RateLimiter {
    pub fn new(config: RateLimitConfig) -> Self {
        Self {
            config,
            counters: Mutex::new(HashMap::new()),
        }
    }

    /// Check whether a request from `key` is allowed.
    ///
    /// Returns `Ok(())` if the request is within limits.
    /// Returns `Err(retry_after_secs)` if the rate limit is exceeded.
    pub fn check(&self, key: &str) -> Result<(), u64> {
        let now = Instant::now();
        let mut counters = self.counters.lock().expect("rate limiter lock poisoned");

        // Evict expired/oldest entries if the map has grown too large.
        if counters.len() >= MAX_TRACKED_KEYS && !counters.contains_key(key) {
            // First pass: remove entries with no active timestamps.
            let window = self.config.window;
            counters.retain(|_, counter| counter.prune_and_count(now, window) > 0);

            // Still over limit — drop oldest entries by earliest remaining timestamp.
            if counters.len() >= MAX_TRACKED_KEYS {
                let mut by_oldest: Vec<(String, Instant)> = counters
                    .iter()
                    .map(|(k, c)| {
                        let oldest = c.timestamps.first().copied().unwrap_or(now);
                        (k.clone(), oldest)
                    })
                    .collect();
                by_oldest.sort_by_key(|(_, t)| *t);
                let to_remove = counters.len() - MAX_TRACKED_KEYS + 1;
                for (k, _) in by_oldest.into_iter().take(to_remove) {
                    counters.remove(&k);
                }
            }
        }

        let counter = counters.entry(key.to_string()).or_insert_with(Counter::new);

        let count = counter.prune_and_count(now, self.config.window);
        if count >= self.config.max_requests {
            // Calculate retry-after from the oldest timestamp in the window
            let oldest = counter.timestamps.first().copied().unwrap_or(now);
            let retry_after = self
                .config
                .window
                .checked_sub(now.duration_since(oldest))
                .unwrap_or(Duration::from_secs(1));
            return Err(retry_after.as_secs().max(1));
        }

        counter.record(now);
        Ok(())
    }

    /// Check without recording (peek). Returns true if a request would be allowed.
    pub fn would_allow(&self, key: &str) -> bool {
        let now = Instant::now();
        let mut counters = self.counters.lock().expect("rate limiter lock poisoned");
        let counter = counters.entry(key.to_string()).or_insert_with(Counter::new);
        let count = counter.prune_and_count(now, self.config.window);
        count < self.config.max_requests
    }

    /// Remove all expired entries (housekeeping).
    pub fn purge_expired(&self) {
        let now = Instant::now();
        let mut counters = self.counters.lock().expect("rate limiter lock poisoned");
        counters.retain(|_, counter| {
            counter.prune_and_count(now, self.config.window) > 0
        });
    }
}

/// Build a 429 Too Many Requests response with Retry-After header.
pub fn too_many_requests(retry_after_secs: u64) -> axum::response::Response {
    use axum::http::{HeaderValue, StatusCode};
    use axum::response::IntoResponse;

    let mut resp = (StatusCode::TOO_MANY_REQUESTS, "rate limit exceeded").into_response();
    if let Ok(val) = HeaderValue::from_str(&retry_after_secs.to_string()) {
        resp.headers_mut()
            .insert(axum::http::header::RETRY_AFTER, val);
    }
    resp
}

/// Middleware that enforces per-IP rate limiting on auth endpoints.
///
/// Extracts the client IP from `X-Forwarded-For` header (first entry) or falls
/// back to the peer address. Returns 429 Too Many Requests when the limit is exceeded.
pub async fn auth_ip_rate_limit(
    axum::extract::State(state): axum::extract::State<super::WebState>,
    req: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let ip = extract_client_ip(&req, state.trust_proxy);
    if let Err(retry_after) = state.rate_limiters.auth_per_ip.check(&ip) {
        return too_many_requests(retry_after);
    }
    next.run(req).await
}

/// Extract the client IP from the request.
///
/// When `trust_proxy` is true, checks `X-Forwarded-For` first (takes the
/// leftmost/client IP), then `X-Real-IP`. When `trust_proxy` is false,
/// proxy headers are ignored and only the peer address is used. Falls back
/// to "unknown" if no address can be determined.
fn extract_client_ip(req: &axum::http::Request<axum::body::Body>, trust_proxy: bool) -> String {
    if trust_proxy {
        // X-Forwarded-For: client, proxy1, proxy2
        if let Some(xff) = req
            .headers()
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
        {
            if let Some(first) = xff.split(',').next() {
                let ip = first.trim();
                if !ip.is_empty() {
                    return ip.to_string();
                }
            }
        }

        // X-Real-IP
        if let Some(real_ip) = req
            .headers()
            .get("x-real-ip")
            .and_then(|v| v.to_str().ok())
        {
            let ip = real_ip.trim();
            if !ip.is_empty() {
                return ip.to_string();
            }
        }
    }

    // Peer address from ConnectInfo (if available via axum extension)
    if let Some(connect_info) = req.extensions().get::<axum::extract::ConnectInfo<std::net::SocketAddr>>() {
        return connect_info.0.ip().to_string();
    }

    "unknown".to_string()
}

/// Shared rate limiters for authentication endpoints.
#[derive(Clone)]
pub struct AuthRateLimiters {
    /// Per-user limiter for recovery challenge/verify requests.
    pub recovery_per_user: std::sync::Arc<RateLimiter>,
    /// Per-user limiter for passkey registration attempts.
    pub passkey_per_user: std::sync::Arc<RateLimiter>,
    /// Per-user limiter for invitation acceptance attempts.
    pub invite_per_user: std::sync::Arc<RateLimiter>,
    /// Per-IP limiter applied to all auth endpoints.
    pub auth_per_ip: std::sync::Arc<RateLimiter>,
}

impl AuthRateLimiters {
    /// Create rate limiters with default production settings.
    pub fn new() -> Self {
        use std::sync::Arc;

        Self {
            // Recovery: 5 attempts per 15-minute window per user
            recovery_per_user: Arc::new(RateLimiter::new(RateLimitConfig {
                max_requests: 5,
                window: Duration::from_secs(15 * 60),
            })),
            // Passkey registration: 10 attempts per 15-minute window per user
            passkey_per_user: Arc::new(RateLimiter::new(RateLimitConfig {
                max_requests: 10,
                window: Duration::from_secs(15 * 60),
            })),
            // Invitation acceptance: 5 attempts per 15-minute window per token/IP
            invite_per_user: Arc::new(RateLimiter::new(RateLimitConfig {
                max_requests: 5,
                window: Duration::from_secs(15 * 60),
            })),
            // Per-IP: 30 auth requests per 15-minute window
            auth_per_ip: Arc::new(RateLimiter::new(RateLimitConfig {
                max_requests: 30,
                window: Duration::from_secs(15 * 60),
            })),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_requests_under_limit() {
        let limiter = RateLimiter::new(RateLimitConfig {
            max_requests: 5,
            window: Duration::from_secs(60),
        });
        for _ in 0..5 {
            assert!(limiter.check("alice").is_ok());
        }
    }

    #[test]
    fn blocks_after_limit_exceeded() {
        let limiter = RateLimiter::new(RateLimitConfig {
            max_requests: 5,
            window: Duration::from_secs(60),
        });
        for _ in 0..5 {
            limiter.check("alice").unwrap();
        }
        // 6th request should be blocked
        let err = limiter.check("alice");
        assert!(err.is_err());
        let retry_after = err.unwrap_err();
        assert!(retry_after >= 1);
    }

    #[test]
    fn different_keys_independent() {
        let limiter = RateLimiter::new(RateLimitConfig {
            max_requests: 2,
            window: Duration::from_secs(60),
        });
        limiter.check("alice").unwrap();
        limiter.check("alice").unwrap();
        assert!(limiter.check("alice").is_err());
        // Bob should still be allowed
        assert!(limiter.check("bob").is_ok());
    }

    #[test]
    fn would_allow_does_not_record() {
        let limiter = RateLimiter::new(RateLimitConfig {
            max_requests: 1,
            window: Duration::from_secs(60),
        });
        assert!(limiter.would_allow("alice"));
        assert!(limiter.would_allow("alice")); // still true — not recorded
        limiter.check("alice").unwrap(); // actually record
        assert!(!limiter.would_allow("alice")); // now full
    }

    #[test]
    fn purge_removes_empty_entries() {
        let limiter = RateLimiter::new(RateLimitConfig {
            max_requests: 5,
            window: Duration::from_secs(0), // instant expiry
        });
        limiter.check("alice").unwrap();
        // With 0-second window, entries expire immediately
        limiter.purge_expired();
        let counters = limiter.counters.lock().unwrap();
        assert!(counters.is_empty());
    }

    #[test]
    fn too_many_requests_response_has_429_and_retry_after() {
        use axum::http::StatusCode;
        let resp = too_many_requests(42);
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
        let retry = resp.headers().get("retry-after").unwrap().to_str().unwrap();
        assert_eq!(retry, "42");
    }
}
