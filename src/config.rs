use anyhow::{anyhow, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::info;

use crate::confirmation::ConfirmRule;

/// Main configuration structure.
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub docker: DockerConfig,
    #[serde(default)]
    pub ssh: SshConfig,
    #[serde(default)]
    pub audit: AuditConfig,
    #[serde(default)]
    pub rate_limits: RateLimitConfig,
    #[serde(default)]
    pub tools: ToolsConfig,
    #[serde(default)]
    pub confirm: Option<HashMap<String, ConfirmRule>>,
    #[serde(default)]
    pub metrics: MetricsConfig,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct DockerConfig {
    #[serde(default)]
    pub hosts: HashMap<String, DockerHost>,
}

impl DockerConfig {
    pub fn len(&self) -> usize {
        self.hosts.len()
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct DockerHost {
    pub host: String,
    pub cert_path: Option<PathBuf>,
    pub key_path: Option<PathBuf>,
    pub ca_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SshConfig {
    #[serde(default)]
    pub hosts: HashMap<String, SshHost>,
    #[serde(default)]
    pub pool: SshPoolConfig,
    #[serde(default)]
    pub command_allowlist: CommandAllowlist,
}

#[derive(Clone, Deserialize)]
pub struct SshHost {
    pub host: String,
    pub port: Option<u16>,
    pub user: String,
    pub private_key_path: PathBuf,
    #[serde(default)]
    pub private_key_passphrase: Option<String>,
}

impl std::fmt::Debug for SshHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SshHost")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("user", &self.user)
            .field("private_key_path", &self.private_key_path)
            .field(
                "private_key_passphrase",
                &self.private_key_passphrase.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct SshPoolConfig {
    #[serde(default = "default_max_sessions")]
    pub max_sessions_per_host: usize,
    #[serde(default = "default_max_lifetime")]
    pub max_lifetime_secs: u64,
    #[serde(default = "default_max_idle")]
    pub max_idle_time_secs: u64,
    #[serde(default = "default_connect_timeout")]
    pub connect_timeout_secs: u64,
    #[serde(default = "default_checkout_timeout")]
    pub checkout_timeout_secs: u64,
    #[serde(default = "default_keepalive_interval")]
    pub keepalive_interval_secs: u64,
    /// Maximum concurrent channels per SSH session.
    /// Default: 10. Set to 1 to disable multiplexing (V1 behavior).
    #[serde(default = "default_max_channels")]
    pub max_channels_per_session: usize,
}

impl Default for SshPoolConfig {
    fn default() -> Self {
        Self {
            max_sessions_per_host: default_max_sessions(),
            max_lifetime_secs: default_max_lifetime(),
            max_idle_time_secs: default_max_idle(),
            connect_timeout_secs: default_connect_timeout(),
            checkout_timeout_secs: default_checkout_timeout(),
            keepalive_interval_secs: default_keepalive_interval(),
            max_channels_per_session: default_max_channels(),
        }
    }
}

fn default_max_sessions() -> usize {
    3
}

fn default_max_lifetime() -> u64 {
    1800
}

fn default_max_idle() -> u64 {
    300
}

fn default_connect_timeout() -> u64 {
    10
}

fn default_checkout_timeout() -> u64 {
    5
}

fn default_keepalive_interval() -> u64 {
    60
}

fn default_max_channels() -> usize {
    10
}

#[derive(Debug, Clone, Deserialize)]
pub struct CommandAllowlist {
    #[serde(default)]
    pub allowed_prefixes: Vec<String>,
    #[serde(default)]
    pub blocked_patterns: Vec<String>,
}

impl Default for CommandAllowlist {
    fn default() -> Self {
        Self {
            allowed_prefixes: vec![
                "docker".to_string(),
                "zpool".to_string(),
                "zfs".to_string(),
                "df".to_string(),
                "free".to_string(),
                "uptime".to_string(),
                "systemctl status".to_string(),
                "systemctl is-active".to_string(),
                "systemctl list-units".to_string(),
                "journalctl".to_string(),
                "sudo systemctl status".to_string(),
                "sudo systemctl restart".to_string(),
                "sudo zpool".to_string(),
                "sudo zfs".to_string(),
            ],
            blocked_patterns: vec![
                "rm -rf".to_string(),
                "dd if=".to_string(),
                "mkfs".to_string(),
                "fdisk".to_string(),
                "parted".to_string(),
                "> /dev/".to_string(),
                "| bash".to_string(),
                "| sh".to_string(),
                "; ".to_string(),
                "&& ".to_string(),
                "$(".to_string(),
                "`".to_string(),
            ],
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct AuditConfig {
    pub file: Option<PathBuf>,
    pub syslog: Option<SyslogConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SyslogConfig {
    pub facility: String,
    pub tag: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RateLimitMode {
    #[default]
    Global,
    PerCaller,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RateLimitConfig {
    /// Rate limiting mode: "global" (default) or "per_caller"
    #[serde(default)]
    pub mode: RateLimitMode,
    #[serde(default)]
    pub limits: HashMap<String, RateLimit>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RateLimit {
    pub per_minute: u32,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ToolsConfig {
    pub enabled: Option<Vec<String>>,
}

impl ToolsConfig {
    /// Check if a tool is enabled by configuration.
    /// - `None` (field omitted from config): all tools enabled
    /// - `Some([])` (explicitly `enabled = []`): NO tools enabled
    /// - `Some(["foo", "bar"])`: only listed tools enabled
    ///
    /// Per security-approach.md Layer 2: "Only explicitly enabled tools
    /// are registered with MCP."
    pub fn is_enabled(&self, name: &str) -> bool {
        match &self.enabled {
            None => true,
            Some(list) => list.iter().any(|tool| tool == name),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct MetricsConfig {
    /// Enable Prometheus metrics endpoint
    #[serde(default)]
    pub enabled: bool,
    /// Listen address for the metrics HTTP server (default: "127.0.0.1:9090")
    #[serde(default = "default_metrics_listen")]
    pub listen: String,
}

fn default_metrics_listen() -> String {
    "127.0.0.1:9090".to_string()
}

impl Config {
    /// Load configuration from a TOML file.
    pub fn load(path: Option<PathBuf>) -> Result<Self> {
        let config_path = if let Some(path) = path {
            path
        } else {
            let home =
                std::env::var("HOME").map_err(|_| anyhow!("HOME environment variable not set"))?;
            PathBuf::from(home)
                .join(".spacebot-homelab")
                .join("config.toml")
        };

        info!("Loading configuration from {:?}", config_path);

        if !config_path.exists() {
            return Err(anyhow!(
                "Configuration file not found at {:?}. Create ~/.spacebot-homelab/config.toml or provide --config <path>",
                config_path
            ));
        }

        check_config_permissions(&config_path)?;

        let config_str = std::fs::read_to_string(&config_path)?;
        let mut config: Config = toml::from_str(&config_str)?;
        config.validate()?;

        info!(
            "Configuration loaded successfully: {} Docker hosts, {} SSH hosts",
            config.docker.hosts.len(),
            config.ssh.hosts.len()
        );

        Ok(config)
    }

    fn validate(&mut self) -> Result<()> {
        for host in self.docker.hosts.values_mut() {
            if let Some(path) = &host.cert_path {
                host.cert_path = Some(expand_home(path));
            }
            if let Some(path) = &host.key_path {
                host.key_path = Some(expand_home(path));
            }
            if let Some(path) = &host.ca_path {
                host.ca_path = Some(expand_home(path));
            }

            if let Some(path) = &host.cert_path {
                ensure_exists(path, "Docker certificate")?;
            }
            if let Some(path) = &host.key_path {
                ensure_exists(path, "Docker private key")?;
            }
            if let Some(path) = &host.ca_path {
                ensure_exists(path, "Docker CA certificate")?;
            }
        }

        for (name, host) in &mut self.ssh.hosts {
            if host.user == "root" {
                tracing::warn!(
                    "SSH host '{}' is configured with user 'root'. Use a restricted user when possible.",
                    name
                );
            }

            host.private_key_path = expand_home(&host.private_key_path);
            ensure_exists(
                &host.private_key_path,
                &format!("SSH private key for host '{}'", name),
            )?;

            // Resolve env var references in passphrase (e.g. "$SSH_KEY_PASS" or "${SSH_KEY_PASS}")
            if let Some(passphrase) = &host.private_key_passphrase {
                host.private_key_passphrase = Some(resolve_env_var(passphrase));
            }
        }

        if self.ssh.command_allowlist.allowed_prefixes.is_empty() {
            self.ssh.command_allowlist = CommandAllowlist::default();
            info!(
                "Using default SSH command allowlist with {} prefixes",
                self.ssh.command_allowlist.allowed_prefixes.len()
            );
        }

        if let Some(rules) = &self.confirm {
            for (name, rule) in rules {
                rule.validate(name)?;
            }
        }

        if let Some(audit_path) = &self.audit.file {
            let audit_path = expand_home(audit_path);
            self.audit.file = Some(audit_path.clone());
            if let Some(parent) = audit_path.parent() {
                if !parent.exists() {
                    std::fs::create_dir_all(parent)?;
                    info!("Created audit log directory: {:?}", parent);
                }
            }
        }

        Ok(())
    }
}

fn ensure_exists(path: &Path, description: &str) -> Result<()> {
    if !path.exists() {
        return Err(anyhow!("{} not found at {:?}", description, path));
    }
    Ok(())
}

fn expand_home(path: &Path) -> PathBuf {
    let path_str = path.to_string_lossy();
    if path_str == "~" {
        return std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| path.to_path_buf());
    }

    if let Some(rest) = path_str.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }

    path.to_path_buf()
}

/// Resolve environment variable references in a string value.
/// Supports `$VAR_NAME` and `${VAR_NAME}` syntax. If the entire string
/// is a single env var reference and the variable is set, the value is
/// replaced. Otherwise the original string is returned as-is.
fn resolve_env_var(value: &str) -> String {
    let trimmed = value.trim();

    // ${VAR_NAME} form
    if let Some(var_name) = trimmed
        .strip_prefix("${")
        .and_then(|rest| rest.strip_suffix('}'))
    {
        if let Ok(resolved) = std::env::var(var_name) {
            return resolved;
        }
    }
    // $VAR_NAME form (whole string must be the variable reference)
    else if let Some(var_name) = trimmed.strip_prefix('$') {
        if !var_name.is_empty()
            && var_name
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '_')
        {
            if let Ok(resolved) = std::env::var(var_name) {
                return resolved;
            }
        }
    }

    value.to_string()
}

/// Validate config file permissions are not too open (Layer 1 security).
/// Rejects files readable by "other" or writable/executable by "group".
/// Acceptable modes: 0600 (owner only) or 0640 (owner + group read).
#[cfg(unix)]
fn check_config_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let metadata = std::fs::metadata(path)?;
    let mode = metadata.permissions().mode() & 0o777;

    let other_bits = mode & 0o007;
    let group_write_exec = mode & 0o030;

    if other_bits != 0 {
        return Err(anyhow!(
            "Configuration file {:?} has mode {:04o} — world-accessible. \
             Config may contain sensitive paths and credentials. \
             Fix with: chmod 600 {:?}",
            path,
            mode,
            path
        ));
    }

    if group_write_exec != 0 {
        return Err(anyhow!(
            "Configuration file {:?} has mode {:04o} — group write/execute is allowed. \
             Fix with: chmod 640 {:?}",
            path,
            mode,
            path
        ));
    }

    info!("Config file permissions {:04o} OK", mode);
    Ok(())
}

#[cfg(not(unix))]
fn check_config_permissions(_path: &Path) -> Result<()> {
    // Permission checks only supported on Unix
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tools_config_none_enables_all() {
        let config = ToolsConfig { enabled: None };
        assert!(config.is_enabled("anything"));
        assert!(config.is_enabled("ssh.exec"));
        assert!(config.is_enabled("docker.container.list"));
    }

    #[test]
    fn test_tools_config_empty_disables_all() {
        let config = ToolsConfig {
            enabled: Some(vec![]),
        };
        assert!(!config.is_enabled("anything"));
        assert!(!config.is_enabled("ssh.exec"));
    }

    #[test]
    fn test_tools_config_specific_list() {
        let config = ToolsConfig {
            enabled: Some(vec!["ssh.exec".into()]),
        };
        assert!(config.is_enabled("ssh.exec"));
        assert!(!config.is_enabled("docker.container.list"));
    }

    #[test]
    fn test_resolve_env_var_dollar_syntax() {
        unsafe { std::env::set_var("TEST_PASSPHRASE_DOLLAR", "secret123") };
        let result = resolve_env_var("$TEST_PASSPHRASE_DOLLAR");
        assert_eq!(result, "secret123");
        unsafe { std::env::remove_var("TEST_PASSPHRASE_DOLLAR") };
    }

    #[test]
    fn test_resolve_env_var_braces_syntax() {
        unsafe { std::env::set_var("TEST_PASSPHRASE_BRACES", "secret123") };
        let result = resolve_env_var("${TEST_PASSPHRASE_BRACES}");
        assert_eq!(result, "secret123");
        unsafe { std::env::remove_var("TEST_PASSPHRASE_BRACES") };
    }

    #[test]
    fn test_resolve_env_var_literal_passthrough() {
        let result = resolve_env_var("literal_value");
        assert_eq!(result, "literal_value");
    }

    #[cfg(unix)]
    #[test]
    fn test_check_config_permissions_rejects_world_readable() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::set_permissions(tmp.path(), std::fs::Permissions::from_mode(0o644)).unwrap();
        let result = check_config_permissions(tmp.path());
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("world-accessible"),
            "error should mention world-accessible, got: {}",
            msg
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_check_config_permissions_accepts_0600() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::set_permissions(tmp.path(), std::fs::Permissions::from_mode(0o600)).unwrap();
        let result = check_config_permissions(tmp.path());
        assert!(result.is_ok(), "0600 should be accepted, got: {:?}", result);
    }
}
