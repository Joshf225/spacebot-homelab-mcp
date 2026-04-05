//! Audit verification tool.
//!
//! Provides `audit_verify_operation` — an MCP tool that checks the server's
//! audit log (or live Docker state) to confirm whether an operation actually
//! ran. This is the primary defense against LLM hallucination of tool results:
//! the client can call this tool after any destructive action to prove the
//! action was (or was not) executed by this server.

use std::sync::Arc;

use anyhow::{Result, anyhow};
use tokio::fs;

use crate::audit::AuditLogger;
use crate::config::Config;
use crate::connection::ConnectionManager;
use crate::tools::wrap_output_envelope;

/// Maximum number of audit log lines to scan (most recent first).
const MAX_SCAN_LINES: usize = 500;

/// Verify an operation by checking the audit log for matching entries.
///
/// Returns a JSON envelope containing:
/// - `verified`: bool — whether a matching audit entry was found
/// - `matches`: array of matching log lines (most recent first, max 10)
/// - `scanned_lines`: number of log lines checked
pub async fn verify_operation(
    config: &Config,
    tool_name: &str,
    contains: Option<&str>,
    last_minutes: Option<u64>,
) -> Result<String> {
    let audit_path = config
        .audit
        .file
        .as_ref()
        .ok_or_else(|| anyhow!(
            "Audit file logging is not configured. \
             Set [audit] file = \"/path/to/audit.log\" in config.toml to enable verification."
        ))?;

    if !audit_path.exists() {
        return Ok(wrap_output_envelope(
            "audit.verify_operation",
            &serde_json::json!({
                "verified": false,
                "reason": "Audit log file does not exist yet (no operations recorded).",
                "matches": [],
                "scanned_lines": 0
            })
            .to_string(),
        ));
    }

    let content = fs::read_to_string(audit_path)
        .await
        .map_err(|e| anyhow!("Failed to read audit log: {}", e))?;

    let minutes = last_minutes.unwrap_or(10);
    let cutoff = chrono::Utc::now() - chrono::Duration::minutes(minutes as i64);
    let cutoff_str = cutoff.format("%Y-%m-%dT%H:%M:%S").to_string();

    // Scan lines in reverse (most recent first), limited to MAX_SCAN_LINES
    let lines: Vec<&str> = content.lines().rev().take(MAX_SCAN_LINES).collect();
    let scanned = lines.len();

    let mut matches: Vec<String> = Vec::new();
    for line in &lines {
        // Each line starts with a timestamp: "2025-01-15T10:30:00Z tool=..."
        // Filter by tool name
        if !line.contains(&format!("tool={}", tool_name)) {
            continue;
        }

        // Filter by time window — compare timestamp prefix lexicographically
        // (ISO-8601 timestamps sort correctly as strings)
        if let Some(ts_end) = line.find(' ') {
            let line_ts = &line[..ts_end.min(19)]; // "2025-01-15T10:30:00"
            if line_ts < cutoff_str.as_str() {
                // Past the time window; since we're scanning newest-first, stop
                break;
            }
        }

        // Filter by content substring if provided
        if let Some(needle) = contains {
            if !line.contains(needle) {
                continue;
            }
        }

        matches.push(line.to_string());
        if matches.len() >= 10 {
            break;
        }
    }

    let verified = !matches.is_empty();

    Ok(wrap_output_envelope(
        "audit.verify_operation",
        &serde_json::json!({
            "verified": verified,
            "tool_name": tool_name,
            "time_window_minutes": minutes,
            "contains_filter": contains,
            "matches": matches,
            "scanned_lines": scanned,
            "audit_file": audit_path.to_string_lossy(),
        })
        .to_string(),
    ))
}

/// Check live Docker state to verify a container operation.
///
/// For example, after a "delete container X" operation, this checks whether
/// container X still exists. After "stop container X", checks if it's stopped.
pub async fn verify_container_state(
    manager: Arc<ConnectionManager>,
    host: &str,
    container: &str,
    expected_state: &str,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    use bollard::container::InspectContainerOptions;

    let docker = manager.get_docker(host)?;

    let inspect_result = docker
        .as_bollard()
        .inspect_container(container, None::<InspectContainerOptions>)
        .await;

    let (verified, actual_state, detail) = match (expected_state, &inspect_result) {
        // Expected: container should not exist (deleted)
        ("absent" | "deleted", Err(_)) => (
            true,
            "absent".to_string(),
            format!("Container '{}' does not exist on host '{}' (confirmed deleted).", container, host),
        ),
        ("absent" | "deleted", Ok(details)) => {
            let state = details
                .state
                .as_ref()
                .and_then(|s| s.status.as_ref())
                .map(|s| format!("{:?}", s))
                .unwrap_or_else(|| "unknown".to_string());
            (
                false,
                state.clone(),
                format!(
                    "Container '{}' STILL EXISTS on host '{}' with state '{}'. Expected: absent.",
                    container, host, state
                ),
            )
        }
        // Expected: container should exist in a specific state
        (expected, Ok(details)) => {
            let state = details
                .state
                .as_ref()
                .and_then(|s| s.status.as_ref())
                .map(|s| format!("{:?}", s).to_lowercase())
                .unwrap_or_else(|| "unknown".to_string());
            let matches = state.contains(&expected.to_lowercase());
            let detail = if matches {
                format!("Container '{}' on host '{}' is in state '{}' (matches expected '{}').", container, host, state, expected)
            } else {
                format!("Container '{}' on host '{}' is in state '{}' (expected '{}').", container, host, state, expected)
            };
            (matches, state, detail)
        }
        (expected, Err(e)) => (
            false,
            "error".to_string(),
            format!(
                "Cannot verify container '{}' on host '{}': {}. Expected state: '{}'.",
                container, host, e, expected
            ),
        ),
    };

    audit
        .log(
            "audit.verify_container_state",
            host,
            if verified { "verified" } else { "mismatch" },
            Some(&detail),
        )
        .await
        .ok();

    Ok(wrap_output_envelope(
        "audit.verify_container_state",
        &serde_json::json!({
            "verified": verified,
            "container": container,
            "host": host,
            "expected_state": expected_state,
            "actual_state": actual_state,
            "detail": detail,
        })
        .to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_verify_operation_no_audit_file_configured() {
        let config = Config {
            docker: Default::default(),
            ssh: Default::default(),
            audit: crate::config::AuditConfig {
                file: None,
                syslog: None,
            },
            rate_limits: Default::default(),
            tools: Default::default(),
            confirm: None,
            metrics: Default::default(),
        };

        let result = verify_operation(&config, "docker.container.delete", None, None).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not configured"));
    }

    #[tokio::test]
    async fn test_verify_operation_missing_audit_file() {
        let config = Config {
            docker: Default::default(),
            ssh: Default::default(),
            audit: crate::config::AuditConfig {
                file: Some("/tmp/nonexistent-audit-test-12345.log".into()),
                syslog: None,
            },
            rate_limits: Default::default(),
            tools: Default::default(),
            confirm: None,
            metrics: Default::default(),
        };

        let result = verify_operation(&config, "docker.container.delete", None, None)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        // The content is a nested JSON string
        let content: serde_json::Value =
            serde_json::from_str(parsed["content"].as_str().unwrap()).unwrap();
        assert_eq!(content["verified"], false);
        assert_eq!(content["scanned_lines"], 0);
    }

    #[tokio::test]
    async fn test_verify_operation_finds_match() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let log_content = format!(
            "{} tool=docker.container.delete host=local result=success details=test-nginx\n\
             {} tool=docker.container.list host=local result=success\n",
            now, now
        );
        std::fs::write(tmp.path(), &log_content).unwrap();

        let config = Config {
            docker: Default::default(),
            ssh: Default::default(),
            audit: crate::config::AuditConfig {
                file: Some(tmp.path().to_path_buf()),
                syslog: None,
            },
            rate_limits: Default::default(),
            tools: Default::default(),
            confirm: None,
            metrics: Default::default(),
        };

        let result = verify_operation(
            &config,
            "docker.container.delete",
            Some("test-nginx"),
            Some(5),
        )
        .await
        .unwrap();

        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        let content: serde_json::Value =
            serde_json::from_str(parsed["content"].as_str().unwrap()).unwrap();
        assert_eq!(content["verified"], true);
        assert_eq!(content["matches"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn test_verify_operation_no_match() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let log_content = format!(
            "{} tool=docker.container.list host=local result=success\n",
            now
        );
        std::fs::write(tmp.path(), &log_content).unwrap();

        let config = Config {
            docker: Default::default(),
            ssh: Default::default(),
            audit: crate::config::AuditConfig {
                file: Some(tmp.path().to_path_buf()),
                syslog: None,
            },
            rate_limits: Default::default(),
            tools: Default::default(),
            confirm: None,
            metrics: Default::default(),
        };

        let result = verify_operation(
            &config,
            "docker.container.delete",
            Some("test-nginx"),
            Some(5),
        )
        .await
        .unwrap();

        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        let content: serde_json::Value =
            serde_json::from_str(parsed["content"].as_str().unwrap()).unwrap();
        assert_eq!(content["verified"], false);
        assert!(content["matches"].as_array().unwrap().is_empty());
    }
}
