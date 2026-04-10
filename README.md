# spacebot-homelab-mcp

An MCP (Model Context Protocol) server that provides Docker, SSH, and Proxmox VE tools for managing homelab infrastructure via Spacebot.

This is a standalone Rust binary that runs as an external process. Spacebot connects to it via the MCP protocol and delegates homelab-related tasks to it.

## Architecture

```
Spacebot Worker
    |
MCP Protocol (stdio)
    |
spacebot-homelab-mcp
    ├── Connection Manager
    │   ├── Docker clients (local socket + remote TCP/TLS)
    │   ├── SSH connection pool (multiplexed sessions)
    │   └── Proxmox REST API clients (token auth + self-signed TLS support)
    ├── Tool implementations (32 tools)
    │   ├── docker.container.*  (7 tools)
    │   ├── docker.image.*      (5 tools)
    │   ├── proxmox.*           (14 tools)
    │   ├── ssh.*               (3 tools)
    │   ├── confirm_operation   (confirmation flow)
    │   └── audit.*             (2 verification tools)
    ├── Security layers
    │   ├── Rate limiting (per-tool, global or per-caller)
    │   ├── Confirmation flow (two-step for destructive ops)
    │   ├── Anti-hallucination (execution proof envelopes)
    │   └── Tool enable/disable (config-driven)
    └── Audit logging (append-only)
```

## Installation

### Install script (recommended)

Download and install the pre-built binary:

```bash
curl -fsSL https://raw.githubusercontent.com/Joshf225/spacebot-homelab-mcp/master/install.sh | bash
```

You can set a specific version and install directory:

```bash
VERSION=0.2.2 INSTALL_DIR=~/.local/bin curl -fsSL https://raw.githubusercontent.com/Joshf225/spacebot-homelab-mcp/master/install.sh | bash
```

### GitHub Releases

Download pre-built binaries for your platform from [GitHub Releases](https://github.com/Joshf225/spacebot-homelab-mcp/releases).

Available targets:
- `x86_64-unknown-linux-gnu` (Linux x64)
- `aarch64-unknown-linux-gnu` (Linux ARM64)
- `x86_64-apple-darwin` (macOS Intel)
- `aarch64-apple-darwin` (macOS Apple Silicon)

### Build from source

Requires [Rust](https://rustup.rs/) 1.85+:

```bash
cargo build --release
# Binary: target/release/spacebot-homelab-mcp
```

## Configuration

Create `~/.spacebot-homelab/config.toml` (see `example.config.toml` for all options):

```toml
# Docker hosts
[docker.hosts.local]
host = "unix:///var/run/docker.sock"

[docker.hosts.vps]
host = "tcp://vps.example.com:2375"

# Proxmox VE hosts
[proxmox.hosts.pve1]
url = "https://192.168.1.10:8006"
token_id = "root@pam!spacebot"
token_secret = "${PVE_TOKEN_SECRET}"
node = "pve1"
verify_tls = false
task_wait_timeout_secs = 600
next_vmid_retry_attempts = 3
next_vmid_retry_backoff_ms = 250

# SSH hosts
[ssh.hosts.home]
host = "192.168.1.1"
user = "homelab-agent"
private_key_path = "~/.ssh/homelab-home"

[ssh.hosts.nas]
host = "192.168.1.50"
user = "homelab-agent"
private_key_path = "~/.ssh/homelab-nas"

# Audit logging
[audit]
file = "/var/log/spacebot-homelab/audit.log"

# SSH command validation
[ssh.command_allowlist]
allowed_prefixes = ["docker", "zpool", "zfs", "df", "sudo"]
blocked_patterns = ["rm -rf", "dd if=", "mkfs", "> /dev/"]

# Rate limiting
[rate_limits.limits]
"docker.container.*" = { per_minute = 5 }
"docker.container.delete" = { per_minute = 1 }
"docker.image.*" = { per_minute = 5 }
"docker.image.delete" = { per_minute = 1 }
"docker.image.prune" = { per_minute = 1 }
"proxmox.*" = { per_minute = 15 }
"ssh.exec" = { per_minute = 10 }

# Confirmation rules for destructive operations
# NOTE: These rules require a client that supports the two-step MCP
# `confirm_operation` flow. If your Spacebot build/UI does not surface
# pending confirmations, leave this section disabled and rely on `dry_run`
# until your client adds confirmation support.
[confirm]
"docker.container.delete" = "always"
"docker.container.stop" = "always"
"docker.image.delete" = "always"
"docker.image.prune" = "always"
"proxmox.vm.stop" = "always"
"proxmox.vm.create" = "always"
"proxmox.vm.delete" = "always"
"proxmox.vm.snapshot.rollback" = "always"
"ssh.exec" = { when_pattern = ["rm -rf", "dd if=", "systemctl restart"] }
```

## Usage

### Command overview

The binary has three main subcommands:

| Command | Purpose |
|---------|---------|
| `server` | Start the MCP server over stdio for Spacebot |
| `doctor` | Validate config, check Docker/SSH/Proxmox connectivity, and print a readiness summary |
| `setup` | Launch the interactive configuration wizard |

### Start the MCP server

```bash
spacebot-homelab-mcp server --config ~/.spacebot-homelab/config.toml
# Runs on stdio, ready for Spacebot to connect
```

### Validate configuration

Use `doctor` before connecting Spacebot or after changing `config.toml`. It validates configuration, checks Docker, SSH, and Proxmox connectivity, and prints a summary of security settings and rate limits.

```bash
spacebot-homelab-mcp doctor --config ~/.spacebot-homelab/config.toml
```

### Interactive setup wizard

```bash
spacebot-homelab-mcp setup
```

### Configure Spacebot

Add to your Spacebot `config.toml`:

```toml
[[mcp_servers]]
name = "homelab"
transport = "stdio"
command = "spacebot-homelab-mcp"
args = ["server", "--config", "~/.spacebot-homelab/config.toml"]
```

## Available Tools

### Docker container tools

| Tool | Description | Destructive | Confirmation |
|------|-------------|:-----------:|:------------:|
| `docker.container.list` | List containers (with optional name filter) | No | — |
| `docker.container.start` | Start a stopped container | No | — |
| `docker.container.stop` | Stop a running container | Yes | Yes |
| `docker.container.logs` | Get container logs (with tail/since) | No | — |
| `docker.container.inspect` | Inspect container details (env vars redacted) | No | — |
| `docker.container.delete` | Delete a container (pre-flight checks for running state & volumes) | Yes | Yes |
| `docker.container.create` | Create a new container (ports, env, volumes, restart policy) | No | — |

### Docker image tools

| Tool | Description | Destructive | Confirmation |
|------|-------------|:-----------:|:------------:|
| `docker.image.list` | List images (with optional name filter) | No | — |
| `docker.image.pull` | Pull an image from a registry | No | — |
| `docker.image.inspect` | Inspect image metadata | No | — |
| `docker.image.delete` | Delete an image | Yes | Yes |
| `docker.image.prune` | Remove unused/dangling images | Yes | Yes |

### SSH tools

| Tool | Description | Destructive | Confirmation |
|------|-------------|:-----------:|:------------:|
| `ssh.exec` | Execute a command on a remote host | Yes | Pattern-based |
| `ssh.upload` | Upload a file via SFTP | Yes | — |
| `ssh.download` | Download a file via SFTP | No | — |

### Proxmox node and VM inventory tools

| Tool | Description | Destructive | Confirmation |
|------|-------------|:-----------:|:------------:|
| `proxmox.node.list` | List Proxmox cluster nodes with CPU, memory, uptime, and version | No | — |
| `proxmox.node.status` | Get detailed node status (CPU, RAM, storage, kernel, uptime) | No | — |
| `proxmox.vm.list` | List QEMU VMs and LXC containers on a node | No | — |
| `proxmox.vm.status` | Get detailed status for a VM or container | No | — |

### Proxmox VM lifecycle tools

| Tool | Description | Destructive | Confirmation |
|------|-------------|:-----------:|:------------:|
| `proxmox.vm.start` | Start a Proxmox VM or LXC container | No | — |
| `proxmox.vm.stop` | Force-stop a Proxmox VM or LXC container immediately | Yes | Yes |
| `proxmox.vm.create` | Create a new Proxmox VM or LXC container | Yes | Yes |
| `proxmox.vm.clone` | Clone an existing Proxmox VM | No | — |
| `proxmox.vm.delete` | Permanently delete a Proxmox VM or LXC container | Yes | Yes |

### Proxmox snapshot and infrastructure tools

| Tool | Description | Destructive | Confirmation |
|------|-------------|:-----------:|:------------:|
| `proxmox.vm.snapshot.list` | List snapshots for a VM or LXC container | No | — |
| `proxmox.vm.snapshot.create` | Create a VM or container snapshot | No | — |
| `proxmox.vm.snapshot.rollback` | Roll back a VM or container to a snapshot | Yes | Yes |
| `proxmox.storage.list` | List Proxmox storage pools and usage | No | — |
| `proxmox.network.list` | List Proxmox network interfaces, bridges, VLANs, and bonds | No | — |

### System tools

| Tool | Description |
|------|-------------|
| `confirm_operation` | Confirm a pending destructive operation (second step of the two-step flow) |
| `audit.verify_operation` | Verify an operation was recorded in the audit log (anti-hallucination) |
| `audit.verify_container_state` | Check live Docker state to verify a container operation result |

### Confirmation flow

Destructive tools use a two-step confirmation flow:

1. Call the destructive tool (e.g. `docker.container.delete` or `proxmox.vm.delete`) — returns `confirmation_required` with a UUID token
2. Call `confirm_operation` with the token and tool name — executes the operation

Tokens are single-use, tool-specific, and expire after 5 minutes.

### Client compatibility

The `[confirm]` rules above only work when the MCP client can complete the second step by calling `confirm_operation`.

If your Spacebot build or UI does not surface pending MCP confirmations yet, destructive tools protected by `[confirm]` will appear to stall because the user cannot complete the confirmation step.

In that case:

1. Leave the `[confirm]` section disabled for homelab MCP tools.
2. Use `dry_run=true` before destructive operations.
3. Re-enable `[confirm]` once your client supports MCP confirmation UX.

### Dry run support

Mutating Docker tools, mutating Proxmox tools, and `ssh.exec` support `dry_run`. Set `dry_run=true` to preview what would happen without executing.

## Security

### Layered defense model

1. **Tool enable/disable** — Only expose tools you need via `[tools] enabled = [...]`
2. **Rate limiting** — Per-tool rate limits with glob pattern support
3. **Dry run** — Preview destructive operations before executing
4. **SSH command allowlist** — Validate commands against configurable prefix/pattern rules
5. **Confirmation flow** — Two-step UUID token flow for destructive operations
6. **Anti-hallucination defenses** — Every tool response includes an execution proof envelope (`server_nonce`, `server_version`, `executed_at`) that only the real server can produce
7. **Audit verification tools** — `audit.verify_operation` and `audit.verify_container_state` let callers confirm operations actually happened
8. **Audit logging** — Append-only log of all tool invocations, outside the agent data directory
9. **Env var redaction** — Container inspect redacts environment variable values
10. **Connection validation** — `doctor` validates Docker, SSH, and Proxmox connections
11. **Graceful degradation** — Individual host connections can fail without disabling other tools

## Development

### Project structure

```
src/
├── main.rs              — CLI entry point (server, doctor, setup subcommands)
├── config.rs            — Configuration loading and validation
├── connection.rs        — Connection manager (Docker clients, SSH pool, Proxmox clients)
├── mcp.rs               — MCP tool handler (32 tools registered)
├── audit.rs             — Audit logging
├── confirmation.rs      — Two-step confirmation manager
├── rate_limit.rs        — Per-tool rate limiting
├── metrics.rs           — Prometheus metrics (optional)
├── notifications.rs     — Desktop notifications
├── health.rs            — Health checks and doctor command
├── setup.rs             — Interactive setup wizard
└── tools/
    ├── mod.rs           — Output envelope wrapping, truncation
    ├── docker.rs        — Docker container tool implementations
    ├── docker_image.rs  — Docker image tool implementations
    ├── proxmox.rs       — Proxmox VE tool implementations
    ├── ssh.rs           — SSH tool implementations
    └── verify.rs        — Audit verification tool implementations
tests/
├── mcp_server.rs        — Integration tests
├── docker_integration.rs
├── ssh_integration.rs
└── mcp_e2e_test.py      — E2E test harness (69 tests across 8 phases)
```

### Building

```bash
cargo build
cargo build --release
```

### Testing

```bash
cargo test
# E2E tests (requires a Docker host configured in config.toml)
python3 tests/mcp_e2e_test.py
```

### Linting

```bash
cargo fmt
cargo clippy
```

## License

MIT
