//! Audit verification tool.
//!
//! Provides `audit_verify_operation` — an MCP tool that checks the server's
//! audit log (or live Docker state) to confirm whether an operation actually
//! ran. This is the primary defense against LLM hallucination of tool results:
//! the client can call this tool after any destructive action to prove the
//! action was (or was not) executed by this server.

use std::{collections::VecDeque, path::Path, sync::Arc};

use anyhow::{Result, anyhow};
use tokio::fs;
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::audit::AuditLogger;
use crate::config::Config;
use crate::connection::ConnectionManager;
use crate::tools::wrap_output_envelope;

/// Maximum number of recent audit log lines to retain and scan.
const MAX_SCAN_LINES: usize = 500;

async fn read_recent_audit_lines(audit_path: &Path) -> Result<Vec<String>> {
    let file = fs::File::open(audit_path)
        .await
        .map_err(|e| anyhow!("Failed to read audit log: {}", e))?;
    let mut reader = BufReader::new(file).lines();
    let mut lines = VecDeque::with_capacity(MAX_SCAN_LINES);

    while let Some(line) = reader
        .next_line()
        .await
        .map_err(|e| anyhow!("Failed to read audit log: {}", e))?
    {
        if lines.len() == MAX_SCAN_LINES {
            lines.pop_front();
        }
        lines.push_back(line);
    }

    Ok(lines.into_iter().rev().collect())
}

fn container_state_matches(expected: &str, actual: &str) -> bool {
    let expected = expected.trim().to_lowercase();
    let actual = actual.trim().to_lowercase();

    actual == expected
        || (expected == "stopped" && actual == "exited")
        || (expected == "exited" && actual == "stopped")
}

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
    let audit_path = config.audit.file.as_ref().ok_or_else(|| {
        anyhow!(
            "Audit file logging is not configured. \
             Set [audit] file = \"/path/to/audit.log\" in config.toml to enable verification."
        )
    })?;

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

    let minutes = last_minutes.unwrap_or(10);
    let cutoff = chrono::Utc::now() - chrono::Duration::minutes(minutes as i64);

    // Scan the most recent retained lines first.
    let lines = read_recent_audit_lines(audit_path).await?;
    let scanned = lines.len();

    let expected_tool_token = format!("tool={}", tool_name);

    let mut matches: Vec<String> = Vec::new();
    for line in &lines {
        // Audit log format (see audit.rs):
        //   {timestamp} tool={name} host={host} result={result} [details={d}]
        // Parse structured prefix fields instead of substring matching.
        let mut tokens = line.split_whitespace();

        // First token: timestamp (e.g. "2025-01-15T10:30:00Z").
        // Skip lines with missing or unparseable timestamps.
        let ts_str = match tokens.next() {
            Some(t) => t,
            None => continue,
        };
        let line_ts = match chrono::NaiveDateTime::parse_from_str(ts_str, "%Y-%m-%dT%H:%M:%SZ") {
            Ok(dt) => dt,
            Err(_) => continue,
        };

        // Filter by time window using parsed timestamps.
        if line_ts < cutoff.naive_utc() {
            continue;
        }

        // Second token: must be exactly "tool={tool_name}" (not a substring).
        match tokens.next() {
            Some(tok) if tok == expected_tool_token => {}
            _ => continue,
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
        // Expected: container should not exist (deleted).
        // ONLY a Docker API 404 confirms absence. Other errors (auth,
        // network, socket, etc.) must NOT be treated as "confirmed deleted".
        (
            "absent" | "deleted",
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404, ..
            }),
        ) => (
            true,
            "absent".to_string(),
            format!(
                "Container '{}' does not exist on host '{}' (confirmed deleted).",
                container, host
            ),
        ),
        ("absent" | "deleted", Err(e)) => (
            false,
            "error".to_string(),
            format!(
                "Cannot confirm container '{}' is absent on host '{}': {}. \
                 Only a Docker 404 response confirms deletion; this error may \
                 indicate an auth, network, or socket problem.",
                container, host, e
            ),
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
            let matches = container_state_matches(expected, &state);
            let detail = if matches {
                format!(
                    "Container '{}' on host '{}' is in state '{}' (matches expected '{}').",
                    container, host, state, expected
                )
            } else {
                format!(
                    "Container '{}' on host '{}' is in state '{}' (expected '{}').",
                    container, host, state, expected
                )
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
            proxmox: Default::default(),
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
            proxmox: Default::default(),
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
            proxmox: Default::default(),
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
            proxmox: Default::default(),
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

    #[tokio::test]
    async fn test_verify_operation_keeps_scanning_past_old_out_of_order_lines() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let fresh = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let stale = (chrono::Utc::now() - chrono::Duration::minutes(30))
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string();
        let log_content = format!(
            "{fresh} tool=docker.container.delete host=local result=success details=test-nginx\n\
             {stale} tool=docker.container.delete host=local result=success details=old-nginx\n\
             {fresh} tool=docker.container.delete host=local result=success details=other-nginx\n"
        );
        std::fs::write(tmp.path(), &log_content).unwrap();

        let config = Config {
            docker: Default::default(),
            ssh: Default::default(),
            proxmox: Default::default(),
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

    #[test]
    fn test_container_state_matches_uses_exact_states_with_stopped_alias() {
        assert!(container_state_matches("running", "running"));
        assert!(container_state_matches("stopped", "exited"));
        assert!(container_state_matches("exited", "stopped"));
        assert!(!container_state_matches("run", "running"));
    }

    /// Verify that only a Docker 404 error is treated as "confirmed absent",
    /// while other errors (IO, auth, 500, etc.) are NOT.
    ///
    /// This exercises the same match pattern used in `verify_container_state`
    /// without requiring a live Docker connection.
    #[test]
    fn test_absent_check_only_confirms_on_404_not_other_errors() {
        use bollard::errors::Error as BollardError;

        /// Mimics the match logic in `verify_container_state` for
        /// `("absent" | "deleted", Err(_))` arms, returning `(verified, actual_state)`.
        fn classify_absent_error(err: &BollardError) -> (bool, &'static str) {
            match err {
                BollardError::DockerResponseServerError {
                    status_code: 404, ..
                } => (true, "absent"),
                _ => (false, "error"),
            }
        }

        // 404 → confirmed absent
        let err_404 = BollardError::DockerResponseServerError {
            status_code: 404,
            message: "No such container: test-nginx".to_string(),
        };
        let (verified, state) = classify_absent_error(&err_404);
        assert!(verified, "Docker 404 should confirm container is absent");
        assert_eq!(state, "absent");

        // 500 → NOT confirmed absent
        let err_500 = BollardError::DockerResponseServerError {
            status_code: 500,
            message: "Internal Server Error".to_string(),
        };
        let (verified, state) = classify_absent_error(&err_500);
        assert!(!verified, "Docker 500 must NOT confirm container is absent");
        assert_eq!(state, "error");

        // 403 → NOT confirmed absent
        let err_403 = BollardError::DockerResponseServerError {
            status_code: 403,
            message: "Forbidden".to_string(),
        };
        let (verified, state) = classify_absent_error(&err_403);
        assert!(!verified, "Docker 403 must NOT confirm container is absent");
        assert_eq!(state, "error");

        // IO error → NOT confirmed absent
        let err_io = BollardError::IOError {
            err: std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "connection refused"),
        };
        let (verified, state) = classify_absent_error(&err_io);
        assert!(!verified, "IO error must NOT confirm container is absent");
        assert_eq!(state, "error");
    }

    #[tokio::test]
    async fn test_verify_operation_skips_malformed_lines() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let log_content = format!(
            // Valid line
            "{now} tool=docker.container.delete host=local result=success details=real\n\
             garbage-no-timestamp tool=docker.container.delete host=local result=success details=fake\n\
             tool=docker.container.delete host=local result=success details=no-ts\n\
             \n"
        );
        std::fs::write(tmp.path(), &log_content).unwrap();

        let config = Config {
            docker: Default::default(),
            ssh: Default::default(),
            proxmox: Default::default(),
            audit: crate::config::AuditConfig {
                file: Some(tmp.path().to_path_buf()),
                syslog: None,
            },
            rate_limits: Default::default(),
            tools: Default::default(),
            confirm: None,
            metrics: Default::default(),
        };

        let result = verify_operation(&config, "docker.container.delete", None, Some(5))
            .await
            .unwrap();

        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        let content: serde_json::Value =
            serde_json::from_str(parsed["content"].as_str().unwrap()).unwrap();
        assert_eq!(content["verified"], true);
        // Only the valid line should match; garbage/malformed lines are skipped
        assert_eq!(content["matches"].as_array().unwrap().len(), 1);
        let first_match = content["matches"][0].as_str().unwrap();
        assert!(first_match.contains("details=real"));
    }

    #[tokio::test]
    async fn test_verify_operation_exact_tool_token_match() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let log_content = format!(
            // tool=docker.container.delete should NOT match a search for
            // "docker.container.del" (substring prefix) or vice versa
            "{now} tool=docker.container.delete host=local result=success details=full-name\n\
             {now} tool=docker.container.del host=local result=success details=prefix\n\
             {now} tool=docker.container.delete_all host=local result=success details=suffix\n"
        );
        std::fs::write(tmp.path(), &log_content).unwrap();

        let config = Config {
            docker: Default::default(),
            ssh: Default::default(),
            proxmox: Default::default(),
            audit: crate::config::AuditConfig {
                file: Some(tmp.path().to_path_buf()),
                syslog: None,
            },
            rate_limits: Default::default(),
            tools: Default::default(),
            confirm: None,
            metrics: Default::default(),
        };

        // Search for exact tool name "docker.container.delete"
        let result = verify_operation(&config, "docker.container.delete", None, Some(5))
            .await
            .unwrap();

        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        let content: serde_json::Value =
            serde_json::from_str(parsed["content"].as_str().unwrap()).unwrap();
        assert_eq!(content["verified"], true);
        // Only the exact match should be returned, not the prefix or suffix
        let arr = content["matches"].as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert!(arr[0].as_str().unwrap().contains("details=full-name"));
    }
}
