# spacebot-homelab-mcp

An MCP (Model Context Protocol) server that provides Docker and SSH tools for managing homelab infrastructure via Spacebot.

This is a standalone Rust binary that runs as an external process. Spacebot connects to it via the MCP protocol and delegates homelab-related tasks to it.

## Architecture

```
Spacebot Worker
    ↓
MCP Protocol (stdio)
    ↓
spacebot-homelab-mcp
    ├── Connection Manager
    │   ├── Docker clients
    │   └── SSH connection pool
    ├── Tool implementations
    │   ├── docker.container.*
    │   └── ssh.*
    └── Audit logging
```

## Installation

### Install script (recommended)

Download and install the pre-built binary:

```bash
curl -fsSL https://raw.githubusercontent.com/Joshf225/spacebot-homelab-mcp/master/install.sh | bash
```

You can set a specific version and install directory:

```bash
VERSION=0.1.0 INSTALL_DIR=~/.local/bin curl -fsSL https://raw.githubusercontent.com/Joshf225/spacebot-homelab-mcp/master/install.sh | bash
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

Create `~/.spacebot-homelab/config.toml`:

```toml
[docker.hosts.local]
host = "unix:///var/run/docker.sock"

[docker.hosts.vps]
host = "tcp://vps.example.com:2375"

[ssh.hosts.home]
host = "192.168.1.1"
user = "homelab-agent"
private_key_path = "~/.ssh/homelab-home"

[ssh.hosts.nas]
host = "192.168.1.50"
user = "homelab-agent"
private_key_path = "~/.ssh/homelab-nas"

[audit]
file = "/var/log/spacebot-homelab/audit.log"

[ssh.command_allowlist]
allowed_prefixes = ["docker", "zpool", "zfs", "df", "sudo"]
blocked_patterns = ["rm -rf", "dd if=", "mkfs", "> /dev/"]
```

## Usage

### Start the MCP server

```bash
spacebot-homelab-mcp --config ~/.spacebot-homelab/config.toml
# Runs on stdio, ready for Spacebot to connect
```

### Validate configuration

```bash
spacebot-homelab-mcp doctor --config ~/.spacebot-homelab/config.toml
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

### Docker tools

- `docker.container.list` — List containers
- `docker.container.start` — Start a container
- `docker.container.stop` — Stop a container
- `docker.container.logs` — Get logs
- `docker.container.inspect` — Inspect container details

### SSH tools

- `ssh.exec` — Execute a command on a remote host
- `ssh.upload` — Upload a file to a remote host
- `ssh.download` — Download a file from a remote host

## Security

See `../spacebot/homelab-integration/security-approach.md` for the full security model.

Key points:

- **SSH least-privilege:** Use restricted users on target hosts, not root
- **Command allowlist:** SSH commands are validated against a configurable allowlist
- **Audit logging:** All tool invocations are logged (append-only, outside agent data directory)
- **Connection validation:** `doctor` subcommand validates all connections at startup
- **Graceful degradation:** Individual connections can fail without disabling all tools

## Development

### Structure

- `src/main.rs` — CLI entry point
- `src/config.rs` — Configuration loading and validation
- `src/connection.rs` — Connection manager (SSH pool, Docker clients)
- `src/audit.rs` — Audit logging
- `src/health.rs` — Health checks and doctor command
- `src/tools/docker.rs` — Docker tool implementations
- `src/tools/ssh.rs` — SSH tool implementations
- `tests/` — Integration tests

### Building

```bash
cargo build
cargo build --release
```

### Testing

```bash
cargo test
# Integration tests require Docker and SSH server (testcontainers)
cargo test --test '*' -- --nocapture
```

### Linting

```bash
cargo fmt
cargo clippy
```

## Implementation Status

This is a scaffolded project. See `../spacebot/homelab-integration/poc-specification.md` for:

- Detailed tool schemas
- Implementation milestones
- Test plan
- Estimated effort (7-11 engineering days)

## License

MIT
