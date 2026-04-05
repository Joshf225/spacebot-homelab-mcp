use anyhow::Result;
use tracing::warn;

use crate::config::Config;
use crate::connection::DockerClient;

/// Run diagnostics and print results to stdout.
pub async fn run_diagnostics(config: &Config) -> Result<()> {
    println!("Checking Docker hosts:");
    for (name, host) in &config.docker.hosts {
        match check_docker_connection(host).await {
            Ok(()) => {
                println!("  ✓ Docker '{}': {} -> accessible", name, host.host);
            }
            Err(error) => {
                println!("  ✗ Docker '{}': {} -> {}", name, host.host, error);
                println!("    -> Check that Docker daemon is running");
                println!("    -> Verify connection string is correct");
            }
        }
    }

    println!("\nChecking SSH hosts:");
    for (name, host) in &config.ssh.hosts {
        match check_ssh_connection(host).await {
            Ok(()) => {
                println!("  ✓ SSH '{}': {}@{} -> OK", name, host.user, host.host);
            }
            Err(error) => {
                println!(
                    "  ✗ SSH '{}': {}@{} -> {}",
                    name, host.user, host.host, error
                );
                println!("    -> Check that SSH server is running");
                println!("    -> Verify host and port are correct");
                println!("    -> Verify private_key_path is correct");
                println!("    -> Verify SSH user has permissions");
            }
        }
    }

    println!("\nChecking security configuration:");
    check_security_config(config);

    println!("\nConfiguration summary:");
    println!(
        "  {} Docker hosts, {} SSH hosts",
        config.docker.hosts.len(),
        config.ssh.hosts.len()
    );
    println!(
        "  SSH pool: max {} sessions, {} min lifetime, {} min idle, {}s keepalive",
        config.ssh.pool.max_sessions_per_host,
        config.ssh.pool.max_lifetime_secs / 60,
        config.ssh.pool.max_idle_time_secs / 60,
        config.ssh.pool.keepalive_interval_secs,
    );

    if config.audit.file.is_some() {
        println!("  Audit logging: enabled (file)");
    } else if config.audit.syslog.is_some() {
        println!("  Audit logging: enabled (syslog)");
    } else {
        println!("  Audit logging: disabled");
    }

    println!("\nRate limits:");
    if config.rate_limits.limits.is_empty() {
        println!("  No rate limits configured (all tools unrestricted)");
    } else {
        for (tool, limit) in &config.rate_limits.limits {
            println!("  {}: {} req/min", tool, limit.per_minute);
        }
    }

    println!("\nConfirmation rules:");
    if let Some(confirm) = &config.confirm {
        if confirm.is_empty() {
            println!("  None configured");
        } else {
            for (tool, rule) in confirm {
                println!("  {}: {:?}", tool, rule);
            }
        }
    } else {
        println!("  None configured");
    }

    println!("\nReady to start.");
    Ok(())
}

async fn check_docker_connection(host: &crate::config::DockerHost) -> Result<()> {
    let docker = DockerClient::new(host)?;
    docker.validate().await
}

async fn check_ssh_connection(host: &crate::config::SshHost) -> Result<()> {
    if !host.private_key_path.exists() {
        return Err(anyhow::anyhow!("Private key file not found"));
    }

    Ok(())
}

fn check_security_config(config: &Config) {
    let mut warnings = 0;

    for (name, host) in &config.ssh.hosts {
        if host.user == "root" {
            warn!(
                "SSH host '{}' uses root user - this is a security risk",
                name
            );
            warnings += 1;
        }
    }

    if config.audit.file.is_none() && config.audit.syslog.is_none() {
        warn!("Audit logging is not configured - tool invocations will not be logged");
        warnings += 1;
    }

    if config.ssh.command_allowlist.allowed_prefixes.is_empty() {
        warn!("No SSH command allowlist configured - all commands will be blocked");
        warnings += 1;
    }

    if warnings > 0 {
        println!("  {} security warnings (see logs)", warnings);
    } else {
        println!("  ✓ Security checks passed");
    }
}
