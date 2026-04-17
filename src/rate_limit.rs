use anyhow::{Result, anyhow};
use dashmap::DashMap;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::config::{RateLimit, RateLimitMode};

/// Sliding-window rate limiter with exact and wildcard pattern support.
pub struct RateLimiter {
    windows: Arc<DashMap<String, Vec<Instant>>>,
    exact_limits: Arc<DashMap<String, u32>>,
    wildcard_limits: Arc<Vec<(String, u32)>>,
    mode: RateLimitMode,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self {
            windows: Arc::new(DashMap::new()),
            exact_limits: Arc::new(DashMap::new()),
            wildcard_limits: Arc::new(Vec::new()),
            mode: RateLimitMode::Global,
        }
    }

    pub fn from_config(limits: &HashMap<String, RateLimit>, mode: RateLimitMode) -> Self {
        let exact = DashMap::new();
        let mut wildcards = Vec::new();

        for (pattern, entry) in limits {
            if pattern.contains('*') {
                wildcards.push((pattern.trim_end_matches('*').to_string(), entry.per_minute));
            } else {
                exact.insert(pattern.clone(), entry.per_minute);
            }
        }

        // Prefer the most specific wildcard first.
        wildcards.sort_by_key(|b| std::cmp::Reverse(b.0.len()));

        Self {
            windows: Arc::new(DashMap::new()),
            exact_limits: Arc::new(exact),
            wildcard_limits: Arc::new(wildcards),
            mode,
        }
    }

    fn resolve_limit(&self, tool_name: &str) -> Option<(String, u32)> {
        if let Some(limit) = self.exact_limits.get(tool_name) {
            return Some((tool_name.to_string(), *limit));
        }

        for (prefix, limit) in self.wildcard_limits.iter() {
            if tool_name.starts_with(prefix) {
                return Some((format!("{}*", prefix), *limit));
            }
        }

        None
    }

    /// Check rate limit for a tool call. In `per_caller` mode, the caller_id
    /// is used to scope the rate limit window. In `global` mode, caller_id is ignored.
    pub fn check(&self, tool_name: &str, caller_id: Option<&str>) -> Result<()> {
        let (rate_key, limit) = match self.resolve_limit(tool_name) {
            Some(limit) => limit,
            None => return Ok(()),
        };

        // In per_caller mode, prefix the window key with caller_id
        let window_key = match (&self.mode, caller_id) {
            (RateLimitMode::PerCaller, Some(id)) if !id.is_empty() => {
                format!("{}:{}", id, rate_key)
            }
            _ => rate_key.clone(),
        };

        let now = Instant::now();
        let window_start = now - Duration::from_secs(60);
        let mut entry = self.windows.entry(window_key).or_default();

        entry.retain(|instant| *instant > window_start);

        if entry.len() >= limit as usize {
            let retry_after = entry
                .first()
                .map(|oldest| 60u64.saturating_sub(oldest.elapsed().as_secs()))
                .unwrap_or(60);

            return Err(anyhow!(
                "Rate limit exceeded for {}. Limit: {}/min. Retry after {}s.",
                tool_name,
                limit,
                retry_after
            ));
        }

        entry.push(now);
        Ok(())
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exact_limit() {
        let limiter = RateLimiter::new();
        limiter.exact_limits.insert("test.tool".to_string(), 3);

        assert!(limiter.check("test.tool", None).is_ok());
        assert!(limiter.check("test.tool", None).is_ok());
        assert!(limiter.check("test.tool", None).is_ok());
        assert!(limiter.check("test.tool", None).is_err());
    }

    #[test]
    fn test_no_limit_means_allowed() {
        let limiter = RateLimiter::new();
        for _ in 0..50 {
            assert!(limiter.check("unlisted.tool", None).is_ok());
        }
    }

    #[test]
    fn test_wildcard_pattern() {
        let limiter = RateLimiter {
            windows: Arc::new(DashMap::new()),
            exact_limits: Arc::new(DashMap::new()),
            wildcard_limits: Arc::new(vec![("docker.container.".to_string(), 5)]),
            mode: RateLimitMode::Global,
        };

        assert!(limiter.check("docker.container.list", None).is_ok());
        assert!(limiter.check("docker.container.start", None).is_ok());
        assert!(limiter.check("docker.container.stop", None).is_ok());
        assert!(limiter.check("docker.container.logs", None).is_ok());
        assert!(limiter.check("docker.container.inspect", None).is_ok());
        assert!(limiter.check("docker.container.list", None).is_err());
    }

    #[test]
    fn test_exact_overrides_wildcard() {
        let exact_limits = DashMap::new();
        exact_limits.insert("docker.container.list".to_string(), 2);

        let limiter = RateLimiter {
            windows: Arc::new(DashMap::new()),
            exact_limits: Arc::new(exact_limits),
            wildcard_limits: Arc::new(vec![("docker.container.".to_string(), 5)]),
            mode: RateLimitMode::Global,
        };

        assert!(limiter.check("docker.container.list", None).is_ok());
        assert!(limiter.check("docker.container.list", None).is_ok());
        assert!(limiter.check("docker.container.list", None).is_err());
    }

    #[test]
    fn test_per_caller_independent_windows() {
        let limiter = RateLimiter {
            windows: Arc::new(DashMap::new()),
            exact_limits: Arc::new({
                let map = DashMap::new();
                map.insert("test.tool".to_string(), 2);
                map
            }),
            wildcard_limits: Arc::new(Vec::new()),
            mode: RateLimitMode::PerCaller,
        };

        // User A uses 2 calls — hits limit
        assert!(limiter.check("test.tool", Some("user_a")).is_ok());
        assert!(limiter.check("test.tool", Some("user_a")).is_ok());
        assert!(limiter.check("test.tool", Some("user_a")).is_err());

        // User B still has their own quota
        assert!(limiter.check("test.tool", Some("user_b")).is_ok());
        assert!(limiter.check("test.tool", Some("user_b")).is_ok());
        assert!(limiter.check("test.tool", Some("user_b")).is_err());
    }

    #[test]
    fn test_per_caller_none_falls_back_to_global() {
        let limiter = RateLimiter {
            windows: Arc::new(DashMap::new()),
            exact_limits: Arc::new({
                let map = DashMap::new();
                map.insert("test.tool".to_string(), 2);
                map
            }),
            wildcard_limits: Arc::new(Vec::new()),
            mode: RateLimitMode::PerCaller,
        };

        // Calls without caller_id share a single window
        assert!(limiter.check("test.tool", None).is_ok());
        assert!(limiter.check("test.tool", None).is_ok());
        assert!(limiter.check("test.tool", None).is_err());
    }

    #[test]
    fn test_global_mode_ignores_caller_id() {
        let limiter = RateLimiter {
            windows: Arc::new(DashMap::new()),
            exact_limits: Arc::new({
                let map = DashMap::new();
                map.insert("test.tool".to_string(), 2);
                map
            }),
            wildcard_limits: Arc::new(Vec::new()),
            mode: RateLimitMode::Global,
        };

        // In global mode, different caller_ids still share one window
        assert!(limiter.check("test.tool", Some("user_a")).is_ok());
        assert!(limiter.check("test.tool", Some("user_b")).is_ok());
        assert!(limiter.check("test.tool", Some("user_c")).is_err());
    }
}
