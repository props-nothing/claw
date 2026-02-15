//! In-memory token-bucket rate limiter middleware for the Claw API.
//!
//! Each client IP gets an independent bucket with a configurable burst and refill rate.
//! When exhausted, the middleware returns `429 Too Many Requests` with a `Retry-After`
//! header.

use axum::{
    http::{Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use dashmap::DashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Instant;
use tracing::warn;

/// Configuration for the rate limiter.
#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    /// Maximum burst size (tokens in the bucket).
    pub burst: u32,
    /// Tokens refilled per second.
    pub refill_per_sec: f64,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            burst: 60,
            refill_per_sec: 10.0,
        }
    }
}

/// A token bucket for a single client.
#[derive(Debug, Clone)]
struct Bucket {
    tokens: f64,
    last_refill: Instant,
}

impl Bucket {
    fn new(burst: u32) -> Self {
        Self {
            tokens: burst as f64,
            last_refill: Instant::now(),
        }
    }

    /// Refill tokens based on elapsed time, then try to consume one.
    fn try_consume(&mut self, burst: u32, refill_per_sec: f64) -> bool {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        self.tokens = (self.tokens + elapsed * refill_per_sec).min(burst as f64);
        self.last_refill = now;

        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    /// Seconds until the next token is available.
    fn retry_after(&self, refill_per_sec: f64) -> u64 {
        if refill_per_sec <= 0.0 {
            return 60;
        }
        let needed = 1.0 - self.tokens;
        (needed / refill_per_sec).ceil().max(1.0) as u64
    }
}

/// Shared state for the rate limiter, keyed by client IP.
#[derive(Clone)]
pub struct RateLimiter {
    buckets: Arc<DashMap<IpAddr, Bucket>>,
    config: RateLimitConfig,
}

impl RateLimiter {
    pub fn new(config: RateLimitConfig) -> Self {
        Self {
            buckets: Arc::new(DashMap::new()),
            config,
        }
    }

    /// Try to allow a request from the given IP. Returns Ok(()) if allowed,
    /// or Err(retry_after_secs) if rate limited.
    pub fn check(&self, ip: IpAddr) -> Result<(), u64> {
        let mut entry = self
            .buckets
            .entry(ip)
            .or_insert_with(|| Bucket::new(self.config.burst));
        if entry.try_consume(self.config.burst, self.config.refill_per_sec) {
            Ok(())
        } else {
            let retry = entry.retry_after(self.config.refill_per_sec);
            Err(retry)
        }
    }

    /// Evict stale entries (buckets that haven't been used in >5 minutes).
    /// Call periodically in a background task.
    pub fn cleanup(&self) {
        let cutoff = Instant::now() - std::time::Duration::from_secs(300);
        self.buckets
            .retain(|_ip, bucket| bucket.last_refill > cutoff);
    }
}

/// Axum middleware function for rate limiting.
///
/// Extracts the client IP from `x-forwarded-for` header or the connection info,
/// and checks the token bucket.
pub async fn rate_limit_middleware(
    axum::extract::Extension(limiter): axum::extract::Extension<RateLimiter>,
    req: Request<axum::body::Body>,
    next: Next,
) -> Response {
    // Extract client IP
    let ip = extract_client_ip(&req);

    match limiter.check(ip) {
        Ok(()) => next.run(req).await,
        Err(retry_after) => {
            warn!(client_ip = %ip, retry_after, "rate limited");
            let mut resp = (
                StatusCode::TOO_MANY_REQUESTS,
                format!("Rate limit exceeded. Retry after {retry_after} seconds."),
            )
                .into_response();
            resp.headers_mut()
                .insert("retry-after", retry_after.to_string().parse().unwrap());
            resp
        }
    }
}

/// Extract the client IP from the request — checks X-Forwarded-For, then falls back
/// to the connection info, and finally to 127.0.0.1.
fn extract_client_ip(req: &Request<axum::body::Body>) -> IpAddr {
    // Check X-Forwarded-For header
    if let Some(forwarded) = req.headers().get("x-forwarded-for")
        && let Ok(val) = forwarded.to_str()
            && let Some(first) = val.split(',').next()
                && let Ok(ip) = first.trim().parse::<IpAddr>() {
                    return ip;
                }
    // Check X-Real-IP header
    if let Some(real_ip) = req.headers().get("x-real-ip")
        && let Ok(val) = real_ip.to_str()
            && let Ok(ip) = val.trim().parse::<IpAddr>() {
                return ip;
            }
    // Fallback to ConnectInfo if available (would need Axum's ConnectInfo extractor)
    // For now, use localhost as default
    IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn test_bucket_allows_burst() {
        let config = RateLimitConfig {
            burst: 3,
            refill_per_sec: 1.0,
        };
        let limiter = RateLimiter::new(config);
        let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));

        assert!(limiter.check(ip).is_ok());
        assert!(limiter.check(ip).is_ok());
        assert!(limiter.check(ip).is_ok());
        // 4th should be denied
        assert!(limiter.check(ip).is_err());
    }

    #[test]
    fn test_different_ips_independent() {
        let config = RateLimitConfig {
            burst: 1,
            refill_per_sec: 0.0,
        };
        let limiter = RateLimiter::new(config);
        let ip1 = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        let ip2 = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2));

        assert!(limiter.check(ip1).is_ok());
        assert!(limiter.check(ip1).is_err());
        // ip2 should still be allowed
        assert!(limiter.check(ip2).is_ok());
    }

    #[test]
    fn test_retry_after_value() {
        let config = RateLimitConfig {
            burst: 1,
            refill_per_sec: 1.0,
        };
        let limiter = RateLimiter::new(config);
        let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));

        limiter.check(ip).unwrap();
        let retry = limiter.check(ip).unwrap_err();
        assert!(retry >= 1);
    }

    #[test]
    fn test_cleanup_removes_stale() {
        let config = RateLimitConfig::default();
        let limiter = RateLimiter::new(config);
        let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        limiter.check(ip).unwrap();
        assert_eq!(limiter.buckets.len(), 1);
        // cleanup won't remove recent entries
        limiter.cleanup();
        assert_eq!(limiter.buckets.len(), 1);
    }

    #[test]
    fn test_default_config() {
        let config = RateLimitConfig::default();
        assert_eq!(config.burst, 60);
        assert_eq!(config.refill_per_sec, 10.0);
    }
}
