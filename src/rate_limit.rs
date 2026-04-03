use anyhow::{anyhow, Result};
use dashmap::DashMap;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::config::RateLimit;

/// Sliding-window rate limiter with exact and wildcard pattern support.
pub struct RateLimiter {
    windows: Arc<DashMap<String, Vec<Instant>>>,
    exact_limits: Arc<DashMap<String, u32>>,
    wildcard_limits: Arc<Vec<(String, u32)>>,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self {
            windows: Arc::new(DashMap::new()),
            exact_limits: Arc::new(DashMap::new()),
            wildcard_limits: Arc::new(Vec::new()),
        }
    }

    pub fn from_config(limits: &HashMap<String, RateLimit>) -> Self {
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
        wildcards.sort_by(|a, b| b.0.len().cmp(&a.0.len()));

        Self {
            windows: Arc::new(DashMap::new()),
            exact_limits: Arc::new(exact),
            wildcard_limits: Arc::new(wildcards),
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

    pub fn check(&self, tool_name: &str) -> Result<()> {
        let (rate_key, limit) = match self.resolve_limit(tool_name) {
            Some(limit) => limit,
            None => return Ok(()),
        };

        let now = Instant::now();
        let window_start = now - Duration::from_secs(60);
        let mut entry = self.windows.entry(rate_key).or_default();

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

        assert!(limiter.check("test.tool").is_ok());
        assert!(limiter.check("test.tool").is_ok());
        assert!(limiter.check("test.tool").is_ok());
        assert!(limiter.check("test.tool").is_err());
    }

    #[test]
    fn test_no_limit_means_allowed() {
        let limiter = RateLimiter::new();
        for _ in 0..50 {
            assert!(limiter.check("unlisted.tool").is_ok());
        }
    }

    #[test]
    fn test_wildcard_pattern() {
        let limiter = RateLimiter {
            windows: Arc::new(DashMap::new()),
            exact_limits: Arc::new(DashMap::new()),
            wildcard_limits: Arc::new(vec![("docker.container.".to_string(), 5)]),
        };

        assert!(limiter.check("docker.container.list").is_ok());
        assert!(limiter.check("docker.container.start").is_ok());
        assert!(limiter.check("docker.container.stop").is_ok());
        assert!(limiter.check("docker.container.logs").is_ok());
        assert!(limiter.check("docker.container.inspect").is_ok());
        assert!(limiter.check("docker.container.list").is_err());
    }

    #[test]
    fn test_exact_overrides_wildcard() {
        let exact_limits = DashMap::new();
        exact_limits.insert("docker.container.list".to_string(), 2);

        let limiter = RateLimiter {
            windows: Arc::new(DashMap::new()),
            exact_limits: Arc::new(exact_limits),
            wildcard_limits: Arc::new(vec![("docker.container.".to_string(), 5)]),
        };

        assert!(limiter.check("docker.container.list").is_ok());
        assert!(limiter.check("docker.container.list").is_ok());
        assert!(limiter.check("docker.container.list").is_err());
    }
}
