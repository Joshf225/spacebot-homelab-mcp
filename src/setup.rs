//! Interactive setup wizard for spacebot-homelab-mcp.
//!
//! Invoked via `spacebot-homelab-mcp setup`.  Walks the user through
//! configuring SSH hosts, Docker, confirmation rules, and rate limits,
//! then writes a TOML config file with secure defaults and `0600` permissions.
//!
//! Design decisions:
//! - Merges into existing config if the target file already exists.
//! - Writes partial progress after each section so a crash/Ctrl-C doesn't
//!   lose everything already entered.
//! - All validation is syntax + file-existence only (no live connectivity).
//! - Security defaults are applied automatically; the user can opt out.

use anyhow::{anyhow, Context, Result};
use console::style;
use dialoguer::{theme::ColorfulTheme, Confirm, Input, Select};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

// ── Output-only config structs ─────────────────────────────────────────────
//
// These mirror the shape that `Config::load` expects in TOML but are kept
// separate to avoid coupling serialization concerns to the runtime config
// structs.  Only the fields the wizard actually sets are included; pool
// settings and other advanced knobs are left for the user to add manually.

#[derive(Debug, Default, Serialize, Deserialize)]
struct WizardConfig {
    #[serde(default, skip_serializing_if = "WizardDockerSection::is_empty")]
    docker: WizardDockerSection,
    #[serde(default, skip_serializing_if = "WizardSshSection::is_empty")]
    ssh: WizardSshSection,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    confirm: Option<HashMap<String, WizardConfirmRule>>,
    #[serde(default, skip_serializing_if = "WizardRateLimitSection::is_empty")]
    rate_limits: WizardRateLimitSection,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct WizardDockerSection {
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    hosts: HashMap<String, WizardDockerHost>,
}

impl WizardDockerSection {
    fn is_empty(&self) -> bool {
        self.hosts.is_empty()
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct WizardDockerHost {
    host: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct WizardSshSection {
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    hosts: HashMap<String, WizardSshHost>,
}

impl WizardSshSection {
    fn is_empty(&self) -> bool {
        self.hosts.is_empty()
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct WizardSshHost {
    host: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    port: Option<u16>,
    user: String,
    private_key_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    private_key_passphrase: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct WizardRateLimitSection {
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    limits: HashMap<String, WizardRateLimit>,
}

impl WizardRateLimitSection {
    fn is_empty(&self) -> bool {
        self.limits.is_empty()
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct WizardRateLimit {
    per_minute: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
enum WizardConfirmRule {
    Always(String),
    WhenPattern { when_pattern: Vec<String> },
}

// ── Security defaults ──────────────────────────────────────────────────────

fn default_confirm_rules() -> HashMap<String, WizardConfirmRule> {
    let mut rules = HashMap::new();
    rules.insert(
        "ssh.exec".to_string(),
        WizardConfirmRule::WhenPattern {
            when_pattern: vec![
                "rm -rf".to_string(),
                "systemctl restart".to_string(),
                "systemctl stop".to_string(),
                "reboot".to_string(),
                "shutdown".to_string(),
                "mkfs".to_string(),
                "dd if=".to_string(),
                "fdisk".to_string(),
            ],
        },
    );
    rules
}

// ── Wizard entry point ─────────────────────────────────────────────────────

pub fn run_setup() -> Result<()> {
    let theme = ColorfulTheme::default();

    print_banner();

    // Step 1: Config file path
    let config_path = prompt_config_path(&theme)?;

    // Step 2: Load existing config for merge (pre-populates prompts)
    let mut config = load_existing_config(&config_path)?;

    // Step 3: SSH Hosts (writes partial config after completion)
    configure_ssh_hosts(&theme, &mut config)?;
    write_config(&config_path, &config)?;

    // Step 4: Confirmation rules (writes partial config after completion)
    configure_confirm_rules(&theme, &mut config)?;
    write_config(&config_path, &config)?;

    // Step 5: Rate limits
    configure_rate_limits(&theme, &mut config)?;

    // Step 6: Final write
    write_config(&config_path, &config)?;

    // Step 7: Summary + next steps + Spacebot snippet
    print_summary(&config_path, &config);

    Ok(())
}

// ── Banner ─────────────────────────────────────────────────────────────────

fn print_banner() {
    println!();
    println!(
        "{}",
        style("╔══════════════════════════════════════════╗")
            .cyan()
            .bold()
    );
    println!(
        "{}",
        style("║   Spacebot Homelab MCP — Setup Wizard    ║")
            .cyan()
            .bold()
    );
    println!(
        "{}",
        style("╚══════════════════════════════════════════╝")
            .cyan()
            .bold()
    );
    println!();
    println!("This wizard will create or update your config file with secure defaults.");
    println!("You can re-run it at any time to add hosts or change settings.");
    println!();
}

// ── Config path ────────────────────────────────────────────────────────────

fn prompt_config_path(theme: &ColorfulTheme) -> Result<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "~".to_string());
    let default_path = format!("{}/.config/spacebot-homelab-mcp/config.toml", home);

    let path_str: String = Input::with_theme(theme)
        .with_prompt("Config file path")
        .default(default_path)
        .interact_text()
        .context("Failed to read config path")?;

    Ok(PathBuf::from(path_str))
}

// ── Existing config loader ─────────────────────────────────────────────────

fn load_existing_config(path: &Path) -> Result<WizardConfig> {
    if !path.exists() {
        return Ok(WizardConfig::default());
    }

    println!();
    println!(
        "{} Found existing config at {}",
        style("→").yellow(),
        style(path.display()).cyan()
    );
    println!("  Existing settings will be used as defaults.");

    let content = std::fs::read_to_string(path).context("Failed to read existing config file")?;

    match toml::from_str::<WizardConfig>(&content) {
        Ok(config) => Ok(config),
        Err(err) => {
            println!(
                "{} Could not parse existing config ({}). Starting fresh.",
                style("⚠").yellow(),
                err
            );
            Ok(WizardConfig::default())
        }
    }
}

// ── SSH Hosts ──────────────────────────────────────────────────────────────

fn configure_ssh_hosts(theme: &ColorfulTheme, config: &mut WizardConfig) -> Result<()> {
    println!();
    println!(
        "{}",
        style("── SSH Hosts ──────────────────────────────────").bold()
    );
    println!("Add the homelab machines you want Spacebot to manage via SSH.");
    println!();

    if !config.ssh.hosts.is_empty() {
        println!("Existing SSH hosts:");
        for (alias, host) in &config.ssh.hosts {
            println!(
                "  {} {} ({}@{})",
                style("•").green(),
                style(alias).bold(),
                host.user,
                host.host
            );
        }
        println!();

        let add_more = Confirm::with_theme(theme)
            .with_prompt("Add more SSH hosts?")
            .default(true)
            .interact()
            .context("Failed to read input")?;

        if !add_more {
            return Ok(());
        }
        println!();
    }

    loop {
        add_ssh_host(theme, config)?;

        println!();
        let add_another = Confirm::with_theme(theme)
            .with_prompt("Add another SSH host?")
            .default(false)
            .interact()
            .context("Failed to read input")?;

        if !add_another {
            break;
        }
        println!();
    }

    Ok(())
}

fn add_ssh_host(theme: &ColorfulTheme, config: &mut WizardConfig) -> Result<()> {
    println!("{}", style("New SSH Host").bold());

    // Alias
    let alias: String = Input::with_theme(theme)
        .with_prompt("Host alias (e.g. 'mynas', 'pi4')")
        .validate_with(|input: &String| {
            let trimmed = input.trim();
            if trimmed.is_empty() {
                Err("Alias cannot be empty".to_string())
            } else if trimmed.contains(' ') {
                Err("Alias cannot contain spaces".to_string())
            } else {
                Ok(())
            }
        })
        .interact_text()
        .context("Failed to read alias")?;

    // Hostname / IP
    let hostname: String = Input::with_theme(theme)
        .with_prompt("Hostname or IP address")
        .validate_with(|input: &String| {
            if input.trim().is_empty() {
                Err("Hostname cannot be empty".to_string())
            } else {
                Ok(())
            }
        })
        .interact_text()
        .context("Failed to read hostname")?;

    // Port
    let port_str: String = Input::with_theme(theme)
        .with_prompt("SSH port")
        .default("22".to_string())
        .interact_text()
        .context("Failed to read port")?;
    let port: u16 = port_str
        .trim()
        .parse()
        .map_err(|_| anyhow!("Invalid port number: '{}'", port_str.trim()))?;
    let port_opt = if port == 22 { None } else { Some(port) };

    // Username
    let user: String = Input::with_theme(theme)
        .with_prompt("SSH username")
        .validate_with(|input: &String| {
            if input.trim().is_empty() {
                Err("Username cannot be empty".to_string())
            } else {
                Ok(())
            }
        })
        .interact_text()
        .context("Failed to read username")?;

    // SSH key
    let key_path = configure_ssh_key(theme, &user, &hostname)?;

    // Docker
    let has_docker = Confirm::with_theme(theme)
        .with_prompt(format!("Does '{}' run Docker?", alias))
        .default(false)
        .interact()
        .context("Failed to read Docker choice")?;

    if has_docker {
        let socket = "unix:///var/run/docker.sock";
        println!(
            "  {} Using default Docker socket: {}",
            style("→").cyan(),
            style(socket).cyan()
        );
        println!(
            "  {} This is correct for most Docker installations.",
            style("ℹ").blue()
        );
        config.docker.hosts.insert(
            alias.clone(),
            WizardDockerHost {
                host: socket.to_string(),
            },
        );
    }

    config.ssh.hosts.insert(
        alias.clone(),
        WizardSshHost {
            host: hostname,
            port: port_opt,
            user,
            private_key_path: key_path,
            private_key_passphrase: None,
        },
    );

    println!(
        "  {} Host '{}' added.",
        style("✓").green(),
        style(&alias).bold()
    );

    Ok(())
}

// ── SSH key handling ───────────────────────────────────────────────────────

fn configure_ssh_key(theme: &ColorfulTheme, user: &str, hostname: &str) -> Result<String> {
    let has_key = Confirm::with_theme(theme)
        .with_prompt("Do you have an existing SSH key for this host?")
        .default(true)
        .interact()
        .context("Failed to read SSH key choice")?;

    if has_key {
        let home = std::env::var("HOME").unwrap_or_else(|_| "~".to_string());
        let default_key = format!("{}/.ssh/id_ed25519", home);

        let key_path: String = Input::with_theme(theme)
            .with_prompt("SSH private key path")
            .default(default_key)
            .validate_with(|input: &String| {
                let expanded = expand_home(input);
                if Path::new(&expanded).exists() {
                    Ok(())
                } else {
                    Err(format!("Key file not found: '{}'", input))
                }
            })
            .interact_text()
            .context("Failed to read key path")?;

        Ok(key_path)
    } else {
        generate_ssh_key(theme, user, hostname)
    }
}

fn generate_ssh_key(theme: &ColorfulTheme, user: &str, hostname: &str) -> Result<String> {
    println!();
    println!("{}", style("Generating SSH key pair (Ed25519)").bold());

    let home = std::env::var("HOME").unwrap_or_else(|_| "~".to_string());
    let default_key_path = format!("{}/.ssh/spacebot-homelab", home);

    let key_path: String = Input::with_theme(theme)
        .with_prompt("Save key to")
        .default(default_key_path)
        .interact_text()
        .context("Failed to read key path")?;

    let expanded = expand_home(&key_path);

    // Ensure ~/.ssh exists
    if let Some(parent) = Path::new(&expanded).parent() {
        std::fs::create_dir_all(parent).context("Failed to create .ssh directory")?;
    }

    // Run ssh-keygen
    println!("  {} Running ssh-keygen...", style("→").cyan());
    let status = Command::new("ssh-keygen")
        .args([
            "-t",
            "ed25519",
            "-f",
            &expanded,
            "-N",
            "",
            "-C",
            "spacebot-homelab",
        ])
        .status()
        .context("Failed to run ssh-keygen — is it installed?")?;

    if !status.success() {
        return Err(anyhow!(
            "ssh-keygen failed with exit code {:?}",
            status.code()
        ));
    }

    println!(
        "  {} Key pair generated at {}",
        style("✓").green(),
        style(&expanded).cyan()
    );

    // Print public key
    let pub_key_path = format!("{}.pub", expanded);
    if let Ok(pub_key) = std::fs::read_to_string(&pub_key_path) {
        println!();
        println!("{}", style("Public key:").bold());
        println!("  {}", style(pub_key.trim()).cyan());
        println!();
    }

    // Offer ssh-copy-id
    let copy_now = Confirm::with_theme(theme)
        .with_prompt(format!(
            "Copy this key to {}@{} now using ssh-copy-id?",
            user, hostname
        ))
        .default(true)
        .interact()
        .context("Failed to read choice")?;

    if copy_now {
        println!(
            "  {} Running ssh-copy-id (you may be prompted for your password)...",
            style("→").cyan()
        );
        let status = Command::new("ssh-copy-id")
            .args(["-i", &pub_key_path, &format!("{}@{}", user, hostname)])
            .status()
            .context("Failed to run ssh-copy-id")?;

        if status.success() {
            println!("  {} Key copied successfully.", style("✓").green());
        } else {
            println!(
                "  {} ssh-copy-id failed. Here are the manual steps:",
                style("⚠").yellow()
            );
            print_manual_copy_steps(&pub_key_path, user, hostname);
        }
    } else {
        println!();
        print_manual_copy_steps(&pub_key_path, user, hostname);
    }

    Ok(key_path)
}

fn print_manual_copy_steps(pub_key_path: &str, user: &str, hostname: &str) {
    println!("{}", style("Manual steps to authorize this key:").bold());
    println!(
        "  1. Run: {}",
        style(format!(
            "ssh-copy-id -i {} {}@{}",
            pub_key_path, user, hostname
        ))
        .cyan()
    );
    println!("     — or —");
    println!(
        "  2. Display the public key:  {}",
        style(format!("cat {}", pub_key_path)).cyan()
    );
    println!("  3. On the remote host, append it to authorized_keys:");
    println!(
        "     {}",
        style("echo '<paste key>' >> ~/.ssh/authorized_keys").cyan()
    );
    println!("  4. Fix permissions on the remote host:");
    println!(
        "     {}",
        style("chmod 700 ~/.ssh && chmod 600 ~/.ssh/authorized_keys").cyan()
    );
    println!();
}

// ── Confirmation rules ─────────────────────────────────────────────────────

fn configure_confirm_rules(theme: &ColorfulTheme, config: &mut WizardConfig) -> Result<()> {
    println!();
    println!(
        "{}",
        style("── Confirmation Rules ─────────────────────────").bold()
    );
    println!("These commands will require human approval before executing.");
    println!();

    if config.confirm.is_none() {
        // No existing rules — show secure defaults and offer to apply them
        let defaults = default_confirm_rules();
        println!(
            "Recommended secure defaults for {}:",
            style("ssh.exec").cyan()
        );
        if let Some(WizardConfirmRule::WhenPattern { when_pattern }) = defaults.get("ssh.exec") {
            for pattern in when_pattern {
                println!(
                    "  {} Commands containing: {}",
                    style("•").yellow(),
                    style(format!("\"{}\"", pattern)).cyan()
                );
            }
        }
        println!();

        let choices = [
            "Use secure defaults (recommended)",
            "Customize confirmation rules",
            "Skip — no confirmation rules",
        ];
        let idx = Select::with_theme(theme)
            .with_prompt("Confirmation rules")
            .items(&choices)
            .default(0)
            .interact()
            .context("Failed to read choice")?;

        match idx {
            0 => {
                config.confirm = Some(defaults);
                println!(
                    "  {} Secure confirmation defaults applied.",
                    style("✓").green()
                );
            }
            1 => {
                config.confirm = Some(HashMap::new());
                add_confirm_rules_interactive(theme, config)?;
            }
            _ => {
                println!(
                    "  {} Skipped — no confirmation rules set.",
                    style("→").yellow()
                );
            }
        }
    } else {
        // Existing rules — show them and offer to customize
        println!("Existing confirmation rules:");
        if let Some(rules) = &config.confirm {
            for (tool, rule) in rules {
                match rule {
                    WizardConfirmRule::Always(_) => {
                        println!("  {} {}: always", style("•").yellow(), style(tool).cyan());
                    }
                    WizardConfirmRule::WhenPattern { when_pattern } => {
                        println!(
                            "  {} {}: when pattern matches {:?}",
                            style("•").yellow(),
                            style(tool).cyan(),
                            when_pattern
                        );
                    }
                }
            }
        }
        println!();

        let customize = Confirm::with_theme(theme)
            .with_prompt("Add more confirmation rules?")
            .default(false)
            .interact()
            .context("Failed to read choice")?;

        if customize {
            add_confirm_rules_interactive(theme, config)?;
        }
    }

    Ok(())
}

fn add_confirm_rules_interactive(theme: &ColorfulTheme, config: &mut WizardConfig) -> Result<()> {
    let rules = config.confirm.get_or_insert_with(HashMap::new);

    println!();
    println!("Available tools: ssh.exec, ssh.upload, ssh.download,");
    println!("                 docker.container.start, docker.container.stop");
    println!("Enter tool names one at a time. Leave empty to finish.");
    println!();

    loop {
        let tool: String = Input::with_theme(theme)
            .with_prompt("Tool name (leave empty to finish)")
            .allow_empty(true)
            .interact_text()
            .context("Failed to read tool name")?;

        if tool.trim().is_empty() {
            break;
        }

        let rule_choices = [
            "when_pattern — require confirmation for specific command patterns",
            "always        — require confirmation for every call",
        ];
        let rule_idx = Select::with_theme(theme)
            .with_prompt("Rule type")
            .items(&rule_choices)
            .default(0)
            .interact()
            .context("Failed to read rule type")?;

        let rule = if rule_idx == 1 {
            WizardConfirmRule::Always("always".to_string())
        } else {
            let patterns_str: String = Input::with_theme(theme)
                .with_prompt("Patterns (comma-separated, e.g. 'rm -rf,reboot,shutdown')")
                .interact_text()
                .context("Failed to read patterns")?;
            let when_pattern = patterns_str
                .split(',')
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty())
                .collect();
            WizardConfirmRule::WhenPattern { when_pattern }
        };

        let tool_name = tool.trim().to_string();
        println!(
            "  {} Rule added for {}.",
            style("✓").green(),
            style(&tool_name).cyan()
        );
        rules.insert(tool_name, rule);
    }

    Ok(())
}

// ── Rate limits ────────────────────────────────────────────────────────────

fn configure_rate_limits(theme: &ColorfulTheme, config: &mut WizardConfig) -> Result<()> {
    println!();
    println!(
        "{}",
        style("── Rate Limits ────────────────────────────────").bold()
    );

    if !config.rate_limits.limits.is_empty() {
        println!("Existing rate limits:");
        for (tool, limit) in &config.rate_limits.limits {
            println!(
                "  {} {}: {} calls/min",
                style("•").cyan(),
                tool,
                limit.per_minute
            );
        }
        println!();

        let customize = Confirm::with_theme(theme)
            .with_prompt("Customize rate limits?")
            .default(false)
            .interact()
            .context("Failed to read choice")?;

        if customize {
            add_rate_limits_interactive(theme, config)?;
        }

        return Ok(());
    }

    // No existing limits — offer default or custom
    println!("A global rate limit controls how many tool calls are allowed per minute.");
    println!();

    let choices = [
        "Use default (30 calls/min for all tools) — recommended",
        "Set a custom limit",
        "Skip — no rate limits",
    ];
    let idx = Select::with_theme(theme)
        .with_prompt("Rate limits")
        .items(&choices)
        .default(0)
        .interact()
        .context("Failed to read choice")?;

    match idx {
        0 => {
            config
                .rate_limits
                .limits
                .insert("*".to_string(), WizardRateLimit { per_minute: 30 });
            println!(
                "  {} Global rate limit set to 30 calls/minute.",
                style("✓").green()
            );
        }
        1 => {
            add_rate_limits_interactive(theme, config)?;
        }
        _ => {
            println!("  {} Skipped — no rate limits set.", style("→").yellow());
        }
    }

    Ok(())
}

fn add_rate_limits_interactive(theme: &ColorfulTheme, config: &mut WizardConfig) -> Result<()> {
    println!();
    println!("Use '*' as the tool name to set a global limit for all tools.");
    println!("Leave the tool name empty to finish.");
    println!();

    loop {
        let tool: String = Input::with_theme(theme)
            .with_prompt("Tool name (or '*' for global, leave empty to finish)")
            .allow_empty(true)
            .interact_text()
            .context("Failed to read tool name")?;

        if tool.trim().is_empty() {
            break;
        }

        let per_min_str: String = Input::with_theme(theme)
            .with_prompt("Calls per minute")
            .default("30".to_string())
            .interact_text()
            .context("Failed to read rate limit")?;

        let per_minute: u32 = per_min_str
            .trim()
            .parse()
            .map_err(|_| anyhow!("Invalid number: '{}'", per_min_str.trim()))?;

        let tool_name = tool.trim().to_string();
        println!(
            "  {} Rate limit: {} → {} calls/min",
            style("✓").green(),
            style(&tool_name).cyan(),
            per_minute
        );
        config
            .rate_limits
            .limits
            .insert(tool_name, WizardRateLimit { per_minute });
    }

    Ok(())
}

// ── Config writer ──────────────────────────────────────────────────────────

fn write_config(path: &Path, config: &WizardConfig) -> Result<()> {
    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create config directory: {}", parent.display()))?;
    }

    let toml_str = toml::to_string_pretty(config).context("Failed to serialize config to TOML")?;

    std::fs::write(path, &toml_str)
        .with_context(|| format!("Failed to write config to: {}", path.display()))?;

    // Restrict permissions to owner read/write only (0600)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(path, perms)
            .context("Failed to set config file permissions to 0600")?;
    }

    Ok(())
}

// ── Summary ────────────────────────────────────────────────────────────────

fn print_summary(config_path: &Path, config: &WizardConfig) {
    let binary = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "spacebot-homelab-mcp".to_string());

    let config_display = config_path.display().to_string();

    println!();
    println!(
        "{}",
        style("╔══════════════════════════════════════════╗")
            .green()
            .bold()
    );
    println!(
        "{}",
        style("║             Setup Complete!              ║")
            .green()
            .bold()
    );
    println!(
        "{}",
        style("╚══════════════════════════════════════════╝")
            .green()
            .bold()
    );
    println!();

    println!("{}", style("What was configured:").bold());
    println!("  Config file  : {}", style(&config_display).cyan());
    println!("  Permissions  : 0600 (owner read/write only)");
    println!("  SSH hosts    : {}", config.ssh.hosts.len());
    if !config.ssh.hosts.is_empty() {
        for (alias, host) in &config.ssh.hosts {
            println!(
                "    {} {} ({}@{})",
                style("•").green(),
                style(alias).bold(),
                host.user,
                host.host
            );
        }
    }
    println!("  Docker hosts : {}", config.docker.hosts.len());
    println!(
        "  Confirm rules: {}",
        config.confirm.as_ref().map(|r| r.len()).unwrap_or(0)
    );
    println!("  Rate limits  : {}", config.rate_limits.limits.len());

    println!();
    println!("{}", style("Next steps:").bold());
    println!();
    println!("  1. Verify connectivity and config with doctor:");
    println!(
        "     {}",
        style(format!("{} doctor --config {}", binary, config_display)).cyan()
    );
    println!();
    println!("  2. Add this server to your Spacebot / Claude Desktop MCP config:");
    println!();
    println!(
        "{}",
        style(format!(
            r#"  {{
    "spacebot-homelab-mcp": {{
      "command": "{}",
      "args": ["server", "--config", "{}"]
    }}
  }}"#,
            binary, config_display
        ))
        .cyan()
    );
    println!();
    println!(
        "  {}",
        style("Tip: run setup again any time to add hosts or adjust settings.").italic()
    );
    println!();
}

// ── Helpers ────────────────────────────────────────────────────────────────

fn expand_home(path: &str) -> String {
    if path == "~" {
        return std::env::var("HOME").unwrap_or_else(|_| "~".to_string());
    }
    if let Some(rest) = path.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{}/{}", home, rest);
        }
    }
    path.to_string()
}
