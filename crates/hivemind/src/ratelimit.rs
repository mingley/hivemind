//! In-process token-bucket rate limits for the shared service.
//!
//! Classes:
//! - **token** — `/oauth/token` (keyed by client IP + OAuth client_id)
//! - **register** — `/oauth/register` (keyed by client IP)
//! - **rpc** — authenticated HTTP `/v1…`, `/mcp`, and gRPC RPCs (keyed by
//!   client IP + principal)
//!
//! Ops probes (`/health`, `/readyz`, `/metrics`, `/healthz`) are not limited.
//! Set a class's `*_per_minute` to `0` to disable that class.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Default sustained rate for `/oauth/token` (per IP+client_id).
pub const DEFAULT_TOKEN_RATE_PER_MINUTE: u64 = 60;
/// Default sustained rate for `/oauth/register` (per IP).
pub const DEFAULT_REGISTER_RATE_PER_MINUTE: u64 = 10;
/// Default sustained rate for authenticated RPC paths (per IP+principal).
pub const DEFAULT_RPC_RATE_PER_MINUTE: u64 = 600;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RateClass {
    Token,
    Register,
    Rpc,
}

#[derive(Clone, Debug)]
pub struct RateLimitConfig {
    /// Sustained requests per minute for `/oauth/token`. `0` disables.
    pub token_per_minute: u64,
    /// Sustained requests per minute for `/oauth/register`. `0` disables.
    pub register_per_minute: u64,
    /// Sustained requests per minute for authenticated RPC. `0` disables.
    pub rpc_per_minute: u64,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            token_per_minute: DEFAULT_TOKEN_RATE_PER_MINUTE,
            register_per_minute: DEFAULT_REGISTER_RATE_PER_MINUTE,
            rpc_per_minute: DEFAULT_RPC_RATE_PER_MINUTE,
        }
    }
}

#[derive(Debug)]
struct Bucket {
    tokens: f64,
    capacity: f64,
    refill_per_sec: f64,
    last: Instant,
}

impl Bucket {
    fn new(per_minute: u64) -> Self {
        let capacity = per_minute.max(1) as f64;
        Self {
            tokens: capacity,
            capacity,
            refill_per_sec: capacity / 60.0,
            last: Instant::now(),
        }
    }

    fn refill(&mut self, now: Instant) {
        let elapsed = now.saturating_duration_since(self.last).as_secs_f64();
        if elapsed > 0.0 {
            self.tokens = (self.tokens + elapsed * self.refill_per_sec).min(self.capacity);
            self.last = now;
        }
    }

    /// Take one token. On deny, returns suggested retry-after seconds.
    fn take(&mut self) -> Result<(), u64> {
        let now = Instant::now();
        self.refill(now);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            Ok(())
        } else {
            let need = 1.0 - self.tokens;
            let secs = (need / self.refill_per_sec).ceil().max(1.0) as u64;
            Err(secs)
        }
    }

    fn idle_for(&self, now: Instant) -> Duration {
        now.saturating_duration_since(self.last)
    }
}

/// Shared in-process limiter (one process = one map).
#[derive(Debug)]
pub struct RateLimiter {
    config: RateLimitConfig,
    buckets: Mutex<HashMap<String, Bucket>>,
}

#[derive(Clone, Debug)]
pub struct RateLimited {
    pub retry_after_secs: u64,
}

impl RateLimiter {
    pub fn new(config: RateLimitConfig) -> Self {
        Self {
            config,
            buckets: Mutex::new(HashMap::new()),
        }
    }

    pub fn config(&self) -> &RateLimitConfig {
        &self.config
    }

    /// Check and consume one request for `class` under `key`.
    pub fn check(&self, class: RateClass, key: &str) -> Result<(), RateLimited> {
        let per_minute = match class {
            RateClass::Token => self.config.token_per_minute,
            RateClass::Register => self.config.register_per_minute,
            RateClass::Rpc => self.config.rpc_per_minute,
        };
        if per_minute == 0 {
            return Ok(());
        }
        let map_key = match class {
            RateClass::Token => format!("token:{key}"),
            RateClass::Register => format!("register:{key}"),
            RateClass::Rpc => format!("rpc:{key}"),
        };
        let mut guard = self.buckets.lock().unwrap_or_else(|e| e.into_inner());
        Self::maybe_prune(&mut guard);
        let bucket = guard
            .entry(map_key)
            .or_insert_with(|| Bucket::new(per_minute));
        // If the configured rate changed since this bucket was created, rebuild.
        if (bucket.capacity - per_minute as f64).abs() > f64::EPSILON {
            *bucket = Bucket::new(per_minute);
        }
        bucket
            .take()
            .map_err(|retry_after_secs| RateLimited { retry_after_secs })
    }

    fn maybe_prune(map: &mut HashMap<String, Bucket>) {
        if map.len() < 4_096 {
            return;
        }
        let now = Instant::now();
        map.retain(|_, b| b.idle_for(now) < Duration::from_secs(120));
    }
}

/// Build a limiter key from peer IP and an optional client/principal label.
pub fn peer_key(ip: &str, client_or_principal: &str) -> String {
    let client = if client_or_principal.trim().is_empty() {
        "unknown"
    } else {
        client_or_principal.trim()
    };
    format!("{ip}:{client}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_under_limit_then_denies() {
        let limiter = RateLimiter::new(RateLimitConfig {
            token_per_minute: 3,
            register_per_minute: 3,
            rpc_per_minute: 3,
        });
        for _ in 0..3 {
            assert!(limiter.check(RateClass::Token, "127.0.0.1:cli").is_ok());
        }
        let denied = limiter
            .check(RateClass::Token, "127.0.0.1:cli")
            .unwrap_err();
        assert!(denied.retry_after_secs >= 1);
        // Different key is independent.
        assert!(limiter.check(RateClass::Token, "127.0.0.1:other").is_ok());
        // RPC class is independent of token class.
        assert!(limiter.check(RateClass::Rpc, "127.0.0.1:cli").is_ok());
        // Register class is independent.
        assert!(limiter.check(RateClass::Register, "127.0.0.1").is_ok());
    }

    #[test]
    fn zero_disables() {
        let limiter = RateLimiter::new(RateLimitConfig {
            token_per_minute: 0,
            register_per_minute: 0,
            rpc_per_minute: 0,
        });
        for _ in 0..100 {
            assert!(limiter.check(RateClass::Token, "x").is_ok());
            assert!(limiter.check(RateClass::Register, "x").is_ok());
            assert!(limiter.check(RateClass::Rpc, "x").is_ok());
        }
    }
}
