# spacebot-homelab-mcp

An MCP (Model Context Protocol) server that provides Docker and SSH tools for managing homelab infrastructure via Spacebot.

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
    │   └── SSH connection pool (multiplexed sessions)
    ├── Tool implementations (18 tools)
    │   ├── docker.container.*  (7 tools)
    │   ├── docker.image.*      (5 tools)
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
"ssh.exec" = { per_minute = 10 }

# Confirmation rules for destructive operations
[confirm]
"docker.container.delete" = "always"
"docker.container.stop" = "always"
"docker.image.delete" = "always"
"docker.image.prune" = "always"
"ssh.exec" = { when_pattern = ["rm -rf", "dd if=", "systemctl restart"] }
```

## Usage

### Start the MCP server

```bash
spacebot-homelab-mcp server --config ~/.spacebot-homelab/config.toml
# Runs on stdio, ready for Spacebot to connect
```

### Validate configuration

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

### System tools

| Tool | Description |
|------|-------------|
| `confirm_operation` | Confirm a pending destructive operation (second step of the two-step flow) |
| `audit.verify_operation` | Verify an operation was recorded in the audit log (anti-hallucination) |
| `audit.verify_container_state` | Check live Docker state to verify a container operation result |

### Confirmation flow

Destructive tools use a two-step confirmation flow:

1. Call the destructive tool (e.g. `docker.container.delete`) — returns `confirmation_required` with a UUID token
2. Call `confirm_operation` with the token and tool name — executes the operation

Tokens are single-use, tool-specific, and expire after 5 minutes.

### Dry run support

All destructive tools accept a `dry_run` parameter. Set `dry_run=true` to preview what would happen without executing.

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
10. **Connection validation** — `doctor` subcommand validates all connections at startup
11. **Graceful degradation** — Individual host connections can fail without disabling other tools

## Development

### Project structure

```
src/
├── main.rs              — CLI entry point (server, doctor, setup subcommands)
├── config.rs            — Configuration loading and validation
├── connection.rs        — Connection manager (Docker clients, SSH pool)
├── mcp.rs               — MCP tool handler (18 tools registered)
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
