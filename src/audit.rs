use crate::config::Config;
use anyhow::Result;
use chrono::Utc;
use std::path::Path;
use std::sync::Arc;
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;
use tracing::debug;

/// Audit logger for tool invocations
pub struct AuditLogger {
    config: Arc<Config>,
}

impl AuditLogger {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }

    /// Log a tool invocation
    pub async fn log(
        &self,
        tool_name: &str,
        host: &str,
        result: &str,
        details: Option<&str>,
    ) -> Result<()> {
        let timestamp = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();

        let entry = if let Some(d) = details {
            format!(
                "{} tool={} host={} result={} details={}\n",
                timestamp, tool_name, host, result, d
            )
        } else {
            format!(
                "{} tool={} host={} result={}\n",
                timestamp, tool_name, host, result
            )
        };

        // Write to file if configured
        if let Some(file_path) = &self.config.audit.file {
            self.write_to_file(file_path, &entry).await?;
        }

        // Write to syslog if configured (uses the system `logger` command)
        if let Some(syslog_config) = &self.config.audit.syslog {
            self.write_to_syslog(syslog_config, &entry).await;
        }

        debug!("Audit: {}", entry.trim());
        Ok(())
    }

    /// Write audit entry to file (append-only)
    async fn write_to_file(&self, path: &Path, entry: &str) -> Result<()> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .await?;

        file.write_all(entry.as_bytes()).await?;
        file.sync_all().await?;

        Ok(())
    }

    /// Write audit entry to syslog via the system `logger` command.
    /// On Unix, this is portable across macOS and Linux without requiring a syslog crate.
    /// On Windows, syslog is not available; a one-time warning is logged instead.
    async fn write_to_syslog(&self, syslog_config: &crate::config::SyslogConfig, entry: &str) {
        #[cfg(unix)]
        {
            let priority = format!("{}.info", syslog_config.facility);
            let result = tokio::process::Command::new("logger")
                .args(["-p", &priority, "-t", &syslog_config.tag, entry.trim()])
                .output()
                .await;

            if let Err(error) = result {
                tracing::warn!("Failed to write to syslog via logger command: {}", error);
            }
        }

        #[cfg(windows)]
        {
            // Silence unused-variable warning; `entry` is only consumed on Unix.
            let _ = entry;
            // Windows has no syslog equivalent. Log a one-time warning.
            // Future enhancement: write to Windows Event Log via `eventlog` crate.
            use std::sync::Once;
            static WARN_ONCE: Once = Once::new();
            WARN_ONCE.call_once(|| {
                tracing::warn!(
                    "Syslog audit logging is not supported on Windows. \
                     Configure audit.file instead. (tag={}, facility={})",
                    syslog_config.tag,
                    syslog_config.facility
                );
            });
        }
    }
}
