use anyhow::{Result, anyhow};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::fs;
use tokio::time::timeout;

use crate::audit::AuditLogger;
use crate::config::CommandAllowlist;
use crate::confirmation::ConfirmationManager;
use crate::connection::ConnectionManager;
use crate::tools::{truncate_output, wrap_output_envelope};

const OUTPUT_MAX_CHARS: usize = 5_000;
const MAX_FILE_SIZE: u64 = 50 * 1024 * 1024;

pub async fn exec(
    manager: Arc<ConnectionManager>,
    confirmation: Arc<ConfirmationManager>,
    host: String,
    command: String,
    timeout_secs: Option<u64>,
    dry_run: Option<bool>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    validate_allowed_prefix(&command, &manager.config().ssh.command_allowlist)?;

    // Check blocked patterns FIRST, before issuing any confirmation tokens.
    // This prevents tokens from being issued for commands that would be blocked anyway.
    if let Some(pattern) = match_blocked_pattern(&command, &manager.config().ssh.command_allowlist)
    {
        let error = anyhow!(
            "Command blocked: contains dangerous pattern '{}'. This pattern is blocked for safety.",
            pattern
        );
        audit
            .log("ssh.exec", &host, "blocked", Some(&error.to_string()))
            .await
            .ok();
        return Err(error);
    }

    if dry_run.unwrap_or(false) {
        let output = format!(
            "DRY RUN: Command '{}' passes prefix validation for host '{}'. Set dry_run=false to execute.",
            command, host
        );
        audit
            .log("ssh.exec", &host, "dry_run", Some(&command))
            .await
            .ok();
        return Ok(wrap_output_envelope("ssh.exec", &output));
    }

    let params_json = serde_json::json!({
        "host": host,
        "command": command,
        "timeout": timeout_secs,
    })
    .to_string();

    if let Some(response) = confirmation
        .check_and_maybe_require(
            "ssh.exec",
            Some(&command),
            &format!("About to run '{}' on host '{}'", command, host),
            &params_json,
        )
        .await?
    {
        audit
            .log("ssh.exec", &host, "confirmation_required", Some(&command))
            .await
            .ok();
        return Ok(response);
    }

    exec_confirmed(manager, host, command, timeout_secs, audit).await
}

pub async fn exec_confirmed(
    manager: Arc<ConnectionManager>,
    host: String,
    command: String,
    timeout_secs: Option<u64>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    validate_allowed_prefix(&command, &manager.config().ssh.command_allowlist)?;

    // Blocked patterns must be enforced even for confirmed commands.
    // Without this check, a command matching a confirmation when_pattern
    // would skip the blocklist entirely (P1-3 security fix).
    if let Some(pattern) = match_blocked_pattern(&command, &manager.config().ssh.command_allowlist)
    {
        let error = anyhow!(
            "Command blocked: contains dangerous pattern '{}'. This pattern is blocked for safety even after confirmation.",
            pattern
        );
        audit
            .log("ssh.exec", &host, "blocked", Some(&error.to_string()))
            .await
            .ok();
        return Err(error);
    }

    let acquired = manager.ssh_acquire_channel(&host).await?;
    let mut broken = false;
    let timeout_duration = Duration::from_secs(timeout_secs.unwrap_or(30).min(300));

    let execution = timeout(timeout_duration, async {
        let mut channel = acquired
            .handle
            .channel_open_session()
            .await
            .map_err(|error| anyhow!("Failed to open SSH channel: {}", error))?;

        channel
            .exec(true, command.as_str())
            .await
            .map_err(|error| anyhow!("Failed to execute SSH command: {}", error))?;

        let mut stdout: Vec<u8> = Vec::new();
        let mut stderr: Vec<u8> = Vec::new();
        let mut exit_status = None;

        while let Some(message) = channel.wait().await {
            match message {
                russh::ChannelMsg::Data { data } => stdout.extend_from_slice(&data),
                russh::ChannelMsg::ExtendedData { data, ext: 1 } => {
                    stderr.extend_from_slice(&data);
                }
                russh::ChannelMsg::ExitStatus {
                    exit_status: status,
                } => {
                    exit_status = Some(status);
                }
                _ => {}
            }
        }

        Ok::<_, anyhow::Error>((stdout, stderr, exit_status))
    })
    .await;

    let result = match execution {
        Ok(Ok((stdout, stderr, exit_status))) => {
            let output = render_exec_output(&stdout, &stderr, exit_status);
            audit
                .log("ssh.exec", &host, "success", Some(&command))
                .await
                .ok();
            Ok(wrap_output_envelope(
                "ssh.exec",
                &truncate_output(&output, OUTPUT_MAX_CHARS),
            ))
        }
        Ok(Err(error)) => {
            broken = true;
            audit
                .log("ssh.exec", &host, "error", Some(&error.to_string()))
                .await
                .ok();
            Err(error)
        }
        Err(_) => {
            broken = true;
            let error = anyhow!(
                "Command timed out after {}s on host '{}'.",
                timeout_secs.unwrap_or(30).min(300),
                host
            );
            audit
                .log("ssh.exec", &host, "timeout", Some(&command))
                .await
                .ok();
            Err(error)
        }
    };

    manager.ssh_release_channel(&host, acquired, broken).await;
    result
}

pub async fn upload(
    manager: Arc<ConnectionManager>,
    host: String,
    local_path: String,
    remote_path: String,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    if !Path::new(&local_path).exists() {
        return Err(anyhow!("Local file does not exist: {}", local_path));
    }

    let metadata = std::fs::metadata(&local_path)
        .map_err(|error| anyhow!("Failed to stat local file '{}': {}", local_path, error))?;
    if metadata.len() > MAX_FILE_SIZE {
        return Err(anyhow!(
            "File too large: {} bytes (max {} bytes / 50MB)",
            metadata.len(),
            MAX_FILE_SIZE
        ));
    }

    let acquired = manager.ssh_acquire_channel(&host).await?;
    let mut broken = false;
    let result: Result<String> = async {
        let channel = acquired
            .handle
            .channel_open_session()
            .await
            .map_err(|error| anyhow!("Failed to open SSH channel: {}", error))?;
        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(|error| anyhow!("Failed to request SFTP subsystem: {}", error))?;
        let sftp = russh_sftp::client::SftpSession::new(channel.into_stream())
            .await
            .map_err(|error| anyhow!("Failed to initialize SFTP session: {}", error))?;

        let data = fs::read(&local_path)
            .await
            .map_err(|error| anyhow!("Failed to read local file '{}': {}", local_path, error))?;
        sftp.write(&remote_path, &data)
            .await
            .map_err(|error| anyhow!("Failed to upload to '{}': {}", remote_path, error))?;
        sftp.close().await.ok();

        Ok(format!(
            "Uploaded {} bytes from '{}' to '{}:{}'.",
            data.len(),
            local_path,
            host,
            remote_path
        ))
    }
    .await;

    if result.is_err() {
        broken = true;
    }
    manager.ssh_release_channel(&host, acquired, broken).await;

    match result {
        Ok(output) => {
            audit
                .log("ssh.upload", &host, "success", Some(&remote_path))
                .await
                .ok();
            Ok(wrap_output_envelope("ssh.upload", &output))
        }
        Err(error) => {
            audit
                .log("ssh.upload", &host, "error", Some(&error.to_string()))
                .await
                .ok();
            Err(error)
        }
    }
}

pub async fn download(
    manager: Arc<ConnectionManager>,
    host: String,
    remote_path: String,
    local_path: Option<String>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let local_dest = local_path.unwrap_or_else(|| {
        let temp = std::env::temp_dir();
        let filename = format!(
            "homelab-download-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        );
        temp.join(filename).to_string_lossy().into_owned()
    });

    let acquired = manager.ssh_acquire_channel(&host).await?;
    let mut broken = false;
    let result: Result<String> = async {
        let channel = acquired
            .handle
            .channel_open_session()
            .await
            .map_err(|error| anyhow!("Failed to open SSH channel: {}", error))?;
        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(|error| anyhow!("Failed to request SFTP subsystem: {}", error))?;
        let sftp = russh_sftp::client::SftpSession::new(channel.into_stream())
            .await
            .map_err(|error| anyhow!("Failed to initialize SFTP session: {}", error))?;

        let data = sftp
            .read(&remote_path)
            .await
            .map_err(|error| anyhow!("Failed to download '{}': {}", remote_path, error))?;
        if data.len() as u64 > MAX_FILE_SIZE {
            return Err(anyhow!(
                "Remote file too large: {} bytes (max {} bytes / 50MB)",
                data.len(),
                MAX_FILE_SIZE
            ));
        }

        if let Some(parent) = Path::new(&local_dest).parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).await.ok();
            }
        }

        fs::write(&local_dest, &data)
            .await
            .map_err(|error| anyhow!("Failed to write local file '{}': {}", local_dest, error))?;
        sftp.close().await.ok();

        Ok(format!(
            "Downloaded {} bytes from '{}:{}' to '{}'.",
            data.len(),
            host,
            remote_path,
            local_dest
        ))
    }
    .await;

    if result.is_err() {
        broken = true;
    }
    manager.ssh_release_channel(&host, acquired, broken).await;

    match result {
        Ok(output) => {
            audit
                .log("ssh.download", &host, "success", Some(&remote_path))
                .await
                .ok();
            Ok(wrap_output_envelope("ssh.download", &output))
        }
        Err(error) => {
            audit
                .log("ssh.download", &host, "error", Some(&error.to_string()))
                .await
                .ok();
            Err(error)
        }
    }
}

#[cfg(test)]
pub fn validate_command(command: &str, allowlist: &CommandAllowlist) -> Result<()> {
    validate_allowed_prefix(command, allowlist)?;
    if let Some(pattern) = match_blocked_pattern(command, allowlist) {
        return Err(anyhow!(
            "Command blocked: contains dangerous pattern '{}'. This pattern is in the blocked list for safety.",
            pattern
        ));
    }
    Ok(())
}

fn validate_allowed_prefix(command: &str, allowlist: &CommandAllowlist) -> Result<()> {
    let allowed = allowlist
        .allowed_prefixes
        .iter()
        .any(|prefix| command == prefix || command.starts_with(&format!("{} ", prefix)));

    if !allowed {
        return Err(anyhow!(
            "Command '{}' does not match any allowed prefix. Allowed: {}",
            command,
            allowlist.allowed_prefixes.join(", ")
        ));
    }

    Ok(())
}

fn match_blocked_pattern<'a>(command: &str, allowlist: &'a CommandAllowlist) -> Option<&'a str> {
    allowlist
        .blocked_patterns
        .iter()
        .find_map(|pattern| command.contains(pattern).then_some(pattern.as_str()))
}

fn render_exec_output(stdout: &[u8], stderr: &[u8], exit_status: Option<u32>) -> String {
    let stdout = String::from_utf8_lossy(stdout);
    let stderr = String::from_utf8_lossy(stderr);

    let mut output = String::new();
    if let Some(status) = exit_status {
        output.push_str(&format!("Exit code: {}\n", status));
    }
    if !stdout.is_empty() {
        output.push_str("--- stdout ---\n");
        output.push_str(&stdout);
        if !stdout.ends_with('\n') {
            output.push('\n');
        }
    }
    if !stderr.is_empty() {
        output.push_str("--- stderr ---\n");
        output.push_str(&stderr);
        if !stderr.ends_with('\n') {
            output.push('\n');
        }
    }
    if output.trim().is_empty() {
        output.push_str("Command completed with no output.");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_command_validation_allowed() {
        let allowlist = CommandAllowlist {
            allowed_prefixes: vec!["docker".to_string(), "df".to_string()],
            blocked_patterns: vec!["rm -rf".to_string()],
        };

        assert!(validate_command("docker ps", &allowlist).is_ok());
        assert!(validate_command("docker", &allowlist).is_ok());
        assert!(validate_command("df -h", &allowlist).is_ok());
    }

    #[test]
    fn test_command_validation_blocked() {
        let allowlist = CommandAllowlist {
            allowed_prefixes: vec!["docker".to_string(), "sudo".to_string()],
            blocked_patterns: vec!["rm -rf".to_string()],
        };

        assert!(validate_command("sudo rm -rf /", &allowlist).is_err());
    }

    #[test]
    fn test_command_validation_not_allowed() {
        let allowlist = CommandAllowlist {
            allowed_prefixes: vec!["docker".to_string()],
            blocked_patterns: vec![],
        };

        assert!(validate_command("apt install foo", &allowlist).is_err());
    }

    #[test]
    fn test_command_validation_prefix_strict() {
        let allowlist = CommandAllowlist {
            allowed_prefixes: vec!["docker".to_string()],
            blocked_patterns: vec![],
        };

        assert!(validate_command("dockerrm ps", &allowlist).is_err());
    }
}
