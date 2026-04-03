# Complete Implementation Guide: M1-M5

This document provides a comprehensive overview of the complete `spacebot-homelab-mcp` implementation across all five milestones.

## Overview

The `spacebot-homelab-mcp` project is a standalone Rust MCP (Model Context Protocol) server that provides Spacebot with tools to manage Docker containers and execute SSH commands on homelab infrastructure.

**Total implementation:** 7-11 engineering days across 5 milestones
**Target:** Fully functional PoC by end of M5

## Files Overview

| File | Purpose |
|------|---------|
| `M1-Implementation.md` | **Binary boots and serves MCP** — MCP server setup, config loading, empty tool registration |
| `M2-M5-Implementation.md` | **Docker tools, SSH tools, safety, end-to-end validation** — Full tool implementation and integration |
| `poc-specification.md` | Reference document with tool schemas, test plans, and requirements (in `spacebot/homelab-integration/`) |

## Milestone Summary

### M1: Binary Boots and Serves MCP (1-2 days)

**Goal:** Make the binary boot successfully and serve an MCP server over stdio with an empty tool list.

**Deliverables:**
- MCP server listens on stdio
- Spacebot can spawn and connect to the binary
- Tool registration framework is in place
- Empty tools/list response works

**Key Files:**
- `src/mcp.rs` — MCP server handler
- `src/main.rs` — CLI parsing, server startup
- `Cargo.toml` — rmcp crate dependency

**See:** `M1-Implementation.md` (detailed step-by-step plan)

---

### M2: Docker Tools Work (2-3 days)

**Goal:** Implement all 5 Docker container management tools with full Docker API integration.

**Deliverables:**
- `docker.container.list` — list containers with filtering
- `docker.container.start` — start stopped container
- `docker.container.stop` — gracefully stop container
- `docker.container.logs` — retrieve logs with truncation
- `docker.container.inspect` — detailed container metadata

**Key Files:**
- `src/tools/docker.rs` — all 5 tool handlers
- `src/connection.rs` — DockerClient initialization
- `src/mcp.rs` — tool registration

**Key Concepts:**
- Bollard Docker API client
- Output formatting for LLM readability
- Connection health tracking
- Audit logging on each call

**See:** `M2-M5-Implementation.md` → "Milestone 2: Docker Tools Work"

---

### M3: SSH Tools Work (2-3 days)

**Goal:** Implement SSH command execution and file transfer with connection pooling.

**Deliverables:**
- `ssh.exec` — execute commands with validation and timeout
- `ssh.upload` — file upload via SFTP
- `ssh.download` — file download via SFTP
- SSH connection pool management

**Key Files:**
- `src/tools/ssh.rs` — all 3 tool handlers
- `src/connection.rs` — SshPool and SshSession
- `src/mcp.rs` — tool registration

**Key Concepts:**
- russh SSH client library
- Command validation (allowlist/blocklist)
- Output truncation
- Timeout enforcement
- File size limits (50MB)

**See:** `M2-M5-Implementation.md` → "Milestone 3: SSH Tools Work"

---

### M4: Safety and Observability (1-2 days)

**Goal:** Implement security gates, rate limiting, confirmation flow, health monitoring, graceful shutdown, and observability.

**Deliverables:**
- Append-only audit logging
- Rate limiting per tool with wildcard pattern support (e.g., `"docker.container.*"`)
- Layer 8: Token-based confirmation flow for destructive operations
- Command allowlist/blocklist enforcement
- Doctor subcommand with health diagnostics
- Health monitor background task (30s interval)
- Reconnection backoff (30s → 60s → 120s → 180s capped)
- Graceful shutdown (SIGTERM/SIGINT with 10s drain)

**Key Files:**
- `src/audit.rs` — audit logging (mostly done)
- `src/rate_limit.rs` — rate limiter with wildcard patterns (new)
- `src/confirmation.rs` — Layer 8 confirmation manager (new)
- `src/health.rs` — doctor subcommand (mostly done)
- `src/connection.rs` — health monitor, backoff, shutdown
- `src/main.rs` — graceful shutdown handler

**Key Concepts:**
- Append-only file logging
- Sliding window rate limiting with glob pattern support
- Token-based confirmation (single-use, 5-min expiry, per security-approach.md Layer 8)
- Command pattern validation
- Exponential backoff for reconnection
- SIGTERM/SIGINT signal handling with in-flight drain

**See:** `M2-M5-Implementation.md` → "Milestone 4: Safety and Observability"

---

### M5: End-to-End Validation (1 day)

**Goal:** Verify complete integration with Spacebot.

**Deliverables:**
- Spacebot configuration for MCP server
- Docker tools accessible and working via Spacebot
- SSH tools accessible and working via Spacebot (with dry-run)
- Layer 8 confirmation flow verified end-to-end
- Layer 9 output envelope verified on all tool responses
- Audit logging and rate limiting verified
- Error handling verified

**Key Activities:**
1. Configure Spacebot MCP settings
2. Test Docker tool discovery (9 tools: 5 Docker + 3 SSH + confirm_operation)
3. Test Docker operations (list, start, stop, logs, inspect)
4. Test SSH operations (dry-run, validation, execution)
5. Test Layer 8 confirmation flow (ssh.exec with dangerous pattern → token → confirm)
6. Verify Layer 9 output envelope (data_classification: "untrusted_external")
7. Verify audit logging
8. Test rate limiting (including wildcard patterns)
9. Test error handling, recovery, and graceful shutdown

**See:** `M2-M5-Implementation.md` → "Milestone 5: End-to-End Validation"

---

## Project Structure

```
spacebot-homelab-mcp/
├── Cargo.toml                    # Dependencies and build config
├── Cargo.lock                    # Lock file
├── README.md                     # User-facing documentation
├── example.config.toml           # Example configuration
│
├── M1-Implementation.md          # M1 plan (11 steps)
├── M2-M5-Implementation.md       # M2-M5 plan (comprehensive)
├── IMPLEMENTATION-GUIDE.md       # This file
│
├── src/
│   ├── main.rs                   # CLI entry point, server startup, graceful shutdown
│   ├── config.rs                 # Configuration parsing/validation
│   ├── connection.rs             # ConnectionManager (Docker + SSH), health monitor, backoff
│   ├── audit.rs                  # Audit logging
│   ├── health.rs                 # Doctor subcommand
│   ├── rate_limit.rs             # Rate limiting with wildcard patterns (M4)
│   ├── confirmation.rs           # Layer 8 confirmation token manager (M4)
│   ├── mcp.rs                    # MCP server handler, tool registration
│   └── tools/
│       ├── mod.rs                # Module exports
│       ├── docker.rs             # 5 Docker tool handlers (M2)
│       └── ssh.rs                # 3 SSH tool handlers (M3)
│
└── tests/
    ├── docker_integration.rs     # Docker integration tests
    └── ssh_integration.rs        # SSH integration tests
```

---

## Dependencies by Milestone

### M1 Core
- `rmcp` (1.1) — MCP protocol server
- `tokio` — async runtime
- `serde`, `serde_json` — serialization
- `clap` — CLI parsing
- `tracing` — logging

### M2 Addition
- `bollard` (0.17) — Docker API client
- `futures` — async utilities

### M3 Addition
- `russh` (0.46) — SSH client
- `russh-keys` (0.46) — SSH key handling

### M4 Addition
- `uuid` (1, features: v4) — confirmation token generation

### M5 Addition
- No new dependencies

---

## Key Patterns and Conventions

### Error Handling
- All tool errors return `anyhow::Result<String>`
- Errors are returned as MCP tool results, not protocol errors
- Clear error messages for LLM to understand and recover

### Output Envelope (Layer 9)
- **All tool output** is wrapped via `wrap_output_envelope()` with `data_classification: "untrusted_external"`
- This marks tool output as data (not instructions) per security-approach.md Layer 9
- Helps defend against prompt injection via tool output

### Confirmation Flow (Layer 8)
- Destructive operations may require token-based confirmation per security-approach.md Layer 8
- Tokens are single-use, expire in 5 minutes, and are tool-specific
- `confirm_operation` MCP tool validates tokens and re-dispatches to the original tool
- Config `[confirm]` section controls which tools require confirmation

### Logging
- Structured logging via `tracing`
- Audit logging records tool invocations
- Health status changes are logged

### Configuration
- TOML-based configuration file
- Defaults provided for all settings
- Validation on load
- Config file permissions validated (0600/0640) per security-approach.md Layer 1

### Testing
- Unit tests for business logic (command validation, rate limiting, confirmation tokens)
- Integration tests marked with `#[ignore]` (require Docker/SSH)
- Tests use `#[tokio::test]` for async

### Output Formatting
- Human-readable tables for lists
- Structured summaries for detailed output
- Output truncation with notice when exceeding limits
- Environment variable values redacted in inspect output

---

## Execution Order

**Phase 1: M1 (Day 1)**
1. Read `M1-Implementation.md`
2. Follow steps 1-11 in order
3. Verify each step with `cargo check`
4. Test locally with `spacebot-homelab-mcp server`

**Phase 2: M2 (Days 2-4)**
1. Read M2 section in `M2-M5-Implementation.md`
2. Implement Docker client initialization
3. Implement 5 Docker tool handlers
4. Register tools with MCP server
5. Compile and test locally
6. Integration test against local Docker

**Phase 3: M3 (Days 5-6)**
1. Read M3 section in `M2-M5-Implementation.md`
2. Implement SSH pool types
3. Implement 3 SSH tool handlers
4. Register tools with MCP server
5. Compile and test locally
6. Integration test with test SSH server

**Phase 4: M4 (Days 7-8)**
1. Read M4 section in `M2-M5-Implementation.md`
2. Implement rate limiting with wildcard pattern support
3. Integrate rate limits into tool handlers
4. Implement Layer 8 confirmation flow (ConfirmationManager + confirm_operation tool)
5. Implement health monitor background task (30s interval)
6. Implement reconnection backoff
7. Implement graceful shutdown (SIGTERM/SIGINT handler)
8. Enhance doctor subcommand
9. Compile and test (rate_limit + confirmation tests)

**Phase 5: M5 (Day 9)**
1. Read M5 section in `M2-M5-Implementation.md`
2. Configure Spacebot for MCP
3. Test tool discovery (9 tools)
4. Test Docker tools end-to-end
5. Test SSH tools end-to-end
6. Test Layer 8 confirmation flow end-to-end
7. Verify Layer 9 output envelope on all responses
8. Verify audit logging and rate limiting
9. Test graceful shutdown

---

## Verification at Each Stage

### M1 Verification
```bash
cargo build --release
./target/release/spacebot-homelab-mcp server --config example.config.toml
# Server should start and wait for MCP messages
# Press Ctrl+C to stop
```

### M2 Verification
```bash
cargo build --release
# If Docker daemon is running:
./target/release/spacebot-homelab-mcp server --config example.config.toml
# Connect with MCP client and call docker.container.list
```

### M3 Verification
```bash
cargo test --lib ssh::tests
# Command validation tests should pass
```

### M4 Verification
```bash
cargo test --lib rate_limit
cargo test --lib confirmation
# Rate limiter tests (including wildcard patterns) should pass
# Confirmation manager tests (token lifecycle) should pass
```

### M5 Verification
```bash
# In Spacebot, send messages:
# "List my Docker containers"
# "Run df -h on localhost (dry-run)"
# "Run rm -rf /tmp/old on localhost"  → should return confirmation token
# Check /tmp/homelab-audit.log for audit entries
# Verify tool responses include data_classification: "untrusted_external"
# Press Ctrl+C to test graceful shutdown
```

---

## Troubleshooting Quick Reference

### Compilation Issues
- **`rmcp` not found:** Verify `Cargo.toml` has correct crate version and features
- **Tool handler signature mismatch:** Ensure parameter names and types match the schema
- **Bollard type mismatch:** Check bollard version (0.17)

### Runtime Issues
- **Docker daemon unreachable:** Verify Docker socket exists and daemon is running
- **SSH connection failed:** Check SSH host is reachable and key permissions are correct
- **Rate limiter blocking:** Wait 60 seconds for window to reset or check configured limits

### Testing Issues
- **Integration tests skipped:** Tests are marked `#[ignore]` — use `--ignored` flag to run
- **Docker not available in CI:** Tests gracefully skip if Docker is not running

---

## Future Enhancements (Post-M5)

1. **SFTP Implementation** — full file transfer support (M3 uses exec-based scp fallback)
2. **Per-User Rate Limiting** — track usage by Spacebot user, not just global
3. **Syslog Support** — optional audit logging to syslog
4. **Metrics** — Prometheus metrics for usage and performance
5. **SSH Channel Multiplexing** — multiple exec channels on a single session (V2 optimization)
6. **docker.image.* tools** — image management (pull, list, prune) per architecture-decision.md
7. **docker.container.delete / create** — destructive tools that fully exercise Layer 8 `"always"` confirmation

---

## Support and Debugging

### Logs
- Server logs go to stderr
- Set `RUST_LOG=debug` for verbose logging: `RUST_LOG=debug ./spacebot-homelab-mcp server`

### Audit Log
- Location: configured in `[audit].file` setting
- Format: `timestamp tool=name host=host result=status details=...`
- Append-only: new entries are always added

### Health Check
```bash
./target/release/spacebot-homelab-mcp doctor --config ~/.spacebot-homelab/config.toml
```

This will validate all configured hosts and report connection status.

---

## Document References

- **Detailed Architecture:** `spacebot/homelab-integration/architecture-decision.md`
- **Security Model:** `spacebot/homelab-integration/security-approach.md`
- **Connection Design:** `spacebot/homelab-integration/connection-manager.md`
- **Tool Schemas:** `spacebot/homelab-integration/poc-specification.md` (lines 57-480)
- **Test Plan:** `spacebot/homelab-integration/poc-specification.md` (lines 515-583)

---

## Quick Start (Post-M5)

Once all milestones are complete:

```bash
# 1. Build the binary
cargo build --release

# 2. Create config
mkdir -p ~/.spacebot-homelab
cp example.config.toml ~/.spacebot-homelab/config.toml
# Edit config with your Docker/SSH hosts

# 3. Verify setup
./target/release/spacebot-homelab-mcp doctor --config ~/.spacebot-homelab/config.toml

# 4. Configure Spacebot (in ~/.spacebot/config.toml)
# [[mcp_servers]]
# name = "homelab"
# transport = "stdio"
# enabled = true
# command = "/path/to/spacebot-homelab-mcp"
# args = ["server", "--config", "~/.spacebot-homelab/config.toml"]

# 5. Restart Spacebot
# homelab tools are now available!
```

---

## Implementation Status

- [x] M1: Binary boots and serves MCP (foundation)
- [x] M2: Docker tools work
- [x] M3: SSH tools work
- [x] M4: Safety and observability
- [ ] M5: End-to-end validation

Ready to execute! See `M1-Implementation.md` to begin.
