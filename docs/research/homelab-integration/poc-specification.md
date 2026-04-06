# PoC Specification: Docker + SSH MCP Tools

**Goal:** Build the minimum viable `spacebot-homelab-mcp` binary that proves the architecture works end-to-end. Two tool domains: Docker container operations and SSH command execution.

**Success criteria:** A Spacebot worker can list Docker containers and execute SSH commands on a remote host via MCP, with connection pooling, audit logging, and basic safety gates.

---

## Binary Structure

```
spacebot-homelab-mcp/
├── Cargo.toml
├── src/
│   ├── main.rs              # CLI entry, config loading, MCP server setup
│   ├── config.rs            # Config parsing and validation
│   ├── connection.rs        # ConnectionManager (see connection-manager.md)
│   ├── audit.rs             # Audit logging
│   ├── tools/
│   │   ├── mod.rs           # Tool registration
│   │   ├── docker.rs        # Docker container tools
│   │   └── ssh.rs           # SSH tools
│   └── health.rs            # doctor subcommand, health monitor
└── tests/
    ├── docker_integration.rs
    └── ssh_integration.rs
```

### Dependencies

```toml
[dependencies]
rmcp = { version = "0.1", features = ["server"] }  # MCP server
bollard = "0.18"                                     # Docker API
russh = "0.46"                                       # SSH client
russh-keys = "0.46"                                  # SSH key loading
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
toml = "0.8"
tracing = "0.1"
tracing-subscriber = "0.3"
dashmap = "6"
clap = { version = "4", features = ["derive"] }
```

### CLI

```
spacebot-homelab-mcp [--config <path>]       # Start MCP server (stdio transport)
spacebot-homelab-mcp doctor [--config <path>] # Validate connections
```

The MCP server communicates over stdio (stdin/stdout) — this is how Spacebot spawns and connects to MCP servers. No TCP listener needed for V1.

---

## Tool Definitions

### docker.container.list

**Purpose:** List running (or all) containers on a configured Docker host.

**MCP Schema:**
```json
{
    "name": "docker.container.list",
    "description": "List Docker containers. Returns container ID, name, image, status, and ports. By default shows only running containers. Set all=true to include stopped containers.",
    "inputSchema": {
        "type": "object",
        "properties": {
            "host": {
                "type": "string",
                "description": "Docker host name from config. Defaults to 'local'.",
                "default": "local"
            },
            "all": {
                "type": "boolean",
                "description": "Include stopped containers. Default: false.",
                "default": false
            },
            "name_filter": {
                "type": "string",
                "description": "Filter containers by name substring. Optional."
            }
        }
    }
}
```

**Implementation:**
```rust
async fn docker_container_list(
    manager: &ConnectionManager,
    host: Option<String>,
    all: Option<bool>,
    name_filter: Option<String>,
) -> Result<String> {
    let host = host.unwrap_or_else(|| "local".into());
    let docker = manager.docker(&host)?;

    let mut filters = HashMap::new();
    if let Some(name) = &name_filter {
        filters.insert("name", vec![name.as_str()]);
    }

    let options = ListContainersOptions {
        all: all.unwrap_or(false),
        filters,
        ..Default::default()
    };

    let containers = docker.client.list_containers(Some(options)).await?;

    // Format as a readable table
    let output = containers.iter().map(|c| {
        format!(
            "{} | {} | {} | {} | {}",
            &c.id.as_deref().unwrap_or("?")[..12],
            c.names.as_ref().map(|n| n.join(", ")).unwrap_or_default(),
            c.image.as_deref().unwrap_or("?"),
            c.status.as_deref().unwrap_or("?"),
            format_ports(&c.ports),
        )
    }).collect::<Vec<_>>().join("\n");

    audit_log("docker.container.list", &host, "success").await;
    Ok(output)
}
```

**Output example:**
```
CONTAINER ID | NAME         | IMAGE              | STATUS        | PORTS
a1b2c3d4e5f6 | /jellyfin    | jellyfin/jellyfin  | Up 3 days     | 8096->8096/tcp
b2c3d4e5f6a7 | /pihole      | pihole/pihole      | Up 5 days     | 53->53/tcp, 80->80/tcp
c3d4e5f6a7b8 | /homebridge  | homebridge/hb      | Exited (0) 2h|
```

---

### docker.container.start

**Purpose:** Start a stopped container.

**MCP Schema:**
```json
{
    "name": "docker.container.start",
    "description": "Start a stopped Docker container. Use docker.container.list to find the container ID or name first.",
    "inputSchema": {
        "type": "object",
        "properties": {
            "host": {
                "type": "string",
                "description": "Docker host name from config. Defaults to 'local'.",
                "default": "local"
            },
            "container": {
                "type": "string",
                "description": "Container ID or name."
            }
        },
        "required": ["container"]
    }
}
```

**Implementation:** Call `docker.start_container()`. Verify the container exists first. Return new status after start. Audit log the action.

---

### docker.container.stop

**Purpose:** Stop a running container.

**MCP Schema:**
```json
{
    "name": "docker.container.stop",
    "description": "Stop a running Docker container. Uses a 10-second graceful shutdown timeout before SIGKILL.",
    "inputSchema": {
        "type": "object",
        "properties": {
            "host": {
                "type": "string",
                "description": "Docker host name from config. Defaults to 'local'.",
                "default": "local"
            },
            "container": {
                "type": "string",
                "description": "Container ID or name."
            },
            "timeout": {
                "type": "integer",
                "description": "Seconds to wait before SIGKILL. Default: 10.",
                "default": 10
            }
        },
        "required": ["container"]
    }
}
```

---

### docker.container.logs

**Purpose:** Retrieve recent logs from a container.

**MCP Schema:**
```json
{
    "name": "docker.container.logs",
    "description": "Get recent logs from a Docker container. Returns the last N lines (default 100). Output is truncated to 10,000 characters to prevent context overflow.",
    "inputSchema": {
        "type": "object",
        "properties": {
            "host": {
                "type": "string",
                "description": "Docker host name from config. Defaults to 'local'.",
                "default": "local"
            },
            "container": {
                "type": "string",
                "description": "Container ID or name."
            },
            "tail": {
                "type": "integer",
                "description": "Number of lines from the end. Default: 100.",
                "default": 100
            },
            "since": {
                "type": "string",
                "description": "Show logs since this timestamp or duration (e.g., '1h', '2024-01-01T00:00:00Z'). Optional."
            }
        },
        "required": ["container"]
    }
}
```

**Output truncation:** The raw log output is capped at 10,000 characters. If truncated, prepend: `"[Output truncated. Showing last 10,000 chars of {total} chars. Use 'since' or reduce 'tail' for more specific results.]\n"`.

**Security note:** Container logs are untrusted data — they may contain attacker-controlled content. The MCP response wraps output in a structured envelope per Layer 9 of `security-approach.md`.

---

### docker.container.inspect

**Purpose:** Get detailed information about a container (config, network, volumes, state).

**MCP Schema:**
```json
{
    "name": "docker.container.inspect",
    "description": "Get detailed information about a Docker container including its configuration, network settings, volume mounts, and current state. Useful for debugging.",
    "inputSchema": {
        "type": "object",
        "properties": {
            "host": {
                "type": "string",
                "description": "Docker host name from config. Defaults to 'local'.",
                "default": "local"
            },
            "container": {
                "type": "string",
                "description": "Container ID or name."
            }
        },
        "required": ["container"]
    }
}
```

**Output:** Formatted summary (not raw JSON dump). Key fields: image, created, status, restart policy, ports, volumes, environment variables (with values redacted — show keys only), network mode, IP addresses.

---

### ssh.exec

**Purpose:** Execute a command on a remote host via SSH.

**MCP Schema:**
```json
{
    "name": "ssh.exec",
    "description": "Execute a command on a remote host via SSH. Commands are validated against the configured allowlist. Use 'sudo' prefix for commands that require elevated privileges (the SSH user must have specific sudo permissions configured on the host). Always prefer dry_run=true first for commands you haven't run before.",
    "inputSchema": {
        "type": "object",
        "properties": {
            "host": {
                "type": "string",
                "description": "SSH host name from config (e.g., 'home', 'proxmox', 'nas')."
            },
            "command": {
                "type": "string",
                "description": "The command to execute. Must match an allowed prefix from config."
            },
            "timeout": {
                "type": "integer",
                "description": "Command timeout in seconds. Default: 30. Max: 300.",
                "default": 30
            },
            "dry_run": {
                "type": "boolean",
                "description": "If true, validates the command against the allowlist but does not execute it. Default: false.",
                "default": false
            }
        },
        "required": ["host", "command"]
    }
}
```

**Implementation:**
```rust
async fn ssh_exec(
    manager: &ConnectionManager,
    host: String,
    command: String,
    timeout: Option<u64>,
    dry_run: Option<bool>,
) -> Result<String> {
    let timeout = Duration::from_secs(timeout.unwrap_or(30).min(300));

    // 1. Validate command against allowlist
    validate_command(&command, &manager.config.ssh.command_allowlist)?;

    // 2. Check for blocked patterns
    check_blocked_patterns(&command, &manager.config.ssh.blocked_patterns)?;

    if dry_run.unwrap_or(false) {
        audit_log("ssh.exec", &host, "dry_run").await;
        return Ok(format!(
            "DRY RUN: Command '{}' passes validation for host '{}'. \
             Set dry_run=false to execute.",
            command, host
        ));
    }

    // 3. Check out an SSH session from the pool
    let session = manager.ssh_checkout(&host).await?;

    // 4. Execute with timeout
    let result = tokio::time::timeout(timeout, async {
        let mut channel = session.channel_open_session().await?;
        channel.exec(true, &command).await?;

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut exit_status = None;

        // Collect output...
        // (simplified — actual impl handles channel messages)

        Ok::<_, anyhow::Error>((stdout, stderr, exit_status))
    }).await;

    // 5. Return session to pool
    manager.ssh_return(&host, session).await;

    // 6. Format result
    match result {
        Ok(Ok((stdout, stderr, status))) => {
            let output = format_ssh_output(&stdout, &stderr, status);
            // Truncate to 5000 chars
            let output = truncate_output(output, 5000);
            audit_log("ssh.exec", &host, "success").await;
            Ok(output)
        }
        Ok(Err(e)) => {
            audit_log("ssh.exec", &host, &format!("error: {}", e)).await;
            Err(e)
        }
        Err(_) => {
            audit_log("ssh.exec", &host, "timeout").await;
            Err(anyhow!("Command timed out after {}s on host '{}'", timeout.as_secs(), host))
        }
    }
}
```

**Command validation:**
```rust
fn validate_command(command: &str, allowlist: &CommandAllowlist) -> Result<()> {
    // Check blocked patterns first (higher priority)
    for pattern in &allowlist.blocked_patterns {
        if command.contains(pattern) {
            return Err(anyhow!(
                "Command blocked: contains dangerous pattern '{}'. \
                 This pattern is in the blocked list for safety.",
                pattern
            ));
        }
    }

    // Check allowed prefixes
    let allowed = allowlist.allowed_prefixes.iter().any(|prefix| {
        command.starts_with(prefix)
    });

    if !allowed {
        return Err(anyhow!(
            "Command '{}' does not match any allowed prefix. \
             Allowed: {:?}",
            command,
            allowlist.allowed_prefixes
        ));
    }

    Ok(())
}
```

---

### ssh.upload

**Purpose:** Upload a file to a remote host via SCP/SFTP.

**MCP Schema:**
```json
{
    "name": "ssh.upload",
    "description": "Upload a file to a remote host via SFTP. The local file must exist. The remote directory must exist. Maximum file size: 50MB.",
    "inputSchema": {
        "type": "object",
        "properties": {
            "host": {
                "type": "string",
                "description": "SSH host name from config."
            },
            "local_path": {
                "type": "string",
                "description": "Path to the local file to upload."
            },
            "remote_path": {
                "type": "string",
                "description": "Destination path on the remote host."
            }
        },
        "required": ["host", "local_path", "remote_path"]
    }
}
```

**Limits:** 50MB max file size. Remote path validated against a configurable allowed-paths list (prevent writing to `/etc`, `/usr`, etc.).

---

### ssh.download

**Purpose:** Download a file from a remote host.

**MCP Schema:**
```json
{
    "name": "ssh.download",
    "description": "Download a file from a remote host via SFTP. Maximum file size: 50MB. Returns the local path where the file was saved.",
    "inputSchema": {
        "type": "object",
        "properties": {
            "host": {
                "type": "string",
                "description": "SSH host name from config."
            },
            "remote_path": {
                "type": "string",
                "description": "Path to the file on the remote host."
            },
            "local_path": {
                "type": "string",
                "description": "Where to save the downloaded file locally. Optional — defaults to a temp directory.",
                "default": null
            }
        },
        "required": ["host", "remote_path"]
    }
}
```

---

## Error Handling

All tool errors are returned as MCP tool results, not protocol errors. This lets the LLM see the error and decide what to do.

**Error format:**
```json
{
    "content": [
        {
            "type": "text",
            "text": "Error: SSH host 'nas' is unreachable. Status: Disconnected. Last error: connection refused. Last successful connection: 12 minutes ago."
        }
    ],
    "isError": true
}
```

**Error categories and LLM guidance:**

| Error | LLM Should... |
|-------|---------------|
| Host unreachable | Inform user, suggest checking if host is online |
| Auth failed | Inform user, do not retry (creds are wrong) |
| Command blocked | Explain what was blocked and why |
| Command timeout | Inform user, suggest shorter timeout or simpler command |
| Rate limited | Wait and retry, or inform user |
| Container not found | List containers to find the right one |
| Permission denied | Inform user, suggest checking sudoers config |

---

## Test Plan

### Unit tests (no external dependencies)

| Test | What it validates |
|------|------------------|
| Command allowlist: allowed command passes | `docker ps` with `docker` prefix → OK |
| Command allowlist: disallowed command fails | `apt install` with no matching prefix → Error |
| Command allowlist: blocked pattern caught | `docker exec foo ; rm -rf /` → Blocked |
| Command allowlist: prefix match is strict | `dockerrm` does not match `docker` prefix (needs space/EOL) |
| Output truncation at limit | 20,000 char output → truncated to limit with notice |
| Config parsing: valid config | Round-trip parse and validate |
| Config parsing: missing required fields | Returns clear error |
| Health status transitions | Connected → Disconnected → Connected |

### Integration tests (require Docker / SSH)

| Test | What it validates |
|------|------------------|
| `docker.container.list` against local Docker | Returns running containers |
| `docker.container.start/stop` lifecycle | Stop a test container, verify stopped, start it, verify running |
| `docker.container.logs` with tail limit | Returns correct number of lines |
| `docker.container.inspect` format | Returns structured info, env values redacted |
| `ssh.exec` against test SSH container | Execute `echo hello`, verify output |
| `ssh.exec` timeout | Long-running command killed after timeout |
| `ssh.exec` concurrent calls | Two simultaneous commands on same host succeed (pool works) |
| `ssh.upload` / `ssh.download` round trip | Upload file, download it, compare |
| Connection failure mid-session | Stop SSH container, verify error message, restart, verify recovery |
| `doctor` subcommand | Reports correct status for reachable and unreachable hosts |

### Integration test infrastructure

Use `testcontainers` crate to spin up:
- An OpenSSH server container with a known keypair
- A Docker-in-Docker container (or mock the Docker API)

```rust
#[tokio::test]
async fn test_ssh_exec_basic() {
    let ssh_server = testcontainers::GenericImage::new("linuxserver/openssh-server", "latest")
        .with_env_var("PASSWORD_ACCESS", "true")
        .with_env_var("USER_PASSWORD", "test")
        .with_env_var("USER_NAME", "testuser")
        .start()
        .await;

    let port = ssh_server.get_host_port(2222).await;

    let config = Config {
        ssh: SshConfig {
            hosts: HashMap::from([(
                "test".into(),
                SshHostConfig {
                    host: "127.0.0.1".into(),
                    port,
                    user: "testuser".into(),
                    // ... auth config
                },
            )]),
            // ...
        },
        // ...
    };

    let manager = ConnectionManager::new(config).await.unwrap();
    let result = ssh_exec(&manager, "test".into(), "echo hello".into(), None, None).await;
    assert!(result.is_ok());
    assert!(result.unwrap().contains("hello"));
}
```

---

## Milestone Checklist

### M1: Binary boots and serves MCP (1-2 days)
- [ ] `main.rs`: CLI parsing, config loading, MCP server on stdio
- [ ] `config.rs`: Parse and validate `~/.spacebot-homelab/config.toml`
- [ ] Register empty tool list with `rmcp` server
- [ ] Verify Spacebot connects and discovers the MCP server

### M2: Docker tools work (2-3 days)
- [ ] `ConnectionManager` with Docker client initialization
- [ ] `docker.container.list` — query and format output
- [ ] `docker.container.start` / `docker.container.stop`
- [ ] `docker.container.logs` with truncation
- [ ] `docker.container.inspect` with formatted output
- [ ] Integration tests against local Docker

### M3: SSH tools work (2-3 days)
- [ ] SSH connection pool (checkout, return, validation)
- [ ] `ssh.exec` with command validation, timeout, output truncation
- [ ] `ssh.upload` / `ssh.download` via SFTP
- [ ] Integration tests with testcontainers SSH server

### M4: Safety and observability (1-2 days)
- [ ] Audit logging (append-only file)
- [ ] Command allowlist / blocklist enforcement
- [ ] Rate limiting
- [ ] `doctor` subcommand
- [ ] Health monitor background task

### M5: End-to-end validation (1 day)
- [ ] Configure Spacebot with `[[agents.mcp]]` pointing to the binary
- [ ] Send a message to the homelab agent: "List my Docker containers"
- [ ] Verify: channel → branch → worker → MCP → Docker API → response
- [ ] Send: "SSH into my NAS and check disk usage"
- [ ] Verify: full flow with SSH pool

**Total estimated effort: 7-11 engineering days for a working PoC.**
