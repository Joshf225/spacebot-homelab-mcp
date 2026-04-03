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

Add to `~/.spacebot/config.toml`:

```toml
[[agents]]
id = "homelab"

[[agents.mcp]]
name = "homelab"
command = "spacebot-homelab-mcp"
args = ["--config", "~/.spacebot-homelab/config.toml"]
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

Same as Spacebot (TBD)
