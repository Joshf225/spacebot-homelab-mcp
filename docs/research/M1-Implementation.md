# M1 Implementation Plan: Binary Boots and Serves MCP

**Scope:** Make the `spacebot-homelab-mcp` binary boot successfully and serve an MCP (Model Context Protocol) server over stdio with an empty tool list. Spacebot should be able to connect to it, discover the server, and confirm it's ready.

**Success Criteria:**
- Binary compiles without errors
- `spacebot-homelab-mcp server --config <path>` starts and listens on stdio
- MCP server responds to `tools/list` with an empty list
- Spacebot can spawn the binary as a child process and retrieve the server info
- `spacebot-homelab-mcp doctor` validates the configuration

**Estimated effort:** 1-2 days

---

## Architecture Overview

The MCP server will:
1. **CLI parsing** — `main.rs` handles `server` and `doctor` subcommands
2. **Config loading** — `config.rs` parses TOML, validates, and provides connection strings
3. **Connection manager** — `connection.rs` initializes Docker/SSH clients (empty stubs for M1)
4. **MCP server** — Runs on stdio using `rmcp` crate, registers empty tool list
5. **Audit logger** — `audit.rs` logs tool invocations (mostly done)
6. **Health checker** — `health.rs` powers the `doctor` subcommand

### Key Technologies

- **MCP crate:** `rmcp` version `1.1` (the official Rust MCP SDK from `github.com/modelcontextprotocol/rust-sdk`)
- **Features needed:** `server` (ServerHandler trait), `transport-io` (stdio), `macros` (#[tool] macros)
- **Transport:** stdio (stdin/stdout) — Spacebot spawns the binary as a child process and communicates over pipes
- **Protocol:** JSON-RPC 2.0 over stdio, per the MCP spec

---

## Step-by-Step Implementation

### Step 1: Update Cargo.toml with correct MCP crate

**File:** `Cargo.toml`

**Current state:** Line 9 has MCP crate commented out:
```toml
# mcp = { version = "0.1" }
```

**Action:** Replace with:
```toml
rmcp = { version = "1.1", features = ["server", "transport-io", "macros"] }
schemars = "1.2"
```

**Why these features:**
- `server` — enables `ServerHandler` trait and `serve_server()` function
- `transport-io` — enables `rmcp::transport::io::stdio()` which handles stdin/stdout
- `macros` — enables `#[tool]`, `#[tool_router]`, `#[tool_handler]` proc macros for tool registration
- `schemars` — JSON schema generation for tool input validation

**Verification:** Run `cargo check` — should compile without errors.

---

### Step 1b: Add config file permission validation at startup

**File:** `src/config.rs` — update the `Config::load()` method

**Design doc reference:** `security-approach.md` Layer 1 — "Config file permissions are validated at startup (must be `0600` or `0640`, owned by the running user)"

**Action:** Add permission validation after the file existence check, before reading the file:

```rust
// In Config::load(), after the config_path.exists() check:

// Validate config file permissions (Layer 1: Credential Management)
#[cfg(unix)]
{
    use std::os::unix::fs::MetadataExt;
    let metadata = std::fs::metadata(&config_path)?;
    let mode = metadata.mode() & 0o777; // Extract permission bits
    if mode != 0o600 && mode != 0o640 {
        return Err(anyhow!(
            "Configuration file {:?} has insecure permissions {:o}. \
             Must be 0600 or 0640 (contains credential references). \
             Fix with: chmod 600 {:?}",
            config_path, mode, config_path
        ));
    }

    // Verify owned by the running user
    let file_uid = metadata.uid();
    let running_uid = unsafe { libc::getuid() };
    if file_uid != running_uid {
        return Err(anyhow!(
            "Configuration file {:?} is owned by UID {} but the server is running as UID {}. \
             The config file must be owned by the running user.",
            config_path, file_uid, running_uid
        ));
    }
}
```

**Why this matters:** The config file contains paths to SSH private keys and Docker connection strings. If another user can read it, they can discover credential locations.

**Dependencies:** Add `libc` to `Cargo.toml`:
```toml
libc = "0.2"
```

**Note:** The existing `config.rs` already has an SSH root user warning in `validate()` (lines 221-229). The plan should ensure this validation stays in place — it implements Layer 4 of `security-approach.md`.

**Verification:** `cargo check` compiles. Test with a config file that has `0644` permissions — should fail with a clear error.

---

### Step 2: Create the MCP Server Handler struct

**File:** Create `src/mcp.rs`

This module defines the MCP server handler that will respond to the MCP protocol.

```rust
use rmcp::handler::server::ServerHandler;
use rmcp::model::{ServerCapabilities, ServerInfo};
use serde::Deserialize;
use std::sync::Arc;
use crate::config::Config;
use crate::connection::ConnectionManager;

/// The MCP server handler
/// Implements the ServerHandler trait to respond to MCP protocol messages
#[derive(Clone)]
pub struct HomelabMcpServer {
    config: Arc<Config>,
    manager: Arc<ConnectionManager>,
}

impl HomelabMcpServer {
    pub fn new(config: Arc<Config>, manager: Arc<ConnectionManager>) -> Self {
        Self { config, manager }
    }
}

// Implement the ServerHandler trait
// This tells rmcp what server info to return and what tools we support
#[rmcp::handler::server::tool_handler]
impl ServerHandler for HomelabMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions("Homelab Docker and SSH tools for Spacebot")
    }
}
```

**Key points:**
- `ServerHandler` is the main trait that rmcp calls
- `get_info()` returns metadata about this server (name, capabilities, instructions)
- We enable tools but don't define any yet (M1 has an empty tool list)
- The struct holds references to Config and ConnectionManager for later use

**Verification:**
- File exists at `src/mcp.rs`
- Module is declared in `main.rs`: `mod mcp;`
- Code compiles without errors

---

### Step 3: Add MCP module to main.rs

**File:** `src/main.rs`

**Action:** Add the module declaration at the top (after other `mod` statements):
```rust
mod mcp;
```

Add this import after the other imports:
```rust
use mcp::HomelabMcpServer;
```

**Verification:** `cargo check` compiles.

---

### Step 4: Implement the `run_server` function to start the MCP server

**File:** `src/main.rs` — replace the `run_server` function

**Current code (lines 59-79):**
```rust
async fn run_server(config_path: Option<PathBuf>) -> Result<()> {
    info!("Starting spacebot-homelab-mcp server");

    // Load config
    let config = Config::load(config_path)?;
    info!("Configuration loaded: {} Docker hosts, {} SSH hosts",
        config.docker.len(),
        config.ssh.hosts.len()
    );

    // Create connection manager
    let _manager = ConnectionManager::new(config).await?;
    info!("Connection manager initialized");

    // TODO: Start MCP server on stdio
    // TODO: Register tools (docker.*, ssh.*)
    // TODO: Handle MCP protocol messages

    info!("MCP server ready");
    Ok(())
}
```

**Replace with:**
```rust
async fn run_server(config_path: Option<PathBuf>) -> Result<()> {
    info!("Starting spacebot-homelab-mcp server");

    // Load config
    let config = std::sync::Arc::new(Config::load(config_path)?);
    info!("Configuration loaded: {} Docker hosts, {} SSH hosts",
        config.docker.hosts.len(),
        config.ssh.hosts.len()
    );

    // Create connection manager
    let manager = ConnectionManager::new((*config).clone()).await?;
    info!("Connection manager initialized");

    // Create MCP server handler
    let server = HomelabMcpServer::new(config.clone(), manager);
    info!("MCP server handler created");

    // Set up stdio transport
    let (read, write) = rmcp::transport::io::stdio();
    info!("Stdio transport initialized");

    // Start the MCP server
    // This function runs the server loop until the connection closes
    let service = rmcp::serve_server(server, (read, write)).await?;
    info!("MCP server started, waiting for messages...");

    // Wait for the server to finish (blocks until connection closes or error)
    service.waiting().await?;

    info!("MCP server connection closed");
    Ok(())
}
```

**Why this works:**
1. Config is loaded and validated (including file permission check from Step 1b, SSH root user warning from existing config.rs)
2. ConnectionManager is created (stubs for M1)
3. `HomelabMcpServer::new()` creates the MCP handler
4. `rmcp::transport::io::stdio()` returns a tuple of (reader, writer) for stdin/stdout
5. `rmcp::serve_server(handler, transport)` starts the MCP server and returns a service
6. `service.waiting()` blocks until the connection closes (Spacebot kills the process)

**Verification:**
- `cargo check` compiles
- No compiler errors about `rmcp` crate or `HomelabMcpServer`

---

### Step 5: Fix the ConnectionManager to be cloneable

**File:** `src/connection.rs`

**Issue:** The current `ConnectionManager::new()` returns `Result<Arc<Self>>`, but we're trying to clone a `Config` into it. We need to adjust the signature.

**Current code (line 46):**
```rust
pub async fn new(config: Config) -> Result<Arc<Self>> {
```

**Change to:**
```rust
pub async fn new(config: Config) -> Result<Self> {
```

**Update the return statement (lines 47-52):**
```rust
pub async fn new(config: Config) -> Result<Self> {
    let manager = Self {
        config: Arc::new(config),
        docker_clients: DashMap::new(),
        ssh_pools: DashMap::new(),
        health: DashMap::new(),
    };
```

And remove the wrapping in `Ok(manager)` — it becomes `Ok(manager)` instead of `Ok(Arc::new(manager))`.

**Rationale:** The `Arc` wrapping happens in `main.rs` at the call site, making the flow clearer. The manager itself should not impose Arc wrapping.

**Verification:** `cargo check` compiles.

---

### Step 6: Add #[derive(Clone)] to DockerClient and SshPool

**File:** `src/connection.rs` — lines 15-25

**Current code:**
```rust
#[derive(Clone)]
pub struct DockerClient {
    // TODO: will contain bollard::Docker client
}

#[derive(Clone)]
pub struct SshPool {
    // TODO: will contain Vec<PooledSession> and pool state
}
```

These are already correct (they have `#[derive(Clone)]`). No changes needed.

**Verification:** Already present.

---

### Step 7: Update main.rs to wrap manager in Arc

**File:** `src/main.rs` — in `run_server` function

After creating the manager (around line 68 in the new code):
```rust
let manager = ConnectionManager::new((*config).clone()).await?;
```

**Change to:**
```rust
let manager = std::sync::Arc::new(ConnectionManager::new((*config).clone()).await?);
```

**Rationale:** The MCP server needs to clone the manager to send it to rmcp. Wrapping in Arc allows cheap clones.

**Updated run_server:**
```rust
async fn run_server(config_path: Option<PathBuf>) -> Result<()> {
    info!("Starting spacebot-homelab-mcp server");

    // Load config
    let config = std::sync::Arc::new(Config::load(config_path)?);
    info!("Configuration loaded: {} Docker hosts, {} SSH hosts",
        config.docker.hosts.len(),
        config.ssh.hosts.len()
    );

    // Create connection manager
    let manager = std::sync::Arc::new(ConnectionManager::new((*config).clone()).await?);
    info!("Connection manager initialized");

    // Create MCP server handler
    let server = HomelabMcpServer::new(config.clone(), manager.clone());
    info!("MCP server handler created");

    // Set up stdio transport
    let (read, write) = rmcp::transport::io::stdio();
    info!("Stdio transport initialized");

    // Start the MCP server
    let service = rmcp::serve_server(server, (read, write)).await?;
    info!("MCP server started, waiting for messages...");

    // Wait for the server to finish
    service.waiting().await?;

    info!("MCP server connection closed");
    Ok(())
}
```

**Verification:** `cargo check` compiles.

---

### Step 8: Compile and test the binary

**Command:**
```bash
cargo build --release
```

**Expected output:**
- Compilation succeeds
- Binary created at `target/release/spacebot-homelab-mcp`
- No warnings about unused code or compilation errors

**If there are compile errors:**
- Read the error message carefully
- Check that all imports are correct
- Verify the rmcp crate version is `1.1` in Cargo.toml
- Verify features are correct: `["server", "transport-io", "macros"]`

---

### Step 9: Test the server startup locally

**Command:**
```bash
./target/release/spacebot-homelab-mcp server --config example.config.toml
```

**Expected output:**
```
Starting spacebot-homelab-mcp server
Configuration loaded: 2 Docker hosts, 3 SSH hosts
Connection manager initialized
MCP server handler created
Stdio transport initialized
MCP server started, waiting for messages...
```

**What happens next:**
- The server is now waiting for MCP protocol messages on stdin
- Any input that's not valid JSON-RPC will cause an error
- Press Ctrl+C to stop the server

**Verification:**
- Server starts without panicking
- Logs appear on stderr
- Process remains running and listening

---

### Step 10: Verify MCP protocol via echo

**Command (in another terminal, with server running):**
```bash
echo '{"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}' | \
  ./target/release/spacebot-homelab-mcp server --config example.config.toml
```

**Expected output:**
The server will respond with an initialization response (valid JSON-RPC). The exact format depends on rmcp's implementation, but it should be valid JSON on stdout.

**Note:** This is a manual test. Spacebot will do this automatically when it spawns the server.

---

### Step 11: Test the doctor subcommand

**Command:**
```bash
./target/release/spacebot-homelab-mcp doctor --config example.config.toml
```

**Expected output:**
```
Validating spacebot-homelab-mcp configuration...

Checking Docker hosts:
  ✓ Docker 'local': unix:///var/run/docker.sock → accessible
  ✓ Docker 'vps': tcp://vps.example.com:2375 → accessible

Checking SSH hosts:
  ✓ SSH 'home': homelab-agent@192.168.1.1 → OK
  ✓ SSH 'nas': homelab-agent@192.168.1.50 → OK
  ✓ SSH 'proxmox': homelab-agent@192.168.1.10 → OK

Checking security configuration:
  ✓ Security checks passed

Configuration summary:
  2 Docker hosts, 3 SSH hosts
  SSH pool: max 3 sessions, 30 min lifetime, 5 min idle
  Audit logging: enabled (file)

Ready to start.
```

(Output may vary based on actual connectivity to Docker/SSH hosts. The important part is that it runs and reports status.)

**Verification:** Doctor command runs without panicking.

---

## Integration with Spacebot

Once M1 is complete, Spacebot can be configured to use the homelab MCP server.

### Spacebot Configuration

In Spacebot's config (typically `~/.spacebot/config.toml`), add:

```toml
[[mcp_servers]]
name = "homelab"
transport = "stdio"
enabled = true
command = "/path/to/spacebot-homelab-mcp"
args = ["server", "--config", "/path/to/config.toml"]
env = {}
```

### Spacebot Connection Flow

1. Spacebot reads the config and sees `mcp_servers.homelab`
2. When initializing tools, Spacebot spawns the child process:
   ```bash
   /path/to/spacebot-homelab-mcp server --config /path/to/config.toml
   ```
3. Spacebot connects via `rmcp::transport::TokioChildProcess::new(child_command)`
4. Spacebot sends MCP `initialize` message
5. Our server responds with `ServerInfo` (from `HomelabMcpServer::get_info()`)
6. Spacebot calls `tools/list`
7. Our server responds with an empty list (M1 has no tools yet)
8. Spacebot confirms the server is ready

### Verification Checklist

- [ ] Binary compiles: `cargo build --release`
- [ ] Binary runs: `./target/release/spacebot-homelab-mcp server --config example.config.toml`
- [ ] Doctor works: `./target/release/spacebot-homelab-mcp doctor --config example.config.toml`
- [ ] MCP server accepts stdio: Can send JSON-RPC messages and receive responses
- [ ] Spacebot can spawn the binary as child process
- [ ] Spacebot receives initialization response
- [ ] Spacebot receives empty tools list
- [ ] Spacebot marks the homelab MCP server as ready

---

## Error Handling Strategy

**If compilation fails:**
1. Check the error message — it will point to the exact line
2. Common issues:
   - `rmcp` crate not found → verify Cargo.toml has the dependency
   - `ServerHandler` not found → verify features include `"server"`
   - `stdio()` function not found → verify features include `"transport-io"`
   - Type mismatch on `serve_server()` — ensure tuple `(read, write)` is passed correctly

**If the server panics on startup:**
1. Check that config.toml is valid TOML
2. Check that Docker hosts and SSH hosts are accessible (or at least configured correctly)
3. Check logs for "Connection manager initialized" — if missing, config loading failed
4. Check that the example.config.toml is in the right format

**If Spacebot can't connect:**
1. Verify the binary path is correct
2. Verify the config path is correct
3. Check that the server process is actually starting (test manually first)
4. Ensure environment variables (PATH, etc.) are available to the child process

---

## Testing Approach for M1

### Unit Tests

Create `tests/mcp_server.rs`:

```rust
#[tokio::test]
async fn test_mcp_server_starts() {
    // Spawn the server as a child process
    // Send initialize message
    // Verify it responds with ServerInfo
    // Assert server_info.name contains "homelab"
    // Assert capabilities.tools is true
}

#[tokio::test]
async fn test_tools_list_empty() {
    // Spawn the server
    // Send tools/list request
    // Verify response contains empty array
}

#[tokio::test]
async fn test_doctor_runs() {
    // Run doctor subcommand
    // Verify exit code is 0
    // Verify output contains "Ready to start"
}
```

These tests verify the MCP protocol works at the binary level.

### Integration with Spacebot

Once the binary is ready:
1. Configure Spacebot's `[[mcp_servers]]` section with the homelab server
2. Restart Spacebot
3. Check logs to see if the server connects successfully
4. Try sending a message that would use homelab tools (they'll error since M2 isn't done, but at least they'll be discoverable)

---

## Files Modified / Created

| File | Status | Notes |
|------|--------|-------|
| `Cargo.toml` | **Edit** | Add rmcp 1.1 with features, add libc 0.2 |
| `src/main.rs` | **Edit** | Add mcp module, rewrite run_server |
| `src/config.rs` | **Edit** | Add config file permission validation (0600/0640) |
| `src/connection.rs` | **Edit** | Change new() return type from Arc<Self> to Self |
| `src/mcp.rs` | **Create** | MCP server handler |
| `tests/mcp_server.rs` | **Create** | Basic MCP tests |

---

## Rollback Plan

If something goes wrong:
1. The changes are isolated to MCP startup — no tools are implemented
2. The `doctor` subcommand is unaffected
3. To rollback: revert Cargo.toml and remove `src/mcp.rs`
4. The ConnectionManager changes (removing Arc) are backward compatible

---

## Success Criteria Checklist

- [ ] `cargo build --release` succeeds with no errors
- [ ] `./spacebot-homelab-mcp server --config <path>` starts and waits for MCP messages
- [ ] `./spacebot-homelab-mcp doctor --config <path>` runs and exits successfully
- [ ] MCP server responds to `tools/list` with an empty list
- [ ] Spacebot can spawn the binary as a child process
- [ ] Spacebot calls initialize and receives ServerInfo
- [ ] Spacebot calls tools/list and receives empty array
- [ ] Process exits cleanly when Spacebot closes the connection

Once all criteria are met, M1 is complete and M2 (Docker tools) can begin.

---

## Next Steps (M2)

After M1 is verified:
1. Implement tool handlers for Docker (container.list, container.start, etc.)
2. Connect the handlers to the MCP server via tool registration
3. Test each tool against local Docker daemon
4. Verify Spacebot can call the tools and receive results

See `poc-specification.md` for M2 details.
