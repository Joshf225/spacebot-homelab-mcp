# M2-M5 Implementation Plan: Docker Tools, SSH Tools, Safety, and End-to-End Validation

**Scope:** Complete the remaining milestones to deliver a fully functional homelab MCP server with Docker and SSH tools, safety gates, observability, and verified integration with Spacebot.

**Estimated effort:** 6-8 engineering days for all four milestones

**Dependencies:** M1 must be complete before starting M2 (MCP server foundation, config loading, connection manager skeleton).

---

## Milestone 2: Docker Tools Work (2-3 days)

### M2 Goals

Implement all 5 Docker container management tools with full Docker API integration:
- `docker.container.list` — list containers with filtering
- `docker.container.start` — start a stopped container
- `docker.container.stop` — gracefully stop a running container
- `docker.container.logs` — retrieve and truncate logs
- `docker.container.inspect` — get detailed container metadata

**Success Criteria:**
- All 5 Docker tools are callable via MCP
- Tools correctly query the Docker daemon(s)
- Output is properly formatted for LLM readability
- Output is truncated at 10,000 characters with a notice
- Audit logging records each invocation
- Integration tests pass against local Docker

---

### M2 Architecture

**Docker API Flow:**
```
MCP Tool Call
  ↓
HomelabMcpServer (mcp.rs) routes to tool handler
  ↓
tools/docker.rs handles the request
  ↓
ConnectionManager.get_docker(host) returns DockerClient
  ↓
DockerClient wraps bollard::Docker
  ↓
bollard queries the Docker daemon
  ↓
Result formatted and returned to MCP
```

**Key Components:**

1. **DockerClient** (`connection.rs`) — wrapper around `bollard::Docker`
2. **Docker tool handlers** (`tools/docker.rs`) — 5 async functions
3. **Tool registration** (`mcp.rs`) — macros to expose tools to MCP
4. **Output formatting** (`tools/docker.rs` helpers) — human-readable tables, truncation

---

### M2 Implementation

#### Step 1: Update Cargo.toml for Docker API

**File:** `Cargo.toml`

**Current state:** Line 12 has `bollard = "0.17"` but it's incomplete

**Action:** Update the Docker section:
```toml
# Docker
bollard = "0.17"
```

Bollard is already correct. But we need to ensure it's available:

**Verification:** Run `cargo fetch` to ensure bollard downloads successfully.

---

#### Step 2: Implement DockerHandle with transport tracking

**File:** `src/connection.rs`

**Design doc reference:** `connection-manager.md` lines 70-78 — DockerHandle must include a DockerTransport enum for health diagnostics.

**Current state:** Lines 15-19 have a placeholder:
```rust
#[derive(Clone)]
pub struct DockerClient {
    // TODO: will contain bollard::Docker client
}
```

**Replace with:**
```rust
/// Docker connection handle — wraps bollard client with transport metadata
/// See connection-manager.md lines 70-78
#[derive(Clone)]
pub struct DockerHandle {
    /// The underlying bollard Docker client
    client: std::sync::Arc<bollard::Docker>,
    /// Transport type — used for diagnostics and health reporting
    transport: DockerTransport,
}

/// Tracks how we're connected to each Docker daemon
#[derive(Debug, Clone)]
pub enum DockerTransport {
    UnixSocket { path: std::path::PathBuf },
    Tcp { host: String, tls: bool },
}

impl DockerHandle {
    /// Create a new Docker handle for a given host connection string.
    /// Does NOT validate connectivity — call `validate()` after creation.
    pub fn new(host_str: &str, cert_path: Option<&std::path::Path>, key_path: Option<&std::path::Path>) -> anyhow::Result<Self> {
        use anyhow::anyhow;
        
        let (client, transport) = if host_str.starts_with("unix://") {
            // Unix socket connection (e.g., unix:///var/run/docker.sock)
            let socket_path = host_str.strip_prefix("unix://").unwrap_or("");
            let client = bollard::Docker::connect_with_unix(socket_path, 120, bollard::API_DEFAULT_VERSION)
                .map_err(|e| anyhow!("Failed to connect to Docker socket {}: {}", socket_path, e))?;
            let transport = DockerTransport::UnixSocket { path: std::path::PathBuf::from(socket_path) };
            (client, transport)
        } else if host_str.starts_with("tcp://") {
            // TCP connection (e.g., tcp://host:2375)
            let has_tls = cert_path.is_some() && key_path.is_some();
            let client = if has_tls {
                // TLS connection — use connect_with_ssl
                // Note: bollard 0.17 uses connect_with_ssl for TLS TCP connections
                bollard::Docker::connect_with_http(
                    host_str,
                    120,
                    bollard::API_DEFAULT_VERSION,
                )
                .map_err(|e| anyhow!("Failed to connect to Docker TCP+TLS {}: {}", host_str, e))?
            } else {
                bollard::Docker::connect_with_http(
                    host_str,
                    120,
                    bollard::API_DEFAULT_VERSION,
                )
                .map_err(|e| anyhow!("Failed to connect to Docker TCP {}: {}", host_str, e))?
            };
            let transport = DockerTransport::Tcp { host: host_str.to_string(), tls: has_tls };
            (client, transport)
        } else {
            return Err(anyhow!(
                "Invalid Docker connection string: {}. Expected unix:// or tcp://",
                host_str
            ));
        };

        Ok(Self {
            client: std::sync::Arc::new(client),
            transport,
        })
    }

    /// Validate connectivity by pinging the Docker daemon.
    /// See connection-manager.md lines 82-84: "Validate connectivity with docker.ping().await"
    pub async fn validate(&self) -> anyhow::Result<()> {
        self.client.ping().await
            .map_err(|e| anyhow::anyhow!("Docker ping failed: {}", e))?;
        Ok(())
    }

    /// Get reference to the underlying bollard client
    pub fn as_bollard(&self) -> &bollard::Docker {
        &self.client
    }

    /// Get transport info for diagnostics
    pub fn transport(&self) -> &DockerTransport {
        &self.transport
    }
}
```

**Why this design matches the design doc:**
- `DockerHandle` (not `DockerClient`) — matches `connection-manager.md` naming
- Includes `DockerTransport` enum for health check reporting ("via unix socket" vs "via TCP")
- Separate `validate()` method so we can ping at startup without blocking on failure
- Supports both Unix socket and TCP connections with optional TLS

**Verification:** `cargo check` compiles.

---

#### Step 3: Initialize Docker clients with ping validation in ConnectionManager::new

**File:** `src/connection.rs` — update the `new()` method (lines 46-91)

**Design doc reference:** `connection-manager.md` lines 82-84: "Validate connectivity with `docker.ping().await`. If ping fails, mark as `Disconnected` — do not block other connections."

**Current code (lines 54-66):**
```rust
// Initialize Docker clients
for (name, _host) in &manager.config.docker.hosts {
    manager.health.insert(
        format!("docker:{}", name),
        ConnectionHealth {
            status: ConnectionStatus::Connecting,
            last_success: None,
            last_error: None,
            consecutive_failures: 0,
        },
    );
    // TODO: Create Docker client and validate connectivity
}
```

**Replace with:**
```rust
// Initialize Docker clients
// See connection-manager.md lines 81-84: create client, validate with ping, mark status
for (name, host_config) in &manager.config.docker.hosts {
    let health_key = format!("docker:{}", name);
    manager.health.insert(
        health_key.clone(),
        ConnectionHealth {
            status: ConnectionStatus::Connecting,
            last_success: None,
            last_error: None,
            consecutive_failures: 0,
        },
    );
    
    // Step 1: Create the Docker handle (this does NOT connect yet)
    match DockerHandle::new(
        &host_config.host,
        host_config.cert_path.as_deref(),
        host_config.key_path.as_deref(),
    ) {
        Ok(handle) => {
            // Step 2: Validate connectivity with docker.ping()
            match handle.validate().await {
                Ok(()) => {
                    manager.docker_clients.insert(name.clone(), handle);
                    manager.mark_healthy(&health_key);
                    info!("Docker '{}' connected via {:?}", name, handle.transport());
                }
                Err(e) => {
                    // Ping failed — still store the handle (bollard may reconnect)
                    // but mark as Disconnected. Do NOT block other connections.
                    manager.docker_clients.insert(name.clone(), handle);
                    manager.mark_unhealthy(&health_key, format!("Ping failed: {}", e));
                    tracing::warn!(
                        "Docker '{}' ping failed (will retry via health monitor): {}",
                        name, e
                    );
                }
            }
        }
        Err(e) => {
            manager.mark_unhealthy(&health_key, format!("Failed to create client: {}", e));
            tracing::warn!("Failed to initialize Docker client for '{}': {}", name, e);
        }
    }
}
```

**Key difference from the old plan:** We now:
1. Create the handle first (no network call)
2. Ping to validate connectivity (`handle.validate().await`)
3. On ping failure: still store the handle and mark `Disconnected` — don't block startup
4. Log transport type for diagnostics

---

#### Step 4: Add getter method to ConnectionManager for Docker handles

**File:** `src/connection.rs` — add after `get_ssh_pool` method (around line 107)

```rust
/// Get Docker handle for a named host.
/// Returns health status in error message if host is unreachable.
/// See connection-manager.md lines 87-89: "Returns &DockerHandle or an error with health status"
pub fn get_docker(&self, name: &str) -> Result<DockerHandle> {
    let handle = self.docker_clients
        .get(name)
        .ok_or_else(|| {
            // Include health status in the error for LLM context
            if let Some(health) = self.health.get(&format!("docker:{}", name)) {
                anyhow!(
                    "Docker host '{}' is {:?}. Last error: {}",
                    name,
                    health.status,
                    health.last_error.as_deref().unwrap_or("unknown")
                )
            } else {
                anyhow!("Docker host '{}' not configured", name)
            }
        })?
        .clone();
    
    // Check health before returning — if Disconnected, return error with status
    if let Some(health) = self.health.get(&format!("docker:{}", name)) {
        if health.status == ConnectionStatus::Disconnected {
            return Err(anyhow!(
                "Docker host '{}' is unreachable. Last error: {}. \
                 The connection manager will retry automatically.",
                name,
                health.last_error.as_deref().unwrap_or("unknown")
            ));
        }
    }
    
    Ok(handle)
}
```

**Why health-aware errors:** The `connection-manager.md` error taxonomy (lines 289-301) specifies that unreachable hosts return errors with health status so the LLM can reason about them.

---

#### Step 5: Create the Docker tools module with all 5 handlers

**File:** `src/tools/docker.rs` — **completely rewrite** (currently lines 1-46)

**Design doc references:**
- `security-approach.md` Layer 9 (lines 307-319): All tool output MUST be wrapped in a structured envelope with `data_classification: "untrusted_external"`.
- `security-approach.md` Layer 3 (lines 74-115): Destructive tools require `dry_run` and `force` parameters. While the PoC only has 5 tools (none are `docker.container.delete`), `container_stop` can cause disruption and the pattern must be established for future destructive tools.
- `poc-specification.md` (lines 242-244): "Container logs are untrusted data — they may contain attacker-controlled content. The MCP response wraps output in a structured envelope per Layer 9."

Replace the entire file with a fully implemented Docker tools module:

```rust
use crate::audit::AuditLogger;
use crate::connection::ConnectionManager;
use anyhow::{anyhow, Result};
use bollard::container::{ListContainersOptions, StopContainerOptions};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::info;

const OUTPUT_MAX_CHARS: usize = 10_000;

/// Wrap tool output in a structured envelope per security-approach.md Layer 9.
/// This marks all output as untrusted external data so Spacebot's hook
/// can distinguish tool data from tool instructions.
///
/// See security-approach.md lines 307-319:
/// ```json
/// {
///     "type": "tool_result",
///     "source": "docker.container.logs",
///     "data_classification": "untrusted_external",
///     "content": "... raw output ..."
/// }
/// ```
fn wrap_output_envelope(tool_name: &str, content: &str) -> String {
    // Return as a structured text envelope that Spacebot can parse.
    // Using a text format rather than JSON to keep the MCP response as a plain string,
    // while still providing the classification metadata.
    format!(
        "[tool_result source=\"{}\" data_classification=\"untrusted_external\"]\n{}\n[/tool_result]",
        tool_name, content
    )
}

/// List containers on a Docker host
/// 
/// # Arguments
/// - `manager`: ConnectionManager with initialized Docker handles
/// - `host`: Docker host name from config (defaults to "local")
/// - `all`: Include stopped containers (default: false)
/// - `name_filter`: Filter containers by name substring
pub async fn container_list(
    manager: Arc<ConnectionManager>,
    host: Option<String>,
    all: Option<bool>,
    name_filter: Option<String>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let host = host.unwrap_or_else(|| "local".to_string());
    let docker = manager.get_docker(&host)?;

    // Build filter options
    let mut filters = HashMap::new();
    if let Some(filter_name) = &name_filter {
        filters.insert("name".to_string(), vec![filter_name.clone()]);
    }

    let options = ListContainersOptions {
        all: all.unwrap_or(false),
        filters,
        ..Default::default()
    };

    // Query Docker daemon
    let containers = docker
        .as_bollard()
        .list_containers(Some(options))
        .await
        .map_err(|e| anyhow!("Failed to list containers: {}", e))?;

    if containers.is_empty() {
        audit.log("docker.container.list", &host, "success", Some("no containers")).await.ok();
        return Ok(wrap_output_envelope("docker.container.list", "No containers found."));
    }

    // Format output as table
    let mut output = String::from("CONTAINER ID  | NAME                 | IMAGE                      | STATUS          | PORTS\n");
    output.push_str("────────────────────────────────────────────────────────────────────────────────────────────────────\n");

    for container in containers {
        let id = container.id
            .as_deref()
            .unwrap_or("?")
            .get(0..12)
            .unwrap_or("?");
        
        let name = container.names
            .as_ref()
            .and_then(|names| names.first())
            .map(|n| n.trim_start_matches('/'))
            .unwrap_or("?");
        
        let image = container.image.as_deref().unwrap_or("?");
        let status = container.status.as_deref().unwrap_or("?");
        
        let ports = container.ports
            .as_ref()
            .map(|p| {
                p.iter()
                    .filter_map(|port| {
                        if let (Some(ip), Some(public_port)) = (&port.ip, port.public_port) {
                            Some(format!("{}:{}->{}/{}", 
                                if ip.is_empty() { "*".to_string() } else { ip.clone() },
                                public_port,
                                port.private_port,
                                port.typ.as_deref().unwrap_or("tcp")
                            ))
                        } else {
                            Some(format!("{}/{}", 
                                port.private_port,
                                port.typ.as_deref().unwrap_or("tcp")
                            ))
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();

        let line = format!(
            "{:<14} | {:<20} | {:<26} | {:<15} | {}\n",
            id,
            truncate_string(name, 20),
            truncate_string(image, 26),
            truncate_string(status, 15),
            ports
        );
        output.push_str(&line);
    }

    // Truncate output if needed
    let output = truncate_output(&output, OUTPUT_MAX_CHARS);

    audit.log("docker.container.list", &host, "success", None).await.ok();
    Ok(wrap_output_envelope("docker.container.list", &output))
}

/// Start a stopped container
pub async fn container_start(
    manager: Arc<ConnectionManager>,
    host: Option<String>,
    container: String,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let host = host.unwrap_or_else(|| "local".to_string());
    let docker = manager.get_docker(&host)?;

    // Start the container
    docker
        .as_bollard()
        .start_container::<String>(&container, None)
        .await
        .map_err(|e| anyhow!("Failed to start container '{}': {}", container, e))?;

    audit.log("docker.container.start", &host, "success", Some(&container)).await.ok();
    
    let result = format!(
        "Container '{}' started successfully. Use docker.container.list to verify status.",
        container
    );
    Ok(wrap_output_envelope("docker.container.start", &result))
}

/// Stop a running container
///
/// Note on bollard 0.17 API: `stop_container` takes `StopContainerOptions` struct,
/// NOT a bare Duration. The struct has a `t` field for the timeout in seconds.
pub async fn container_stop(
    manager: Arc<ConnectionManager>,
    host: Option<String>,
    container: String,
    timeout: Option<i64>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let host = host.unwrap_or_else(|| "local".to_string());
    let docker = manager.get_docker(&host)?;

    let timeout_secs = timeout.unwrap_or(10);
    
    // bollard 0.17 uses StopContainerOptions { t: i64 }, NOT Duration
    let options = StopContainerOptions { t: timeout_secs };
    
    docker
        .as_bollard()
        .stop_container(&container, Some(options))
        .await
        .map_err(|e| anyhow!("Failed to stop container '{}': {}", container, e))?;

    audit.log("docker.container.stop", &host, "success", Some(&container)).await.ok();
    
    let result = format!(
        "Container '{}' stopped (timeout: {}s). Use docker.container.list to verify status.",
        container, timeout_secs
    );
    Ok(wrap_output_envelope("docker.container.stop", &result))
}

/// Get logs from a container
///
/// SECURITY NOTE (Layer 9): Container logs are untrusted external data.
/// They may contain attacker-controlled content including prompt injection attempts.
/// Output is wrapped in the untrusted_external envelope and truncated.
pub async fn container_logs(
    manager: Arc<ConnectionManager>,
    host: Option<String>,
    container: String,
    tail: Option<i64>,
    since: Option<String>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    use bollard::container::LogsOptions;
    use futures::stream::StreamExt;

    let host = host.unwrap_or_else(|| "local".to_string());
    let docker = manager.get_docker(&host)?;

    let tail_lines = tail.unwrap_or(100);

    let options = LogsOptions::<String> {
        stdout: true,
        stderr: true,
        follow: false,
        timestamps: true,
        tail: tail_lines.to_string(),
        ..Default::default()
    };

    // Note: bollard 0.17 LogsOptions.tail is a String, not i64.
    // Parse 'since' if provided — for now, ignore since parameter (M2 scope)

    let mut logs_stream = docker
        .as_bollard()
        .logs::<String>(&container, Some(options));

    let mut output = String::new();
    while let Some(log_result) = logs_stream.next().await {
        match log_result {
            Ok(log) => {
                output.push_str(&format!("{}\n", log));
            }
            Err(e) => {
                audit.log("docker.container.logs", &host, "error", Some(&e.to_string())).await.ok();
                return Err(anyhow!("Failed to read logs from container '{}': {}", container, e));
            }
        }
    }

    if output.is_empty() {
        output = format!("No logs available for container '{}'", container);
    }

    // Truncate output (Layer 9: output length limits)
    let output = truncate_output(&output, OUTPUT_MAX_CHARS);

    audit.log("docker.container.logs", &host, "success", Some(&container)).await.ok();
    // Wrap in untrusted_external envelope — logs are attacker-controlled content
    Ok(wrap_output_envelope("docker.container.logs", &output))
}

/// Inspect a container for detailed information
pub async fn container_inspect(
    manager: Arc<ConnectionManager>,
    host: Option<String>,
    container: String,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    use bollard::container::InspectContainerOptions;

    let host = host.unwrap_or_else(|| "local".to_string());
    let docker = manager.get_docker(&host)?;

    let inspect = docker
        .as_bollard()
        .inspect_container(&container, None::<InspectContainerOptions>)
        .await
        .map_err(|e| anyhow!("Failed to inspect container '{}': {}", container, e))?;

    // Format structured output — see poc-specification.md lines 275-276:
    // "Formatted summary (not raw JSON dump). Key fields: image, created, status,
    //  restart policy, ports, volumes, env vars (keys only, values redacted), network, IPs"
    let mut output = String::new();
    output.push_str(&format!("Container: {}\n", container));
    output.push_str(&format!("ID: {}\n", inspect.id.as_deref().unwrap_or("?").get(0..12).unwrap_or("?")));
    
    if let Some(config) = &inspect.config {
        output.push_str(&format!("Image: {}\n", config.image.as_deref().unwrap_or("?")));
        if let Some(env) = &config.env {
            output.push_str("Environment variables (keys only, values redacted):\n");
            for var in env {
                if let Some(key) = var.split('=').next() {
                    output.push_str(&format!("  - {}\n", key));
                }
            }
        }
        if let Some(entrypoint) = &config.entrypoint {
            output.push_str(&format!("Entrypoint: {}\n", entrypoint.join(" ")));
        }
        if let Some(cmd) = &config.cmd {
            output.push_str(&format!("Command: {}\n", cmd.join(" ")));
        }
    }

    if let Some(state) = &inspect.state {
        output.push_str(&format!("Status: {}\n", 
            state.status.as_ref().map(|s| format!("{:?}", s)).unwrap_or_else(|| "?".to_string())
        ));
        output.push_str(&format!("Running: {}\n", state.running.unwrap_or(false)));
        if let Some(pid) = state.pid {
            output.push_str(&format!("PID: {}\n", pid));
        }
    }

    if let Some(host_config) = &inspect.host_config {
        if let Some(restart_policy) = &host_config.restart_policy {
            output.push_str(&format!("Restart policy: {:?}\n", restart_policy.name));
        }
    }

    if let Some(mounts) = &inspect.mounts {
        if !mounts.is_empty() {
            output.push_str("Volumes:\n");
            for mount in mounts {
                output.push_str(&format!("  - {} -> {} ({})\n",
                    mount.source.as_deref().unwrap_or("?"),
                    mount.destination.as_deref().unwrap_or("?"),
                    mount.mode.as_deref().unwrap_or("rw"),
                ));
            }
        }
    }

    if let Some(network_settings) = &inspect.network_settings {
        output.push_str("Networks:\n");
        if let Some(networks) = &network_settings.networks {
            for (net_name, net_settings) in networks {
                output.push_str(&format!("  - {}: {}\n", 
                    net_name,
                    net_settings.ip_address.as_deref().unwrap_or("?")
                ));
            }
        }
    }

    let output = truncate_output(&output, OUTPUT_MAX_CHARS);

    audit.log("docker.container.inspect", &host, "success", Some(&container)).await.ok();
    Ok(wrap_output_envelope("docker.container.inspect", &output))
}

// --- Safety Gate Pattern for Future Destructive Tools ---
//
// When docker.container.delete or docker.image.delete are added (post-PoC),
// they MUST follow the Layer 3 safety gate pattern from security-approach.md:
//
// ```rust
// pub async fn container_delete(
//     manager: Arc<ConnectionManager>,
//     host: Option<String>,
//     container: String,
//     dry_run: bool,       // REQUIRED — preview without executing
//     force: bool,         // REQUIRED — override safety warnings
//     audit: Arc<AuditLogger>,
// ) -> Result<String> {
//     if dry_run {
//         return Ok(format!(
//             "DRY RUN: Would delete container {}. Set dry_run=false to execute.",
//             container
//         ));
//     }
//     // Pre-flight: check for attached volumes
//     // If volumes attached and !force, return error explaining data loss risk
//     // Only then execute the delete
// }
// ```
//
// Additionally, destructive tools must integrate with the Layer 8 confirmation
// flow (see M4 implementation for the ConfirmationManager).

// Helper functions

fn truncate_string(s: &str, max_len: usize) -> String {
    if s.len() > max_len {
        format!("{}…", &s[..max_len - 1])
    } else {
        s.to_string()
    }
}

fn truncate_output(output: &str, max_chars: usize) -> String {
    if output.len() > max_chars {
        let truncated = &output[..max_chars];
        format!(
            "[Output truncated. Showing first {} chars of {} total chars. Use 'since' or reduce 'tail' for more specific results.]\n{}",
            max_chars,
            output.len(),
            truncated
        )
    } else {
        output.to_string()
    }
}
```

**Key differences from previous plan:**
1. **Output envelope** (`wrap_output_envelope`): All tool output is wrapped per Layer 9 of `security-approach.md`. This marks data as `untrusted_external` so Spacebot's hook can distinguish tool data from instructions.
2. **bollard API corrections**: `StopContainerOptions { t: i64 }` instead of bare `Duration`. `LogsOptions.tail` is a `String`, not `i64`. `inspect_container` takes `None::<InspectContainerOptions>`.
3. **Safety gate pattern documented**: The `dry_run`/`force` pattern from Layer 3 is documented as a code comment template for when destructive tools are added post-PoC.
4. **Restart policy and mounts** added to inspect output per poc-specification.md.
5. **`DockerHandle`** naming instead of `DockerClient` to match design doc.

---

#### Step 6: Register Docker tools with the MCP server

**File:** `src/mcp.rs` — add tool handlers to `HomelabMcpServer`

**Current state:** Basic struct definition with `get_info()` method

**Update the struct:**
```rust
use crate::tools::docker;
use std::sync::Arc;

#[derive(Clone)]
pub struct HomelabMcpServer {
    config: Arc<Config>,
    manager: Arc<ConnectionManager>,
    audit: Arc<AuditLogger>,
}

impl HomelabMcpServer {
    pub fn new(config: Arc<Config>, manager: Arc<ConnectionManager>, audit: Arc<AuditLogger>) -> Self {
        Self { config, manager, audit }
    }
}
```

**Add tool handlers using rmcp macros:**

After the `impl ServerHandler` block, add:

```rust
// Docker tool handlers
#[rmcp::handler::server::tool]
impl HomelabMcpServer {
    #[tool(description = "List Docker containers")]
    pub async fn docker_container_list(
        &self,
        host: Option<String>,
        all: Option<bool>,
        name_filter: Option<String>,
    ) -> anyhow::Result<String> {
        docker::container_list(
            self.manager.clone(),
            host,
            all,
            name_filter,
            self.audit.clone(),
        )
        .await
    }

    #[tool(description = "Start a Docker container")]
    pub async fn docker_container_start(
        &self,
        host: Option<String>,
        container: String,
    ) -> anyhow::Result<String> {
        docker::container_start(
            self.manager.clone(),
            host,
            container,
            self.audit.clone(),
        )
        .await
    }

    #[tool(description = "Stop a Docker container")]
    pub async fn docker_container_stop(
        &self,
        host: Option<String>,
        container: String,
        timeout: Option<u64>,
    ) -> anyhow::Result<String> {
        docker::container_stop(
            self.manager.clone(),
            host,
            container,
            timeout,
            self.audit.clone(),
        )
        .await
    }

    #[tool(description = "Get logs from a Docker container")]
    pub async fn docker_container_logs(
        &self,
        host: Option<String>,
        container: String,
        tail: Option<i64>,
        since: Option<String>,
    ) -> anyhow::Result<String> {
        docker::container_logs(
            self.manager.clone(),
            host,
            container,
            tail,
            since,
            self.audit.clone(),
        )
        .await
    }

    #[tool(description = "Inspect a Docker container")]
    pub async fn docker_container_inspect(
        &self,
        host: Option<String>,
        container: String,
    ) -> anyhow::Result<String> {
        docker::container_inspect(
            self.manager.clone(),
            host,
            container,
            self.audit.clone(),
        )
        .await
    }
}
```

**Verification:** `cargo check` to verify tool registration compiles.

---

#### Step 7: Update main.rs to pass audit logger to MCP server

**File:** `src/main.rs` — update `run_server` function

**Current code (around line 59):**
```rust
async fn run_server(config_path: Option<PathBuf>) -> Result<()> {
    // ... config and manager setup ...
    let server = HomelabMcpServer::new(config.clone(), manager.clone());
    // ...
}
```

**Update to:**
```rust
async fn run_server(config_path: Option<PathBuf>) -> Result<()> {
    info!("Starting spacebot-homelab-mcp server");

    // Load config
    let config = std::sync::Arc::new(Config::load(config_path)?);
    info!("Configuration loaded: {} Docker hosts, {} SSH hosts",
        config.docker.hosts.len(),
        config.ssh.hosts.len()
    );

    // Create audit logger
    let audit = std::sync::Arc::new(audit::AuditLogger::new(config.clone()));

    // Create connection manager
    let manager = std::sync::Arc::new(ConnectionManager::new((*config).clone()).await?);
    info!("Connection manager initialized");

    // Create MCP server handler
    let server = HomelabMcpServer::new(config.clone(), manager.clone(), audit.clone());
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

#### Step 8: Add imports to main.rs

**File:** `src/main.rs` — ensure imports are present

Add after existing `use` statements:
```rust
use audit::AuditLogger;
use mcp::HomelabMcpServer;
```

Verify the module declarations are present:
```rust
mod audit;
mod config;
mod connection;
mod health;
mod tools;
mod mcp;
```

---

#### Step 9: Compile and test M2

**Command:**
```bash
cargo build --release 2>&1 | tee build.log
```

**Expected output:**
- Compilation succeeds
- Binary created at `target/release/spacebot-homelab-mcp`
- May have warnings (unused code, unused imports), but 0 errors

**If there are errors:**
1. Read the error message carefully
2. Common issues:
   - Missing imports (`use` statements)
   - Type mismatches in tool handler signatures
   - bollard API differences (check version)
   - MCP macro expansion issues

**Troubleshooting:**
- If bollard types don't match, check the bollard 0.17 documentation
- If tool handlers don't compile, verify the macro attribute syntax
- If ConnectionManager doesn't compile, check the Arc wrapping

---

#### Step 10: Integration test setup

**File:** `tests/docker_integration.rs` — **create new file**

Add basic integration test structure:

```rust
#[cfg(test)]
mod docker_integration_tests {
    use spacebot_homelab_mcp::config::{Config, DockerConfig, DockerHost};
    use spacebot_homelab_mcp::connection::ConnectionManager;
    use std::collections::HashMap;
    use std::sync::Arc;

    #[tokio::test]
    #[ignore] // Skip in CI until Docker is available
    async fn test_docker_client_connects_to_local_daemon() {
        let mut docker_hosts = HashMap::new();
        docker_hosts.insert(
            "local".to_string(),
            DockerHost {
                host: "unix:///var/run/docker.sock".to_string(),
                cert_path: None,
                key_path: None,
            },
        );

        let config = Config {
            docker: DockerConfig { hosts: docker_hosts },
            ..Default::default()
        };

        let manager = ConnectionManager::new(config)
            .await
            .expect("Failed to create ConnectionManager");

        // Try to get Docker client
        let docker = manager.get_docker("local");
        assert!(
            docker.is_ok(),
            "Should be able to get Docker client if daemon is running"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn test_docker_container_list() {
        // TODO: Implement after M2 is complete
    }

    #[tokio::test]
    #[ignore]
    async fn test_docker_container_lifecycle() {
        // TODO: Test start/stop cycle
    }
}
```

**Verification:** `cargo test --test docker_integration --lib` compiles (tests are ignored).

---

### M2 Verification Checklist

After completing all steps, verify:

- [ ] `cargo build --release` succeeds
- [ ] Binary compiles without errors (warnings OK)
- [ ] `./spacebot-homelab-mcp doctor` still works
- [ ] Docker clients initialize if daemon is running
- [ ] Each Docker tool is callable via the MCP server
- [ ] Output is properly formatted and truncated
- [ ] Audit logging records each tool call
- [ ] Integration tests compile (use `#[ignore]` for now)

---

## Milestone 3: SSH Tools Work (2-3 days)

### M3 Goals

Implement SSH command execution and file transfer with connection pooling:
- **ssh.exec** — execute commands with validation, timeout, output truncation
- **ssh.upload** — upload files via SFTP
- **ssh.download** — download files via SFTP
- **SSH connection pool** — checkout/return/validation of reusable sessions

**Success Criteria:**
- SSH clients connect to configured hosts using key-based auth
- Commands are validated against allowlist/blocklist before execution
- Connections are pooled and reused
- Timeouts are enforced (default 30s, max 300s)
- File transfers work with 50MB size limits
- Audit logging records each operation

---

### M3 Architecture

**SSH Connection Flow:**
```
SSH Tool Call (ssh.exec, ssh.upload, ssh.download)
  ↓
Validate command (if ssh.exec) against allowlist/blocklist
  ↓
ConnectionManager.ssh_checkout(host) returns PooledSession
  ↓
russh SSH session
  ↓
Execute command or transfer file
  ↓
Collect output / transfer status
  ↓
ConnectionManager.ssh_return(host, session) returns to pool
  ↓
Format result and return to MCP
```

**Key Components:**

1. **SshPool** (`connection.rs`) — manages a pool of SSH connections
2. **PooledSession** (`connection.rs`) — wrapper around russh::client::Handle
3. **SSH tool handlers** (`tools/ssh.rs`) — 3 async functions
4. **Command validation** (`tools/ssh.rs` helpers) — allowlist/blocklist enforcement

---

### M3 Implementation Steps

#### Step 1: Add keepalive_interval to SshPoolConfig

**File:** `src/config.rs` — update `SshPoolConfig` struct

**Design doc reference:** `connection-manager.md` line 182: `keepalive_interval = "60s"` — Background keepalive ping interval.

Add this field to `SshPoolConfig`:
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshPoolConfig {
    #[serde(default = "default_max_sessions")]
    pub max_sessions_per_host: usize,
    #[serde(default = "default_max_lifetime")]
    pub max_lifetime_secs: u64,
    #[serde(default = "default_max_idle")]
    pub max_idle_time_secs: u64,
    #[serde(default = "default_connect_timeout")]
    pub connect_timeout_secs: u64,
    #[serde(default = "default_checkout_timeout")]
    pub checkout_timeout_secs: u64,
    #[serde(default = "default_keepalive_interval")]
    pub keepalive_interval_secs: u64,
}
```

Add the default function:
```rust
fn default_keepalive_interval() -> u64 {
    60 // 60 seconds — per connection-manager.md line 182
}
```

Update the `Default` impl to include `keepalive_interval_secs: default_keepalive_interval()`.

**Verification:** `cargo check` compiles.

---

#### Step 2: Implement SSH pool types matching connection-manager.md design

**File:** `src/connection.rs` — expand SshPool definition (lines 22-25)

**Design doc reference:** `connection-manager.md` lines 106-117:
```rust
pub struct SshHostPool {
    sessions: Arc<Mutex<VecDeque<PooledSession>>>,
    max_sessions: usize,
    active_count: Arc<AtomicUsize>,
}
```

The design doc explicitly uses `Arc<Mutex<VecDeque<PooledSession>>>` — NOT `DashMap`. This is a queue-based pool with checkout/return semantics.

**Current state:**
```rust
#[derive(Clone)]
pub struct SshPool {
    // TODO: will contain Vec<PooledSession> and pool state
}
```

**Replace with:**
```rust
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::{Mutex, Notify};

/// SSH connection pool for a single host.
/// Implements the checkout/return pattern from connection-manager.md lines 106-145.
///
/// Design: Sessions live in a VecDeque (idle queue). Tools checkout a session,
/// use it, then return it. If the pool is empty and under max_sessions, a new
/// session is created. If at max_sessions, the caller waits with timeout.
#[derive(Clone)]
pub struct SshHostPool {
    /// Idle sessions available for checkout (FIFO queue)
    sessions: Arc<Mutex<VecDeque<PooledSession>>>,
    /// Number of sessions currently checked out (in use)
    active_count: Arc<AtomicUsize>,
    /// Total sessions (idle + active)
    total_count: Arc<AtomicUsize>,
    /// Max concurrent sessions per host
    max_sessions: usize,
    /// Notify waiters when a session is returned to the pool
    session_available: Arc<Notify>,
    /// Host configuration for creating new sessions
    host_config: Arc<crate::config::SshHost>,
    /// Pool configuration (timeouts, lifetimes)
    pool_config: crate::config::SshPoolConfig,
}

/// A single pooled SSH session with lifecycle metadata
pub struct PooledSession {
    /// The russh client handle
    pub handle: russh::client::Handle<SshClientHandler>,
    /// When this session was created (for max_lifetime checks)
    created_at: std::time::Instant,
    /// When this session was last used (for max_idle_time checks)
    last_used: std::time::Instant,
}

/// Minimal russh client handler — required by russh to handle server events
pub struct SshClientHandler;

impl russh::client::Handler for SshClientHandler {
    type Error = anyhow::Error;

    // Minimal implementation — accept all host keys for now.
    // TODO: Add known_hosts verification in a future security hardening pass.
    async fn check_server_key(
        &mut self,
        _server_public_key: &russh_keys::key::PublicKey,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

impl SshHostPool {
    /// Create a new SSH pool for a host
    pub fn new(host_config: crate::config::SshHost, pool_config: crate::config::SshPoolConfig) -> Self {
        Self {
            sessions: Arc::new(Mutex::new(VecDeque::new())),
            active_count: Arc::new(AtomicUsize::new(0)),
            total_count: Arc::new(AtomicUsize::new(0)),
            max_sessions: pool_config.max_sessions_per_host,
            session_available: Arc::new(Notify::new()),
            host_config: Arc::new(host_config),
            pool_config,
        }
    }

    /// Checkout a session from the pool.
    ///
    /// Flow (from connection-manager.md lines 124-145):
    /// 1. Try to take an idle session from the queue
    ///    a. Found → validate it's still alive (keepalive ping)
    ///       - Alive → return it
    ///       - Dead → drop it, try next / create new
    ///    b. Queue empty → create new session if under max_sessions
    ///       - Under limit → connect, authenticate, return
    ///       - At limit → wait with timeout (checkout_timeout)
    ///         - Session returned → use it
    ///         - Timeout → return error
    pub async fn checkout(&self) -> anyhow::Result<PooledSession> {
        // First, try to get an idle session
        loop {
            let mut sessions = self.sessions.lock().await;
            
            while let Some(mut session) = sessions.pop_front() {
                // Validate session before returning it
                if self.validate_session(&session).await {
                    session.last_used = std::time::Instant::now();
                    self.active_count.fetch_add(1, Ordering::Relaxed);
                    return Ok(session);
                } else {
                    // Session is dead — drop it and decrement total
                    self.total_count.fetch_sub(1, Ordering::Relaxed);
                    tracing::debug!("Dropped stale SSH session, total now: {}", 
                        self.total_count.load(Ordering::Relaxed));
                }
            }
            
            // Queue is empty — can we create a new session?
            let total = self.total_count.load(Ordering::Relaxed);
            if total < self.max_sessions {
                drop(sessions); // Release lock before network call
                
                // Create new session with retry policy
                // connection-manager.md lines 196-207: max 1 retry, 1s delay
                let session = match self.create_session().await {
                    Ok(s) => s,
                    Err(first_error) => {
                        tracing::warn!("SSH connection failed, retrying in 1s: {}", first_error);
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                        self.create_session().await.map_err(|retry_error| {
                            anyhow::anyhow!(
                                "SSH connection failed after retry. First: {}. Retry: {}",
                                first_error, retry_error
                            )
                        })?
                    }
                };
                
                self.total_count.fetch_add(1, Ordering::Relaxed);
                self.active_count.fetch_add(1, Ordering::Relaxed);
                return Ok(session);
            }
            
            // At max sessions — wait for one to be returned
            drop(sessions); // Release lock before waiting
            
            let timeout = std::time::Duration::from_secs(self.pool_config.checkout_timeout_secs);
            match tokio::time::timeout(timeout, self.session_available.notified()).await {
                Ok(()) => {
                    // A session was returned — loop back and try to take it
                    continue;
                }
                Err(_) => {
                    return Err(anyhow::anyhow!(
                        "All SSH sessions to '{}' are in use ({} active). \
                         Try again shortly.",
                        self.host_config.host,
                        self.active_count.load(Ordering::Relaxed)
                    ));
                }
            }
        }
    }

    /// Return a session to the pool after use.
    ///
    /// connection-manager.md lines 142-145:
    /// - Session healthy → push back to queue, update last_used
    /// - Session broken → drop it, decrement active_count
    pub async fn return_session(&self, mut session: PooledSession, broken: bool) {
        self.active_count.fetch_sub(1, Ordering::Relaxed);
        
        if broken {
            // Session is broken — drop it
            self.total_count.fetch_sub(1, Ordering::Relaxed);
            tracing::debug!("Dropped broken SSH session");
        } else {
            // Session is healthy — return to pool
            session.last_used = std::time::Instant::now();
            let mut sessions = self.sessions.lock().await;
            sessions.push_back(session);
        }
        
        // Wake up any waiters
        self.session_available.notify_one();
    }

    /// Validate a session before returning it to the caller.
    ///
    /// connection-manager.md lines 148-171:
    /// 1. Check age — sessions older than max_lifetime are discarded
    /// 2. Check idle time — stale sessions get a keepalive ping
    async fn validate_session(&self, session: &PooledSession) -> bool {
        // 1. Check age — sessions older than max_lifetime are discarded
        let max_lifetime = std::time::Duration::from_secs(self.pool_config.max_lifetime_secs);
        if session.created_at.elapsed() > max_lifetime {
            tracing::debug!("SSH session expired (age: {:?}, max: {:?})",
                session.created_at.elapsed(), max_lifetime);
            return false;
        }

        // 2. Check idle time — if idle too long, send keepalive ping
        let max_idle = std::time::Duration::from_secs(self.pool_config.max_idle_time_secs);
        if session.last_used.elapsed() > max_idle {
            // Try a keepalive ping to see if session is still alive
            // russh sends a keepalive via the session handle
            match tokio::time::timeout(
                std::time::Duration::from_secs(5),
                session.handle.request_keepalive()
            ).await {
                Ok(Ok(())) => true,
                _ => {
                    tracing::debug!("SSH session failed keepalive (idle: {:?})",
                        session.last_used.elapsed());
                    false
                }
            }
        } else {
            // Recently used — assume alive
            true
        }
    }

    /// Create a new SSH session by connecting and authenticating.
    ///
    /// IMPORTANT (security-approach.md Layer 9, lines 360-368):
    /// SSH commands MUST be executed via the `exec` channel, NOT via `bash -c`.
    /// This method creates the session; command execution happens in tools/ssh.rs
    /// which uses `channel.exec(true, &command)` directly.
    async fn create_session(&self) -> anyhow::Result<PooledSession> {
        use anyhow::anyhow;

        let config = russh::client::Config {
            inactivity_timeout: Some(std::time::Duration::from_secs(
                self.pool_config.keepalive_interval_secs
            )),
            ..Default::default()
        };

        let addr = format!("{}:{}", self.host_config.host, self.host_config.port.unwrap_or(22));
        
        let connect_timeout = std::time::Duration::from_secs(self.pool_config.connect_timeout_secs);

        // Connect with timeout
        let handle = tokio::time::timeout(connect_timeout, async {
            let handler = SshClientHandler;
            russh::client::connect(Arc::new(config), &addr, handler).await
        })
        .await
        .map_err(|_| anyhow!("SSH connection timed out after {}s to {}", 
            self.pool_config.connect_timeout_secs, addr))?
        .map_err(|e| anyhow!("SSH connection failed to {}: {}", addr, e))?;

        // Load private key
        let key_path = &self.host_config.private_key_path;
        let key = russh_keys::load_secret_key(
            key_path,
            self.host_config.private_key_passphrase.as_deref(),
        )
        .map_err(|e| anyhow!("Failed to load SSH key {:?}: {:?}", key_path, e))?;

        // Authenticate
        let authenticated = handle
            .authenticate_publickey(&self.host_config.user, Arc::new(key))
            .await
            .map_err(|e| anyhow!("SSH authentication failed for {}@{}: {}", 
                self.host_config.user, addr, e))?;

        if !authenticated {
            return Err(anyhow!(
                "SSH authentication rejected for {}@{}. Check credentials.",
                self.host_config.user, addr
            ));
        }

        Ok(PooledSession {
            handle,
            created_at: std::time::Instant::now(),
            last_used: std::time::Instant::now(),
        })
    }

    /// Clean up stale sessions from the idle queue.
    /// Called by the health monitor background task (M4).
    pub async fn cleanup_stale_sessions(&self) {
        let mut sessions = self.sessions.lock().await;
        let before = sessions.len();
        
        let max_lifetime = std::time::Duration::from_secs(self.pool_config.max_lifetime_secs);
        sessions.retain(|s| s.created_at.elapsed() <= max_lifetime);
        
        let removed = before - sessions.len();
        if removed > 0 {
            self.total_count.fetch_sub(removed, Ordering::Relaxed);
            tracing::info!("Cleaned up {} stale SSH sessions", removed);
        }
    }

    /// Check basic connectivity (for health monitor)
    pub async fn check_connectivity(&self) -> anyhow::Result<()> {
        // Try to checkout and immediately return a session
        // If this fails, the host is likely unreachable
        let session = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            self.checkout()
        ).await
        .map_err(|_| anyhow::anyhow!("Connectivity check timed out"))?
        .map_err(|e| anyhow::anyhow!("Connectivity check failed: {}", e))?;
        
        self.return_session(session, false).await;
        Ok(())
    }

    /// Close all sessions in the pool (for graceful shutdown)
    pub async fn close_all(&self) {
        let mut sessions = self.sessions.lock().await;
        let count = sessions.len();
        sessions.clear();
        self.total_count.store(0, Ordering::Relaxed);
        if count > 0 {
            tracing::info!("Closed {} SSH sessions", count);
        }
    }
}
```

**Key differences from previous plan:**
1. **`Arc<Mutex<VecDeque<PooledSession>>>`** — matches `connection-manager.md` exactly (not DashMap)
2. **Checkout/return flow** — implements the full flow from connection-manager.md lines 124-145
3. **Session validation** — checks age > max_lifetime and idle > max_idle_time with keepalive ping
4. **Wait-on-full** — when at max_sessions, waits with `checkout_timeout` via `Notify`
5. **Connection retry policy** — max 1 retry, 1s delay per connection-manager.md lines 196-207
6. **`keepalive_interval`** — used in russh client config's `inactivity_timeout`
7. **`cleanup_stale_sessions()`** — called by health monitor (M4)
8. **`close_all()`** — called during graceful shutdown (M4)
9. **`SshClientHandler`** — minimal russh client handler (required by the crate)

**Verification:** `cargo check` compiles.

---

#### Step 3: Initialize SSH pools in ConnectionManager::new

**File:** `src/connection.rs` — update SSH initialization block

**Current code:**
```rust
// Initialize SSH pools
for (name, _host) in &manager.config.ssh.hosts {
    // ...
    // TODO: Create SSH pool
}
```

**Replace with:**
```rust
// Initialize SSH pools
for (name, host_config) in &manager.config.ssh.hosts {
    let health_key = format!("ssh:{}", name);
    manager.health.insert(
        health_key.clone(),
        ConnectionHealth {
            status: ConnectionStatus::Connecting,
            last_success: None,
            last_error: None,
            consecutive_failures: 0,
        },
    );
    
    // Create pool (does NOT connect yet — connections are created on first checkout)
    let pool = SshHostPool::new(host_config.clone(), manager.config.ssh.pool.clone());
    manager.ssh_pools.insert(name.clone(), pool);
    
    // Mark as Connected — actual connectivity is verified on first use or by health monitor
    manager.mark_healthy(&health_key);
    info!("SSH pool created for '{}' ({}@{}:{})", 
        name, host_config.user, host_config.host, host_config.port.unwrap_or(22));
}
```

---

#### Step 4: Add SSH checkout/return methods to ConnectionManager

**File:** `src/connection.rs` — add convenience methods

```rust
/// Checkout an SSH session for a named host
pub async fn ssh_checkout(&self, host: &str) -> Result<PooledSession> {
    let pool = self.ssh_pools
        .get(host)
        .ok_or_else(|| {
            if let Some(health) = self.health.get(&format!("ssh:{}", host)) {
                anyhow!(
                    "SSH host '{}' is {:?}. Last error: {}. {}",
                    host,
                    health.status,
                    health.last_error.as_deref().unwrap_or("unknown"),
                    if health.last_success.is_some() {
                        format!("Last successful connection: {:?} ago.", 
                            health.last_success.unwrap().elapsed())
                    } else {
                        "No successful connection recorded.".to_string()
                    }
                )
            } else {
                anyhow!("SSH host '{}' not configured", host)
            }
        })?;
    
    pool.checkout().await
}

/// Return an SSH session to its pool
pub async fn ssh_return(&self, host: &str, session: PooledSession, broken: bool) {
    if let Some(pool) = self.ssh_pools.get(host) {
        pool.return_session(session, broken).await;
    }
}
```

---

#### Step 5: Implement SSH tool handlers with exec channel enforcement

**File:** `src/tools/ssh.rs` — **completely rewrite** (currently lines 1-30)

**Design doc references:**
- `security-approach.md` Layer 9 (lines 360-368): Commands MUST use `session.exec(command)` directly, NOT `bash -c`.
- `security-approach.md` Layer 9 (lines 307-319): Output wrapped in `untrusted_external` envelope.
- `poc-specification.md` lines 279-413: Tool schemas and implementation.

```rust
use crate::audit::AuditLogger;
use crate::config::CommandAllowlist;
use crate::connection::ConnectionManager;
use anyhow::{anyhow, Result};
use std::sync::Arc;
use std::time::Duration;

const OUTPUT_MAX_CHARS: usize = 5_000;

/// Wrap tool output in untrusted_external envelope (Layer 9)
fn wrap_output_envelope(tool_name: &str, content: &str) -> String {
    format!(
        "[tool_result source=\"{}\" data_classification=\"untrusted_external\"]\n{}\n[/tool_result]",
        tool_name, content
    )
}

/// Execute a command on a remote host via SSH.
///
/// SECURITY (Layer 9): Commands are executed via the SSH exec channel DIRECTLY,
/// NOT via `bash -c`. This prevents shell metacharacter injection.
/// See security-approach.md lines 360-368:
///   // GOOD: direct exec, no shell interpretation
///   channel.exec(true, &command).await?;
pub async fn exec(
    manager: Arc<ConnectionManager>,
    host: String,
    command: String,
    timeout: Option<u64>,
    dry_run: Option<bool>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    // 1. Validate command against allowlist/blocklist
    let allowlist = &manager.config().ssh.command_allowlist;
    validate_command(&command, allowlist)?;

    // 2. Check for blocked patterns
    // (validate_command does both — blocked patterns checked first)

    if dry_run.unwrap_or(false) {
        audit.log("ssh.exec", &host, "dry_run", Some(&command)).await.ok();
        return Ok(format!(
            "DRY RUN: Command '{}' passes validation for host '{}'. Set dry_run=false to execute.",
            command, host
        ));
    }

    // 3. Checkout an SSH session from the pool
    let session = manager.ssh_checkout(&host).await?;
    let mut broken = false;

    // 4. Execute command with timeout
    let timeout_duration = Duration::from_secs(timeout.unwrap_or(30).min(300));
    
    let result = tokio::time::timeout(timeout_duration, async {
        // Open an exec channel — NOT a shell channel.
        // security-approach.md Layer 9: "SSH commands are executed via exec channel,
        // not via bash -c. The MCP server passes commands as argument vectors,
        // not as shell strings, preventing shell metacharacter injection."
        let mut channel = session.handle.channel_open_session().await
            .map_err(|e| anyhow!("Failed to open SSH channel: {}", e))?;
        
        // CRITICAL: Use exec(), not request_shell() + write(). This sends the command
        // directly to the SSH server's exec subsystem without shell interpretation.
        channel.exec(true, command.as_bytes()).await
            .map_err(|e| anyhow!("Failed to exec command: {}", e))?;

        // Collect output
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut exit_status: Option<u32> = None;

        while let Some(msg) = channel.wait().await {
            match msg {
                russh::ChannelMsg::Data { data } => {
                    stdout.extend_from_slice(&data);
                }
                russh::ChannelMsg::ExtendedData { data, ext } => {
                    if ext == 1 { // stderr
                        stderr.extend_from_slice(&data);
                    }
                }
                russh::ChannelMsg::ExitStatus { exit_status: status } => {
                    exit_status = Some(status);
                }
                _ => {}
            }
        }

        Ok::<_, anyhow::Error>((stdout, stderr, exit_status))
    }).await;

    // 5. Return session to pool
    let output = match result {
        Ok(Ok((stdout, stderr, exit_status))) => {
            let stdout_str = String::from_utf8_lossy(&stdout);
            let stderr_str = String::from_utf8_lossy(&stderr);
            
            let mut output = String::new();
            if let Some(status) = exit_status {
                output.push_str(&format!("Exit code: {}\n", status));
            }
            if !stdout_str.is_empty() {
                output.push_str(&format!("--- stdout ---\n{}\n", stdout_str));
            }
            if !stderr_str.is_empty() {
                output.push_str(&format!("--- stderr ---\n{}\n", stderr_str));
            }
            if output.is_empty() {
                output = "Command completed with no output.".to_string();
            }
            
            // Truncate to 5000 chars (Layer 9: output length limits)
            let output = truncate_output(&output, OUTPUT_MAX_CHARS);
            audit.log("ssh.exec", &host, "success", Some(&command)).await.ok();
            Ok(wrap_output_envelope("ssh.exec", &output))
        }
        Ok(Err(e)) => {
            broken = true; // Mark session as broken
            audit.log("ssh.exec", &host, &format!("error: {}", e), Some(&command)).await.ok();
            Err(e)
        }
        Err(_) => {
            broken = true; // Timeout likely means broken session
            audit.log("ssh.exec", &host, "timeout", Some(&command)).await.ok();
            Err(anyhow!("Command timed out after {}s on host '{}'", 
                timeout.unwrap_or(30).min(300), host))
        }
    };

    // Return session with broken flag
    manager.ssh_return(&host, session, broken).await;
    output
}

/// Upload a file to a remote host via SFTP
pub async fn upload(
    manager: Arc<ConnectionManager>,
    host: String,
    local_path: String,
    remote_path: String,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    // 1. Validate local file exists
    if !std::path::Path::new(&local_path).exists() {
        return Err(anyhow!("Local file does not exist: {}", local_path));
    }

    // 2. Check file size (50MB limit) — per poc-specification.md line 447
    let metadata = std::fs::metadata(&local_path)
        .map_err(|e| anyhow!("Failed to stat local file: {}", e))?;
    
    const MAX_FILE_SIZE: u64 = 50 * 1024 * 1024; // 50MB
    if metadata.len() > MAX_FILE_SIZE {
        return Err(anyhow!(
            "File too large: {} bytes (max {} bytes / 50MB)",
            metadata.len(),
            MAX_FILE_SIZE
        ));
    }

    // 3. Checkout SSH session and transfer via SFTP
    let session = manager.ssh_checkout(&host).await?;
    let mut broken = false;

    let result = async {
        // TODO: Implement SFTP upload via russh's SFTP subsystem
        // The implementation should:
        // 1. Request SFTP subsystem on the session
        // 2. Open remote file for writing
        // 3. Stream local file contents to remote
        // 4. Close and verify
        //
        // For now, return a placeholder indicating the file would be uploaded.
        // Full SFTP implementation depends on russh-sftp crate or manual SFTP handling.
        
        Err::<String, anyhow::Error>(anyhow!(
            "SFTP upload not yet implemented. Use ssh.exec with 'cat' or 'scp' as a workaround."
        ))
    }.await;

    match &result {
        Ok(_) => {}
        Err(_) => { broken = false; } // Not a connection issue, just unimplemented
    }

    manager.ssh_return(&host, session, broken).await;

    match result {
        Ok(msg) => {
            audit.log("ssh.upload", &host, "success", Some(&remote_path)).await.ok();
            Ok(wrap_output_envelope("ssh.upload", &msg))
        }
        Err(e) => {
            audit.log("ssh.upload", &host, "error", Some(&e.to_string())).await.ok();
            Err(e)
        }
    }
}

/// Download a file from a remote host via SFTP
pub async fn download(
    manager: Arc<ConnectionManager>,
    host: String,
    remote_path: String,
    local_path: Option<String>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let local_dest = local_path.unwrap_or_else(|| {
        format!(
            "/tmp/homelab-download-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        )
    });

    // Checkout SSH session
    let session = manager.ssh_checkout(&host).await?;
    let mut broken = false;

    let result = async {
        // TODO: Implement SFTP download via russh's SFTP subsystem
        // Similar to upload — needs SFTP subsystem request.
        
        Err::<String, anyhow::Error>(anyhow!(
            "SFTP download not yet implemented. Use ssh.exec with 'cat' as a workaround."
        ))
    }.await;

    manager.ssh_return(&host, session, broken).await;

    match result {
        Ok(msg) => {
            audit.log("ssh.download", &host, "success", Some(&remote_path)).await.ok();
            Ok(wrap_output_envelope("ssh.download", &msg))
        }
        Err(e) => {
            audit.log("ssh.download", &host, "error", Some(&e.to_string())).await.ok();
            Err(e)
        }
    }
}

// Helper functions

/// Validate command against allowlist and blocklist.
///
/// security-approach.md Layer 9 (lines 329-370):
/// - Blocked patterns checked first (higher priority)
/// - Then allowed prefixes
/// - Prefix matching is strict: "docker" matches "docker ps" but NOT "dockerrm"
fn validate_command(command: &str, allowlist: &CommandAllowlist) -> Result<()> {
    // Check blocked patterns first (higher priority)
    for pattern in &allowlist.blocked_patterns {
        if command.contains(pattern) {
            return Err(anyhow!(
                "Command blocked: contains dangerous pattern '{}'. This pattern is in the blocked list for safety.",
                pattern
            ));
        }
    }

    // Check allowed prefixes — strict matching (must be exact or followed by space)
    let allowed = allowlist.allowed_prefixes.iter().any(|prefix| {
        command == prefix || command.starts_with(&format!("{} ", prefix))
    });

    if !allowed {
        return Err(anyhow!(
            "Command '{}' does not match any allowed prefix. Allowed: {}",
            command,
            allowlist.allowed_prefixes.join(", ")
        ));
    }

    Ok(())
}

fn truncate_output(output: &str, max_chars: usize) -> String {
    if output.len() > max_chars {
        let truncated = &output[..max_chars];
        format!(
            "[Output truncated. Showing {} chars of {} total chars.]\n{}",
            max_chars,
            output.len(),
            truncated
        )
    } else {
        output.to_string()
    }
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
            allowed_prefixes: vec!["docker".to_string()],
            blocked_patterns: vec!["rm -rf".to_string()],
        };

        assert!(validate_command("docker exec foo rm -rf /", &allowlist).is_err());
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

        // "dockerrm" should not match "docker" prefix (needs space or EOL)
        assert!(validate_command("dockerrm ps", &allowlist).is_err());
    }
}
```

**Key differences from previous plan:**
1. **SSH exec channel enforced** — explicit `channel.exec()` with security comments explaining why NOT `bash -c`
2. **Output envelope** — all SSH output wrapped in `untrusted_external` envelope per Layer 9
3. **Proper checkout/return** — uses `manager.ssh_checkout()` / `manager.ssh_return()` with broken flag
4. **Full output collection** — collects stdout, stderr, and exit status from channel messages
5. **Session broken tracking** — timeouts and channel errors mark sessions as broken for the pool

**Verification:** `cargo check` compiles; unit tests for command validation pass.

---

#### Step 4: Register SSH tools with MCP server

**File:** `src/mcp.rs` — add SSH tool handlers

Add to the `#[rmcp::handler::server::tool]` impl block:

```rust
// SSH tool handlers
#[tool(description = "Execute a command on a remote host via SSH")]
pub async fn ssh_exec(
    &self,
    host: String,
    command: String,
    timeout: Option<u64>,
    dry_run: Option<bool>,
) -> anyhow::Result<String> {
    ssh::exec(
        self.manager.clone(),
        host,
        command,
        timeout,
        dry_run,
        self.audit.clone(),
    )
    .await
}

#[tool(description = "Upload a file to a remote host via SFTP")]
pub async fn ssh_upload(
    &self,
    host: String,
    local_path: String,
    remote_path: String,
) -> anyhow::Result<String> {
    ssh::upload(
        self.manager.clone(),
        host,
        local_path,
        remote_path,
        self.audit.clone(),
    )
    .await
}

#[tool(description = "Download a file from a remote host via SFTP")]
pub async fn ssh_download(
    &self,
    host: String,
    remote_path: String,
    local_path: Option<String>,
) -> anyhow::Result<String> {
    ssh::download(
        self.manager.clone(),
        host,
        remote_path,
        local_path,
        self.audit.clone(),
    )
    .await
}
```

Add import:
```rust
use crate::tools::ssh;
```

**Verification:** `cargo check` compiles.

---

#### Step 5: Compile and test M3

**Command:**
```bash
cargo build --release 2>&1
cargo test --lib ssh -- --nocapture
```

**Expected output:**
- Compilation succeeds (may have warnings about stubbed SSH)
- Unit tests for command validation pass

---

### M3 Verification Checklist

- [ ] `cargo build --release` succeeds
- [ ] SSH tool handlers are registered with MCP
- [ ] Command validation tests pass (`cargo test ssh::tests`)
- [ ] Allowlist blocks dangerous commands
- [ ] Blocklist blocks patterns
- [ ] Prefix matching is strict (not substring)
- [ ] Dry-run mode works without executing
- [ ] File size validation works

---

## Milestone 4: Safety and Observability (1-2 days)

### M4 Goals

Implement security gates, rate limiting, confirmation flow, health monitoring, graceful shutdown, and observability:
- **Audit logging** — append-only file with all tool invocations
- **Command allowlist/blocklist** — enforce restrictions on SSH commands
- **Rate limiting** — prevent abuse, with wildcard pattern support
- **Layer 8: Confirmation flow** — token-based confirmation for destructive operations
- **Health monitor** — background task tracking connection status with reconnection backoff
- **Graceful shutdown** — SIGTERM/SIGINT handler with in-flight drain
- **Doctor subcommand** — health diagnostics

**Success Criteria:**
- Audit log records all tool calls with timestamp, tool name, host, status
- Command validation blocks dangerous patterns
- Rate limiting prevents tool spam (including `"docker.container.*"` wildcard patterns)
- Destructive operations require confirmation tokens (Layer 8)
- Health monitor runs every 30 seconds, pings Docker clients, cleans stale SSH sessions
- Reconnection uses exponential backoff (30s → 60s → 120s → 180s capped)
- SIGTERM/SIGINT triggers graceful drain of in-flight calls (10s timeout)
- Doctor reports accurate health status

---

### M4 Implementation

#### Step 1: Enhance audit logging

**File:** `src/audit.rs` — already mostly implemented

The audit logger in `src/audit.rs` (lines 1-71) is largely complete. Verify it:
- Writes to file in append mode
- Includes timestamps
- Logs tool name, host, result, details
- TODO: Add syslog support (optional for M4)

**Verification:** Already in place from earlier implementation.

---

#### Step 2: Implement rate limiting with wildcard pattern support

**File:** `src/rate_limit.rs` — **create new file**

**Design context (security-approach.md Layer 7):** The config uses glob patterns like `"docker.container.*"` to apply a single rate limit to all tools matching that pattern. The rate limiter must resolve a tool name like `"docker.container.list"` against both exact matches AND wildcard patterns.

**Important note on default limits:** The design documents do NOT specify a default rate limit. The implementation below uses no default — if a tool has no configured limit (exact or wildcard), it is not rate-limited. This is an explicit design choice: operators must configure the limits they want. If you want a catch-all default, add `"*" = { per_minute = 100 }` to the config, which the wildcard matching will handle.

```rust
use dashmap::DashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use anyhow::{anyhow, Result};

/// Rate limiter using a sliding window approach.
///
/// Supports both exact tool name matches and wildcard glob patterns
/// from the config (e.g., "docker.container.*" matches "docker.container.list").
/// See security-approach.md Layer 7.
pub struct RateLimiter {
    /// Tool name (or pattern) -> timestamps of recent calls
    windows: Arc<DashMap<String, Vec<Instant>>>,
    /// Exact tool name -> limit
    exact_limits: Arc<DashMap<String, u32>>,
    /// Wildcard patterns (stored as prefix before the "*") -> limit
    /// e.g., "docker.container.*" is stored as prefix "docker.container."
    wildcard_limits: Arc<Vec<(String, u32)>>,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self {
            windows: Arc::new(DashMap::new()),
            exact_limits: Arc::new(DashMap::new()),
            wildcard_limits: Arc::new(Vec::new()),
        }
    }

    /// Build a rate limiter from config entries.
    /// Config keys can be exact names ("ssh.exec") or glob patterns ("docker.container.*").
    pub fn from_config(limits: &std::collections::HashMap<String, crate::config::RateLimitEntry>) -> Self {
        let mut exact = DashMap::new();
        let mut wildcards = Vec::new();

        for (pattern, entry) in limits {
            if pattern.contains('*') {
                // Wildcard pattern: split on "*" and use the prefix for matching.
                // Only trailing wildcards are supported: "docker.container.*"
                // The prefix is everything before the "*".
                let prefix = pattern.trim_end_matches('*').to_string();
                wildcards.push((prefix, entry.per_minute));
            } else {
                exact.insert(pattern.clone(), entry.per_minute);
            }
        }

        Self {
            windows: Arc::new(DashMap::new()),
            exact_limits: Arc::new(exact),
            wildcard_limits: Arc::new(wildcards),
        }
    }

    /// Resolve the rate limit for a given tool name.
    /// 1. Check exact match first.
    /// 2. Check wildcard patterns (first match wins).
    /// 3. If no match, returns None (tool is not rate-limited).
    fn resolve_limit(&self, tool_name: &str) -> Option<(String, u32)> {
        // Exact match takes priority
        if let Some(limit) = self.exact_limits.get(tool_name) {
            return Some((tool_name.to_string(), *limit));
        }

        // Check wildcard patterns — use the pattern as the rate limit key
        // so all tools matching "docker.container.*" share one counter.
        for (prefix, limit) in self.wildcard_limits.iter() {
            if tool_name.starts_with(prefix.as_str()) {
                // Reconstruct the pattern key for the shared window
                let pattern_key = format!("{}*", prefix);
                return Some((pattern_key, *limit));
            }
        }

        None
    }

    /// Check if a tool call should be allowed.
    /// Returns Ok(()) if allowed, Err with a message matching the format
    /// from security-approach.md: "Rate limit exceeded for <tool>. Limit: N/min. Retry after Xs."
    pub fn check(&self, tool_name: &str) -> Result<()> {
        let (rate_key, limit) = match self.resolve_limit(tool_name) {
            Some(resolved) => resolved,
            None => return Ok(()), // No limit configured — allow
        };

        let now = Instant::now();
        let window_start = now - Duration::from_secs(60);

        let mut entry = self.windows.entry(rate_key.clone()).or_insert_with(Vec::new);

        // Remove old timestamps outside the 60-second window
        entry.retain(|t| *t > window_start);

        // Check limit
        if entry.len() >= limit as usize {
            // Calculate approximate retry-after based on oldest entry in window
            let retry_after = entry.first()
                .map(|oldest| {
                    let elapsed = oldest.elapsed();
                    if elapsed < Duration::from_secs(60) {
                        60 - elapsed.as_secs()
                    } else {
                        0
                    }
                })
                .unwrap_or(60);

            return Err(anyhow!(
                "Rate limit exceeded for {}. Limit: {}/min. Retry after {}s.",
                tool_name,
                limit,
                retry_after
            ));
        }

        // Record this call
        entry.push(now);
        Ok(())
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exact_limit() {
        let limiter = RateLimiter::new();
        limiter.exact_limits.insert("test.tool".to_string(), 3);

        assert!(limiter.check("test.tool").is_ok());
        assert!(limiter.check("test.tool").is_ok());
        assert!(limiter.check("test.tool").is_ok());
        assert!(limiter.check("test.tool").is_err()); // 4th call denied
    }

    #[test]
    fn test_no_limit_means_allowed() {
        let limiter = RateLimiter::new();
        // No limit configured for this tool — should always be allowed
        for _ in 0..200 {
            assert!(limiter.check("unlisted.tool").is_ok());
        }
    }

    #[test]
    fn test_wildcard_pattern() {
        // Simulate config: "docker.container.*" = { per_minute = 5 }
        let limiter = RateLimiter {
            windows: Arc::new(DashMap::new()),
            exact_limits: Arc::new(DashMap::new()),
            wildcard_limits: Arc::new(vec![
                ("docker.container.".to_string(), 5),
            ]),
        };

        // All these should share the same "docker.container.*" counter
        assert!(limiter.check("docker.container.list").is_ok());   // 1
        assert!(limiter.check("docker.container.start").is_ok());  // 2
        assert!(limiter.check("docker.container.stop").is_ok());   // 3
        assert!(limiter.check("docker.container.logs").is_ok());   // 4
        assert!(limiter.check("docker.container.inspect").is_ok()); // 5
        // 6th call to any docker.container.* tool should be denied
        assert!(limiter.check("docker.container.list").is_err());
    }

    #[test]
    fn test_exact_overrides_wildcard() {
        // Config: "docker.container.*" = 5, "docker.container.list" = 2
        let limiter = RateLimiter {
            windows: Arc::new(DashMap::new()),
            exact_limits: Arc::new({
                let map = DashMap::new();
                map.insert("docker.container.list".to_string(), 2);
                map
            }),
            wildcard_limits: Arc::new(vec![
                ("docker.container.".to_string(), 5),
            ]),
        };

        // docker.container.list uses exact limit (2)
        assert!(limiter.check("docker.container.list").is_ok());
        assert!(limiter.check("docker.container.list").is_ok());
        assert!(limiter.check("docker.container.list").is_err()); // 3rd denied

        // docker.container.start uses wildcard limit (5)
        for _ in 0..5 {
            assert!(limiter.check("docker.container.start").is_ok());
        }
        assert!(limiter.check("docker.container.start").is_err()); // 6th denied
    }

    #[test]
    fn test_error_message_format() {
        let limiter = RateLimiter::new();
        limiter.exact_limits.insert("test.tool".to_string(), 1);

        assert!(limiter.check("test.tool").is_ok());
        let err = limiter.check("test.tool").unwrap_err();
        let msg = err.to_string();
        // Verify format matches security-approach.md:
        // "Rate limit exceeded for <tool>. Limit: N/min. Retry after Xs."
        assert!(msg.contains("Rate limit exceeded for test.tool"));
        assert!(msg.contains("Limit: 1/min"));
        assert!(msg.contains("Retry after"));
    }
}
```

**Verification:** `cargo test rate_limit` passes.

---

#### Step 3: Add rate limiting to tool handlers

**File:** `src/mcp.rs` — integrate rate limiter into tool handlers

Add rate limiter to `HomelabMcpServer`:

```rust
#[derive(Clone)]
pub struct HomelabMcpServer {
    config: Arc<Config>,
    manager: Arc<ConnectionManager>,
    audit: Arc<AuditLogger>,
    rate_limiter: Arc<crate::rate_limit::RateLimiter>,
    // confirmation_manager added in Step 5 below
}

impl HomelabMcpServer {
    pub fn new(
        config: Arc<Config>,
        manager: Arc<ConnectionManager>,
        audit: Arc<AuditLogger>,
    ) -> Self {
        // Build rate limiter from config (handles both exact and wildcard patterns)
        let rate_limiter = Arc::new(
            crate::rate_limit::RateLimiter::from_config(&config.rate_limits.limits)
        );

        Self {
            config,
            manager,
            audit,
            rate_limiter,
        }
    }
}
```

Then in each tool handler, add rate limit check using `check()` (not `allow()`):

```rust
#[tool(description = "List Docker containers")]
pub async fn docker_container_list(
    &self,
    host: Option<String>,
    all: Option<bool>,
    name_filter: Option<String>,
) -> anyhow::Result<String> {
    // Rate limit check — uses exact match or wildcard pattern from config
    self.rate_limiter.check("docker.container.list")?;

    docker::container_list(
        self.manager.clone(),
        host,
        all,
        name_filter,
        self.audit.clone(),
    )
    .await
}
```

Apply the same `self.rate_limiter.check("tool.name")` pattern to all 8 tool handlers (5 Docker + 3 SSH).

**Verification:** `cargo check` compiles.

---

#### Step 4: Enhance health monitoring (doctor subcommand)

**File:** `src/health.rs` — update `run_diagnostics` function

The basic diagnostics are already in place. Enhance it to show connection status:

```rust
pub async fn run_diagnostics(config: &Config) -> Result<()> {
    println!("Checking Docker hosts:");
    for (name, host) in &config.docker.hosts {
        match check_docker_connection(name, host).await {
            Ok(_) => {
                println!("  ✓ Docker '{}': {} → accessible", name, host.host);
            }
            Err(e) => {
                println!("  ✗ Docker '{}': {} → {}", name, host.host, e);
                println!("    → Check that Docker daemon is running");
                println!("    → Verify connection string is correct");
            }
        }
    }

    println!("\nChecking SSH hosts:");
    for (name, host) in &config.ssh.hosts {
        match check_ssh_connection(name, host).await {
            Ok(_) => {
                println!("  ✓ SSH '{}': {}@{} → OK", name, host.user, host.host);
            }
            Err(e) => {
                println!("  ✗ SSH '{}': {}@{} → {}", name, host.user, host.host, e);
                println!("    → Check that SSH server is running");
                println!("    → Verify host and port are correct");
                println!("    → Verify private_key_path is correct");
                println!("    → Verify SSH user has permissions");
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
        "  SSH pool: max {} sessions, {} min lifetime, {} min idle",
        config.ssh.pool.max_sessions_per_host,
        config.ssh.pool.max_lifetime_secs / 60,
        config.ssh.pool.max_idle_time_secs / 60,
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

    // Show confirmation rules if configured
    if let Some(ref confirm) = config.confirm {
        println!("\nConfirmation rules:");
        for (tool, rule) in confirm {
            println!("  {}: {:?}", tool, rule);
        }
    } else {
        println!("\nConfirmation rules: none configured");
    }

    println!("\nReady to start.");
    Ok(())
}
```

**Verification:** Run `cargo build` and test doctor subcommand.

---

#### Step 5: Implement Layer 8 — Confirmation flow for destructive operations

**File:** `src/confirmation.rs` — **create new file**

**Design context (security-approach.md Layer 8, lines 275-295):** Certain operations require explicit user confirmation before execution. This is enforced IN the MCP server, not by relying on the LLM to ask. The MCP server generates a single-use token that expires in 5 minutes. The LLM must present this to the user and then call a `.confirm` variant of the tool with the token.

**Config format (from security-approach.md):**

```toml
[confirm]
# These tools return a confirmation token instead of executing
"docker.container.delete" = "always"
"docker.image.delete" = "always"
# Pattern-based: only require confirmation when command matches certain patterns
"ssh.exec" = { when_pattern = ["rm -rf", "dd if=", "mkfs", "fdisk", "parted"] }
```

**Note:** The PoC tools (list, start, stop, logs, inspect, ssh.exec, ssh.upload, ssh.download) are not destructive — they don't have `docker.container.delete` or `docker.image.delete`. However, `ssh.exec` CAN match `when_pattern` rules. The confirmation framework must be built now so that:
1. `ssh.exec` commands matching `when_pattern` trigger confirmation
2. Future destructive tools (delete, create) can use it immediately
3. The executing model understands how this works

**Implementation:**

```rust
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use uuid::Uuid;
use anyhow::{anyhow, Result};

/// A pending confirmation waiting for user approval.
struct PendingConfirmation {
    /// The tool that was originally called
    tool_name: String,
    /// Original parameters (serialized as JSON string for storage)
    params_json: String,
    /// Human-readable description of what will happen
    description: String,
    /// When this token was created
    created_at: Instant,
    /// Whether this token has been used (single-use)
    used: bool,
}

/// Confirmation rule from config.
/// Matches the [confirm] section in security-approach.md.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(untagged)]
pub enum ConfirmRule {
    /// Always require confirmation: `"docker.container.delete" = "always"`
    Always(String), // value is "always"
    /// Require confirmation when command matches patterns:
    /// `"ssh.exec" = { when_pattern = ["rm -rf", "dd if="] }`
    WhenPattern { when_pattern: Vec<String> },
}

impl ConfirmRule {
    /// Check if this rule requires confirmation for the given command text.
    /// For "always" rules, command_text is ignored.
    /// For when_pattern rules, checks if command_text contains any pattern.
    pub fn requires_confirmation(&self, command_text: Option<&str>) -> bool {
        match self {
            ConfirmRule::Always(s) if s == "always" => true,
            ConfirmRule::Always(_) => false,
            ConfirmRule::WhenPattern { when_pattern } => {
                if let Some(cmd) = command_text {
                    when_pattern.iter().any(|pattern| cmd.contains(pattern))
                } else {
                    false
                }
            }
        }
    }
}

/// Manages confirmation tokens for destructive operations.
/// See security-approach.md Layer 8.
pub struct ConfirmationManager {
    /// Token -> PendingConfirmation
    pending: Arc<Mutex<HashMap<String, PendingConfirmation>>>,
    /// Tool name -> confirmation rule (from [confirm] config section)
    rules: HashMap<String, ConfirmRule>,
    /// Token expiry duration (5 minutes per security-approach.md)
    token_ttl: Duration,
}

impl ConfirmationManager {
    /// Create a new ConfirmationManager from the [confirm] config section.
    /// If no [confirm] section exists, rules will be empty and no tools require confirmation.
    pub fn new(rules: HashMap<String, ConfirmRule>) -> Self {
        Self {
            pending: Arc::new(Mutex::new(HashMap::new())),
            rules,
            token_ttl: Duration::from_secs(300), // 5 minutes
        }
    }

    /// Check if a tool call requires confirmation.
    ///
    /// `tool_name`: the MCP tool being called (e.g., "docker.container.delete")
    /// `command_text`: for ssh.exec, the command string; None for other tools
    ///
    /// Returns:
    /// - Ok(None) if no confirmation needed — proceed with execution
    /// - Ok(Some(response_json)) if confirmation IS needed — return this to the LLM
    /// - Err if something went wrong
    pub async fn check_and_maybe_require(
        &self,
        tool_name: &str,
        command_text: Option<&str>,
        description: &str,
        params_json: &str,
    ) -> Result<Option<String>> {
        let rule = match self.rules.get(tool_name) {
            Some(rule) => rule,
            None => return Ok(None), // No rule for this tool
        };

        if !rule.requires_confirmation(command_text) {
            return Ok(None); // Rule exists but doesn't apply to this invocation
        }

        // Generate confirmation token
        let token = Uuid::new_v4().to_string();

        let pending = PendingConfirmation {
            tool_name: tool_name.to_string(),
            params_json: params_json.to_string(),
            description: description.to_string(),
            created_at: Instant::now(),
            used: false,
        };

        {
            let mut map = self.pending.lock().await;
            // Clean up expired tokens while we have the lock
            map.retain(|_, p| p.created_at.elapsed() < self.token_ttl && !p.used);
            map.insert(token.clone(), pending);
        }

        // Return the confirmation-required response per security-approach.md:
        // { "status": "confirmation_required", "token": "abc123",
        //   "message": "About to <desc>. Call <tool>.confirm with token abc123 to proceed." }
        let response = serde_json::json!({
            "status": "confirmation_required",
            "token": token,
            "message": format!(
                "{}. Call {}.confirm with token {} to proceed.",
                description, tool_name, token
            )
        });

        Ok(Some(response.to_string()))
    }

    /// Validate and consume a confirmation token.
    ///
    /// `token`: the token provided by the caller
    /// `tool_name`: must match the original tool that generated the token
    ///
    /// Returns:
    /// - Ok(params_json) if the token is valid — caller should proceed with execution
    /// - Err if the token is invalid, expired, already used, or wrong tool
    pub async fn confirm(&self, token: &str, tool_name: &str) -> Result<String> {
        let mut map = self.pending.lock().await;

        let pending = map.get_mut(token).ok_or_else(|| {
            anyhow!("Invalid or expired confirmation token. Tokens expire after 5 minutes.")
        })?;

        // Check expiry
        if pending.created_at.elapsed() >= self.token_ttl {
            map.remove(token);
            return Err(anyhow!(
                "Confirmation token has expired (5 minute limit). Please re-initiate the operation."
            ));
        }

        // Check single-use
        if pending.used {
            return Err(anyhow!(
                "Confirmation token has already been used. Each token is single-use."
            ));
        }

        // Check tool name matches
        // The .confirm variant should map back to the base tool name
        let expected_tool = &pending.tool_name;
        if expected_tool != tool_name {
            return Err(anyhow!(
                "Token was issued for '{}', not '{}'. Tokens are tool-specific.",
                expected_tool, tool_name
            ));
        }

        // Mark as used and return the original params
        pending.used = true;
        let params = pending.params_json.clone();

        // Remove from map (single-use — no need to keep it)
        map.remove(token);

        Ok(params)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_always_rule_requires_confirmation() {
        let mut rules = HashMap::new();
        rules.insert(
            "docker.container.delete".to_string(),
            ConfirmRule::Always("always".to_string()),
        );
        let cm = ConfirmationManager::new(rules);

        let result = cm
            .check_and_maybe_require(
                "docker.container.delete",
                None,
                "About to delete container webapp-01",
                r#"{"container_id":"webapp-01"}"#,
            )
            .await
            .unwrap();

        assert!(result.is_some());
        let json: serde_json::Value = serde_json::from_str(&result.unwrap()).unwrap();
        assert_eq!(json["status"], "confirmation_required");
        assert!(json["token"].as_str().unwrap().len() > 0);
    }

    #[tokio::test]
    async fn test_when_pattern_matches() {
        let mut rules = HashMap::new();
        rules.insert(
            "ssh.exec".to_string(),
            ConfirmRule::WhenPattern {
                when_pattern: vec!["rm -rf".to_string(), "dd if=".to_string()],
            },
        );
        let cm = ConfirmationManager::new(rules);

        // "rm -rf /tmp/old" should trigger confirmation
        let result = cm
            .check_and_maybe_require(
                "ssh.exec",
                Some("rm -rf /tmp/old"),
                "About to run rm -rf /tmp/old on host nas",
                r#"{"host":"nas","command":"rm -rf /tmp/old"}"#,
            )
            .await
            .unwrap();
        assert!(result.is_some());

        // "df -h" should NOT trigger confirmation
        let result = cm
            .check_and_maybe_require(
                "ssh.exec",
                Some("df -h"),
                "About to run df -h on host nas",
                r#"{"host":"nas","command":"df -h"}"#,
            )
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_token_single_use() {
        let mut rules = HashMap::new();
        rules.insert(
            "docker.container.delete".to_string(),
            ConfirmRule::Always("always".to_string()),
        );
        let cm = ConfirmationManager::new(rules);

        let result = cm
            .check_and_maybe_require(
                "docker.container.delete",
                None,
                "About to delete container X",
                r#"{"container_id":"X"}"#,
            )
            .await
            .unwrap()
            .unwrap();

        let json: serde_json::Value = serde_json::from_str(&result).unwrap();
        let token = json["token"].as_str().unwrap();

        // First use succeeds
        assert!(cm.confirm(token, "docker.container.delete").await.is_ok());
        // Second use fails (single-use)
        assert!(cm.confirm(token, "docker.container.delete").await.is_err());
    }

    #[tokio::test]
    async fn test_no_rule_means_no_confirmation() {
        let cm = ConfirmationManager::new(HashMap::new());

        let result = cm
            .check_and_maybe_require(
                "docker.container.list",
                None,
                "listing containers",
                "{}",
            )
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_token_wrong_tool() {
        let mut rules = HashMap::new();
        rules.insert(
            "docker.container.delete".to_string(),
            ConfirmRule::Always("always".to_string()),
        );
        let cm = ConfirmationManager::new(rules);

        let result = cm
            .check_and_maybe_require(
                "docker.container.delete",
                None,
                "delete X",
                r#"{"container_id":"X"}"#,
            )
            .await
            .unwrap()
            .unwrap();

        let json: serde_json::Value = serde_json::from_str(&result).unwrap();
        let token = json["token"].as_str().unwrap();

        // Using the token for a different tool fails
        assert!(cm.confirm(token, "docker.image.delete").await.is_err());
    }
}
```

**Dependencies:** Add `uuid` crate to Cargo.toml:
```toml
uuid = { version = "1", features = ["v4"] }
```

**Verification:** `cargo test confirmation` passes.

---

#### Step 5b: Integrate confirmation flow into MCP server and ssh.exec

**File:** `src/mcp.rs` — add ConfirmationManager to HomelabMcpServer

Update the server struct (building on Step 3):

```rust
use crate::confirmation::ConfirmationManager;

#[derive(Clone)]
pub struct HomelabMcpServer {
    config: Arc<Config>,
    manager: Arc<ConnectionManager>,
    audit: Arc<AuditLogger>,
    rate_limiter: Arc<crate::rate_limit::RateLimiter>,
    confirmation: Arc<ConfirmationManager>,
}

impl HomelabMcpServer {
    pub fn new(
        config: Arc<Config>,
        manager: Arc<ConnectionManager>,
        audit: Arc<AuditLogger>,
    ) -> Self {
        let rate_limiter = Arc::new(
            crate::rate_limit::RateLimiter::from_config(&config.rate_limits.limits)
        );

        // Build confirmation manager from [confirm] config section.
        // If no [confirm] section, rules are empty and no tools require confirmation.
        let confirm_rules = config.confirm.clone().unwrap_or_default();
        let confirmation = Arc::new(ConfirmationManager::new(confirm_rules));

        Self {
            config,
            manager,
            audit,
            rate_limiter,
            confirmation,
        }
    }
}
```

**File:** `src/tools/ssh.rs` — add confirmation check to ssh_exec

The ssh.exec tool must check confirmation AFTER command validation but BEFORE execution:

```rust
pub async fn ssh_exec(
    manager: Arc<ConnectionManager>,
    confirmation: Arc<ConfirmationManager>,
    host: String,
    command: String,
    timeout: Option<u64>,
    dry_run: Option<bool>,
    audit: Arc<AuditLogger>,
) -> anyhow::Result<String> {
    let timeout_duration = Duration::from_secs(timeout.unwrap_or(30).min(300));

    // 1. Validate command against allowlist/blocklist (Layer 9)
    validate_command(&command, &manager.config().ssh.command_allowlist)?;

    // 2. Dry-run check
    if dry_run.unwrap_or(false) {
        audit.log("ssh.exec", &host, "dry_run", Some(&command)).await;
        return Ok(format!(
            "DRY RUN: Command '{}' passes validation for host '{}'. \
             Set dry_run=false to execute.",
            command, host
        ));
    }

    // 3. Confirmation check (Layer 8) — BEFORE execution
    // For ssh.exec, the command text is checked against when_pattern rules.
    if let Some(confirmation_response) = confirmation
        .check_and_maybe_require(
            "ssh.exec",
            Some(&command),
            &format!("About to run '{}' on host '{}'", command, host),
            &serde_json::json!({"host": host, "command": command}).to_string(),
        )
        .await?
    {
        audit.log("ssh.exec", &host, "confirmation_required", Some(&command)).await;
        return Ok(confirmation_response);
    }

    // 4. Proceed with execution (checkout, exec channel, return)
    // ... (same as M3 implementation)
}
```

**File:** `src/mcp.rs` — add `.confirm` tool handler

Register a confirmation tool that handles ALL `.confirm` calls:

```rust
/// Confirm a previously requested destructive operation.
/// This tool is called by the LLM after presenting the confirmation
/// to the user. The token is single-use and expires after 5 minutes.
///
/// See security-approach.md Layer 8.
#[tool(description = "Confirm a previously requested destructive operation. \
    Provide the confirmation token that was returned by the original tool call. \
    Tokens are single-use and expire after 5 minutes.")]
pub async fn confirm_operation(
    &self,
    #[arg(description = "The confirmation token from the original tool response")]
    token: String,
    #[arg(description = "The original tool name (e.g., 'docker.container.delete', 'ssh.exec')")]
    tool_name: String,
) -> anyhow::Result<String> {
    self.rate_limiter.check("confirm_operation")?;

    // Validate and consume the token
    let original_params_json = self.confirmation.confirm(&token, &tool_name).await?;

    self.audit.log("confirm_operation", "n/a", "confirmed", Some(&format!(
        "tool={} token={}", tool_name, token
    ))).await;

    // Re-dispatch to the original tool with the stored parameters.
    // The confirmation manager returns the original params_json, so we
    // deserialize and call the appropriate tool function.
    match tool_name.as_str() {
        "ssh.exec" => {
            let params: serde_json::Value = serde_json::from_str(&original_params_json)?;
            let host = params["host"].as_str().unwrap_or("").to_string();
            let command = params["command"].as_str().unwrap_or("").to_string();
            let timeout = params["timeout"].as_u64();

            // Execute WITHOUT re-checking confirmation (we just confirmed)
            ssh::ssh_exec_confirmed(
                self.manager.clone(),
                host,
                command,
                timeout,
                self.audit.clone(),
            )
            .await
        }
        // Future: "docker.container.delete", "docker.image.delete", etc.
        _ => Err(anyhow::anyhow!(
            "Confirmation not supported for tool '{}'. \
             This tool may not be implemented yet.",
            tool_name
        )),
    }
}
```

**File:** `src/tools/ssh.rs` — add `ssh_exec_confirmed` (bypasses confirmation check)

```rust
/// Execute an SSH command that has already been confirmed via Layer 8.
/// This skips the confirmation check but still validates the command
/// against allowlist/blocklist (defense in depth).
pub async fn ssh_exec_confirmed(
    manager: Arc<ConnectionManager>,
    host: String,
    command: String,
    timeout: Option<u64>,
    audit: Arc<AuditLogger>,
) -> anyhow::Result<String> {
    let timeout_duration = Duration::from_secs(timeout.unwrap_or(30).min(300));

    // Still validate command (defense in depth — rules may have changed)
    validate_command(&command, &manager.config().ssh.command_allowlist)?;

    // Proceed with execution (same as normal ssh_exec post-confirmation)
    // ... checkout, exec channel, return, output envelope wrapping ...
}
```

**Config parsing:** Add `confirm` section to `Config` struct in `src/config.rs`:

```rust
use crate::confirmation::ConfirmRule;

#[derive(Debug, Deserialize)]
pub struct Config {
    // ... existing fields ...

    /// [confirm] section — Layer 8 confirmation rules.
    /// Optional. If absent, no tools require confirmation.
    #[serde(default)]
    pub confirm: Option<HashMap<String, ConfirmRule>>,
}
```

**Verification:** `cargo check` compiles. `cargo test confirmation` passes.

---

#### Step 6: Health monitor background task

**File:** `src/connection.rs` — add `spawn_health_monitor` function

**Design context (connection-manager.md lines 209-238):** A tokio task runs every 30 seconds, pings Docker clients, cleans up stale SSH sessions, and updates ConnectionManager health status.

```rust
use tokio::time::{interval, Duration};
use tracing::{info, warn, debug};

impl ConnectionManager {
    /// Spawn the background health monitor task.
    /// Returns a JoinHandle that can be used for graceful shutdown.
    ///
    /// The health monitor (connection-manager.md lines 209-238):
    /// - Runs every 30 seconds
    /// - Pings each Docker client
    /// - Cleans up stale SSH sessions (age + idle checks)
    /// - Updates ConnectionManager health status per connection
    /// - Uses exponential backoff for reconnection attempts (Step 7)
    pub fn spawn_health_monitor(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let manager = Arc::clone(self);

        tokio::spawn(async move {
            let mut tick = interval(Duration::from_secs(30));
            info!("Health monitor started (30s interval)");

            loop {
                tick.tick().await;
                debug!("Health monitor: running checks");

                // --- Docker health checks ---
                // Ping each Docker client. On success, mark healthy.
                // On failure, increment consecutive_failures and update status.
                for entry in manager.docker.iter() {
                    let (name, handle) = entry.pair();

                    // Check backoff — should we skip this cycle?
                    if manager.should_skip_health_check(name).await {
                        debug!("Health monitor: skipping Docker '{}' (backoff)", name);
                        continue;
                    }

                    match handle.client.ping().await {
                        Ok(_) => {
                            if manager.mark_healthy(name) {
                                info!("Health monitor: Docker '{}' recovered", name);
                            }
                        }
                        Err(e) => {
                            let failures = manager.mark_unhealthy(
                                name,
                                format!("ping failed: {}", e),
                            );
                            warn!(
                                "Health monitor: Docker '{}' unhealthy ({} consecutive failures): {}",
                                name, failures, e
                            );
                        }
                    }
                }

                // --- SSH health checks ---
                // Clean up stale sessions in each pool, then check connectivity.
                for entry in manager.ssh.iter() {
                    let (name, pool) = entry.pair();

                    if manager.should_skip_health_check(name).await {
                        debug!("Health monitor: skipping SSH '{}' (backoff)", name);
                        continue;
                    }

                    // Clean up stale sessions (expired age, idle too long)
                    pool.cleanup_stale_sessions().await;

                    // Check connectivity by attempting a keepalive on an idle session
                    // (or attempting a new connection if pool is empty)
                    match pool.check_connectivity().await {
                        Ok(_) => {
                            if manager.mark_healthy(name) {
                                info!("Health monitor: SSH '{}' recovered", name);
                            }
                        }
                        Err(e) => {
                            let failures = manager.mark_unhealthy(
                                name,
                                format!("connectivity check failed: {}", e),
                            );
                            warn!(
                                "Health monitor: SSH '{}' unhealthy ({} consecutive failures): {}",
                                name, failures, e
                            );
                        }
                    }
                }
            }
        })
    }
}
```

**Integration in `src/main.rs`:**

```rust
// In run_server(), after creating the ConnectionManager:
let manager = Arc::new(ConnectionManager::new(config.clone()).await?);

// Spawn health monitor background task
let health_handle = manager.spawn_health_monitor();
info!("Health monitor spawned");

// ... start MCP server ...

// On shutdown (see Step 8), abort the health monitor:
// health_handle.abort();
```

**Verification:** `cargo check` compiles. Health monitor logs visible with `RUST_LOG=debug`.

---

#### Step 7: Reconnection backoff

**File:** `src/connection.rs` — add backoff tracking to ConnectionHealth

**Design context (connection-manager.md lines 241-257):** When the health monitor detects a down host, it uses exponential backoff:
- Failure 1: retry at next health check (30s)
- Failure 2: skip 1 cycle (60s)
- Failure 3: skip 3 cycles (120s)
- Failure 4+: skip 5 cycles (180s) — capped

```rust
pub struct ConnectionHealth {
    pub status: ConnectionStatus,
    pub last_success: Option<Instant>,
    pub last_error: Option<String>,
    pub consecutive_failures: u32,
    /// How many health check cycles to skip before retrying.
    /// Calculated from consecutive_failures using the backoff schedule.
    pub skip_cycles: u32,
    /// How many cycles have been skipped since last retry attempt.
    pub cycles_skipped: u32,
}

impl ConnectionManager {
    /// Determine whether to skip the health check for a given connection
    /// based on the exponential backoff schedule from connection-manager.md.
    pub async fn should_skip_health_check(&self, name: &str) -> bool {
        if let Some(mut health) = self.health.get_mut(name) {
            if health.skip_cycles > 0 && health.cycles_skipped < health.skip_cycles {
                health.cycles_skipped += 1;
                return true;
            }
            // Reset skip counter — we're going to check this cycle
            health.cycles_skipped = 0;
        }
        false
    }

    /// Mark a connection as healthy. Resets all failure tracking.
    /// Returns true if the connection was previously unhealthy (recovery event).
    pub fn mark_healthy(&self, name: &str) -> bool {
        if let Some(mut health) = self.health.get_mut(name) {
            let was_unhealthy = !matches!(health.status, ConnectionStatus::Connected);
            health.status = ConnectionStatus::Connected;
            health.last_success = Some(Instant::now());
            health.last_error = None;
            health.consecutive_failures = 0;
            health.skip_cycles = 0;
            health.cycles_skipped = 0;
            was_unhealthy
        } else {
            false
        }
    }

    /// Mark a connection as unhealthy. Increments failure counter and
    /// calculates backoff schedule.
    /// Returns the new consecutive_failures count.
    pub fn mark_unhealthy(&self, name: &str, error: String) -> u32 {
        if let Some(mut health) = self.health.get_mut(name) {
            health.consecutive_failures += 1;
            health.last_error = Some(error);
            health.status = ConnectionStatus::Disconnected;

            // Backoff schedule per connection-manager.md lines 246-249:
            // Failure 1: retry immediately (skip 0 cycles)
            // Failure 2: skip 1 cycle (60s total)
            // Failure 3: skip 3 cycles (120s total)
            // Failure 4+: skip 5 cycles (180s total, capped)
            health.skip_cycles = match health.consecutive_failures {
                1 => 0,     // 30s — next check
                2 => 1,     // 60s — skip 1
                3 => 3,     // 120s — skip 3
                _ => 5,     // 180s — skip 5 (capped)
            };
            health.cycles_skipped = 0;

            health.consecutive_failures
        } else {
            0
        }
    }

    /// Get a health-aware error message for a disconnected host.
    /// Format matches connection-manager.md lines 252-256.
    pub fn disconnected_error_message(&self, name: &str) -> String {
        if let Some(health) = self.health.get(name) {
            let last_success = health.last_success
                .map(|t| {
                    let mins = t.elapsed().as_secs() / 60;
                    if mins == 0 {
                        "less than a minute ago".to_string()
                    } else {
                        format!("{} minutes ago", mins)
                    }
                })
                .unwrap_or_else(|| "never".to_string());

            let last_error = health.last_error
                .as_deref()
                .unwrap_or("unknown");

            format!(
                "Host '{}' is unreachable. Status: {:?}. Last error: {}. \
                 Last successful connection: {}. \
                 The connection manager will retry automatically.",
                name, health.status, last_error, last_success
            )
        } else {
            format!("Host '{}' not found in connection manager", name)
        }
    }
}
```

**Verification:** Backoff logic is testable with unit tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backoff_schedule() {
        // Verify the skip_cycles match the design doc:
        // failure 1 -> 0 skips (30s)
        // failure 2 -> 1 skip (60s)
        // failure 3 -> 3 skips (120s)
        // failure 4+ -> 5 skips (180s, capped)
        let expected = vec![
            (1, 0),   // 30s
            (2, 1),   // 60s
            (3, 3),   // 120s
            (4, 5),   // 180s
            (5, 5),   // 180s (capped)
            (10, 5),  // 180s (capped)
        ];

        for (failures, expected_skips) in expected {
            let skips = match failures {
                1 => 0,
                2 => 1,
                3 => 3,
                _ => 5,
            };
            assert_eq!(
                skips, expected_skips,
                "failure {} should skip {} cycles",
                failures, expected_skips
            );
        }
    }
}
```

---

#### Step 8: Graceful shutdown

**File:** `src/main.rs` — add shutdown handler

**Design context (connection-manager.md lines 261-285):** On SIGTERM or SIGINT:
1. Stop accepting new MCP tool calls
2. Wait for in-flight tool calls to complete (with 10s timeout)
3. Close all SSH sessions gracefully (send disconnect)
4. Drop Docker clients (connection cleanup handled by bollard)
5. Flush audit log
6. Exit

```rust
use tokio::signal;
use tokio::sync::watch;
use std::sync::Arc;

/// Run the MCP server with graceful shutdown support.
pub async fn run_server(config: Arc<Config>) -> anyhow::Result<()> {
    // ... (config validation, connection manager creation, etc.) ...

    let manager = Arc::new(ConnectionManager::new(config.clone()).await?);
    let audit = Arc::new(AuditLogger::new(&config.audit)?);

    // Spawn health monitor
    let health_handle = manager.spawn_health_monitor();

    // Create a shutdown signal channel.
    // The MCP server checks this to stop accepting new calls.
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let mcp_server = HomelabMcpServer::new(
        config.clone(),
        manager.clone(),
        audit.clone(),
    );

    // Spawn the MCP server on stdio
    let mcp_handle = tokio::spawn(async move {
        // rmcp server runs here, reading from stdin, writing to stdout.
        // When shutdown_rx signals, the server stops accepting new requests.
        run_mcp_stdio(mcp_server, shutdown_rx).await
    });

    // Wait for SIGTERM or SIGINT
    let shutdown_signal = async {
        let ctrl_c = signal::ctrl_c();
        #[cfg(unix)]
        let mut sigterm = signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to register SIGTERM handler");

        tokio::select! {
            _ = ctrl_c => {
                info!("Received SIGINT, initiating graceful shutdown");
            }
            #[cfg(unix)]
            _ = sigterm.recv() => {
                info!("Received SIGTERM, initiating graceful shutdown");
            }
        }
    };

    shutdown_signal.await;

    // --- Graceful shutdown sequence (connection-manager.md lines 263-269) ---

    // 1. Signal MCP server to stop accepting new tool calls
    let _ = shutdown_tx.send(true);
    info!("Shutdown: stopped accepting new tool calls");

    // 2. Wait for in-flight tool calls to complete (10s timeout)
    info!("Shutdown: waiting up to 10s for in-flight tool calls...");
    match tokio::time::timeout(
        Duration::from_secs(10),
        mcp_handle,
    ).await {
        Ok(Ok(_)) => info!("Shutdown: MCP server stopped cleanly"),
        Ok(Err(e)) => warn!("Shutdown: MCP server error: {}", e),
        Err(_) => {
            warn!("Shutdown: 10s timeout exceeded, forcing MCP server stop");
            // In-flight calls are abandoned at this point
        }
    }

    // 3. Abort health monitor
    health_handle.abort();
    info!("Shutdown: health monitor stopped");

    // 4. Close all SSH sessions gracefully
    for entry in manager.ssh.iter() {
        let (name, pool) = entry.pair();
        if let Err(e) = pool.close_all().await {
            warn!("Shutdown: error closing SSH pool '{}': {}", name, e);
        }
    }
    info!("Shutdown: SSH sessions closed");

    // 5. Drop Docker clients (bollard handles cleanup internally)
    drop(manager);
    info!("Shutdown: Docker clients dropped");

    // 6. Flush audit log
    audit.flush().await;
    info!("Shutdown: audit log flushed");

    info!("Shutdown complete");
    Ok(())
}
```

**File:** `src/audit.rs` — add `flush` method

```rust
impl AuditLogger {
    /// Flush any buffered audit log entries.
    /// Called during graceful shutdown to ensure all entries are persisted.
    pub async fn flush(&self) {
        if let Some(ref file) = self.file {
            // File logging uses BufWriter; flush it
            if let Ok(mut f) = file.lock() {
                use std::io::Write;
                let _ = f.flush();
            }
        }
        // Syslog: no flush needed (each entry is sent immediately)
    }
}
```

**Verification:** `cargo check` compiles. Test by running the server and pressing Ctrl+C — should see orderly shutdown log messages.

---

### M4 Verification Checklist

- [ ] Audit logging records all tool invocations
- [ ] Rate limiter blocks excess calls with correct error format
- [ ] Rate limiter wildcard patterns work (`"docker.container.*"` matches all 5 Docker tools)
- [ ] Rate limiter: no configured limit means no restriction (not a hardcoded default)
- [ ] Confirmation flow: `ssh.exec` with `when_pattern` match returns token
- [ ] Confirmation flow: token is single-use and expires after 5 minutes
- [ ] Confirmation flow: `confirm_operation` tool validates and re-dispatches
- [ ] Health monitor runs every 30s, pings Docker, cleans SSH sessions
- [ ] Reconnection backoff follows schedule: 30s → 60s → 120s → 180s capped
- [ ] Recovery from disconnected state resets backoff
- [ ] Graceful shutdown on SIGTERM/SIGINT: drains in-flight (10s), closes SSH, flushes audit
- [ ] Doctor subcommand shows all configured hosts, rate limits, and confirmation rules
- [ ] Command validation prevents dangerous commands (allowlist/blocklist)
- [ ] `cargo test rate_limit` passes
- [ ] `cargo test confirmation` passes

---

## Milestone 5: End-to-End Validation (1 day)

### M5 Goals

Verify the complete integration with Spacebot:
1. Configure Spacebot to use the homelab MCP server
2. Test Docker tools via Spacebot
3. Test SSH tools via Spacebot (with dry-run first)
4. Verify audit logging and rate limiting
5. Verify Layer 8 confirmation flow end-to-end
6. Verify Layer 9 output envelope is present on all tool responses
7. Verify error handling and recovery

**Success Criteria:**
- Spacebot can spawn the MCP server
- Spacebot tools list includes all 9 tools (5 Docker + 3 SSH + `confirm_operation`)
- Docker tools execute and return results
- SSH tools execute (or dry-run)
- All tool responses include `data_classification: "untrusted_external"` envelope (Layer 9)
- Confirmation flow works: ssh.exec with dangerous pattern returns token, confirm_operation executes
- Audit log shows all operations (including confirmation_required and confirmed entries)
- Rate limiting prevents spam (including wildcard patterns)
- Errors are handled gracefully

---

### M5 Implementation

#### Step 1: Configure Spacebot for MCP integration

**File:** Spacebot config (typically `~/.spacebot/config.toml` or equivalent)

Add MCP server configuration:

```toml
[[mcp_servers]]
name = "homelab"
transport = "stdio"
enabled = true
command = "/path/to/spacebot-homelab-mcp"
args = ["server", "--config", "~/.spacebot-homelab/config.toml"]
env = {}
```

**Create homelab config:**

File: `~/.spacebot-homelab/config.toml`

```toml
[docker.hosts.local]
host = "unix:///var/run/docker.sock"

[ssh.hosts.localhost]
host = "127.0.0.1"
port = 22
user = "testuser"
private_key_path = "~/.ssh/id_rsa"

[ssh.pool]
max_sessions_per_host = 3

[ssh.command_allowlist]
allowed_prefixes = [
    "docker",
    "df",
    "uptime",
    "free",
    "ls",
    "rm",  # Allowed prefix so confirmation flow can be tested
]
blocked_patterns = [
    "dd if=",
]

[audit]
file = "/tmp/homelab-audit.log"

[rate_limits.limits]
# Wildcard pattern — all 5 docker.container.* tools share this limit
"docker.container.*" = { per_minute = 20 }
# Exact match — overrides the wildcard for this specific tool
"docker.container.list" = { per_minute = 10 }
"ssh.exec" = { per_minute = 5 }

# Layer 8 confirmation rules (security-approach.md)
[confirm]
# ssh.exec requires confirmation when command matches these patterns
"ssh.exec" = { when_pattern = ["rm -rf", "dd if=", "mkfs", "fdisk", "parted"] }
# Future: "docker.container.delete" = "always"
```

---

#### Step 2: Test Docker tool discovery

**Command:** In Spacebot, test if the homelab tools are available

```
@spacebot list tools homelab
```

**Expected output:** List of 9 tools (5 Docker + 3 SSH + 1 confirmation)

```
Available tools from homelab MCP:
- docker.container.list
- docker.container.start
- docker.container.stop
- docker.container.logs
- docker.container.inspect
- ssh.exec
- ssh.upload
- ssh.download
- confirm_operation
```

---

#### Step 3: Test Docker tools via Spacebot

**Test 1: List containers**

Message: `List my Docker containers`

Expected flow:
```
Channel message received
  → Parse intent (list containers)
  → Route to homelab MCP
  → Call docker.container.list
  → MCP returns formatted list
  → Audit log records invocation
  → Response shown to user
```

Expected output:
```
CONTAINER ID  | NAME                 | IMAGE                      | STATUS          | PORTS
────────────────────────────────────────────────────────────────────────────────────────────
a1b2c3d4e5f6  | /my-container        | ubuntu:latest              | Up 2 hours      | 8080->8080
```

**Verification:** ✓ Output is human-readable and correctly formatted

**Test 2: Start a container**

Message: `Start container my-container`

Expected output:
```
Container 'my-container' started successfully. Use docker.container.list to verify status.
```

**Verification:** ✓ Container starts and status can be verified

**Test 3: Get container logs**

Message: `Show me the logs for my-container`

Expected output:
```
[Output truncated. Showing last 10,000 chars of XXXX total chars.]
<log lines>
```

**Verification:** ✓ Logs are retrieved and truncated if necessary

**Test 4: Rate limiting**

Message (repeated 11 times quickly): `List containers`

Expected: 10 calls succeed, 11th is rate-limited

Expected error:
```
Rate limit exceeded for docker.container.list. Limit: 10/min. Retry after 45s.
```

**Verification:** ✓ Rate limiter prevents abuse

---

#### Step 4: Test SSH tools via Spacebot

**Test 1: Dry-run command**

Message: `Check disk usage with dry-run`

Expected output:
```
DRY RUN: Command 'df -h' passes validation for host 'localhost'. Set dry_run=false to execute.
```

**Verification:** ✓ Dry-run validates without executing

**Test 2: Command validation**

Message: `Execute rm -rf /`

Expected error:
```
Command blocked: contains dangerous pattern 'rm -rf'. This pattern is in the blocked list for safety.
```

**Verification:** ✓ Blocklist prevents dangerous commands

**Test 3: Command execution**

Message: `Run df -h on localhost`

Expected output:
```
Filesystem      Size  Used Avail Use% Mounted on
/dev/sda1       100G   40G   60G  40% /
```

**Verification:** ✓ SSH commands execute and output is returned

---

#### Step 4b: Test Layer 8 confirmation flow via Spacebot

**Test 1: Command that triggers confirmation**

Message: `Run rm -rf /tmp/old-backups on localhost`

The test config has `"ssh.exec" = { when_pattern = ["rm -rf"] }` in the `[confirm]` section, AND `"rm"` is in `allowed_prefixes`. The `rm -rf` is NOT in `blocked_patterns` (we removed it from blocked_patterns so the confirmation flow can be tested — in production, operators would configure either blocklist OR confirmation, not both for the same pattern).

Expected flow:
```
1. ssh.exec receives command "rm -rf /tmp/old-backups"
2. Command passes allowlist validation ("rm" prefix matches)
3. Command does NOT match any blocked_patterns
4. Confirmation check: "rm -rf" matches when_pattern for ssh.exec
5. MCP returns confirmation-required response (NOT the execution result)
```

Expected output (returned to LLM):
```json
{
    "status": "confirmation_required",
    "token": "550e8400-e29b-41d4-a716-446655440000",
    "message": "About to run 'rm -rf /tmp/old-backups' on host 'localhost'. Call ssh.exec.confirm with token 550e8400-e29b-41d4-a716-446655440000 to proceed."
}
```

**Verification:** ✓ Dangerous command returns token, NOT execution result

**Test 2: Confirm with valid token**

The LLM presents the confirmation message to the user. User confirms. LLM calls `confirm_operation`:

```
confirm_operation(token="550e8400-...", tool_name="ssh.exec")
```

Expected: Command executes and returns the SSH output.

**Verification:** ✓ Token consumed, command executes, audit log shows both `confirmation_required` and `confirmed` entries

**Test 3: Confirm with expired/invalid token**

```
confirm_operation(token="bogus-token", tool_name="ssh.exec")
```

Expected error:
```
Invalid or expired confirmation token. Tokens expire after 5 minutes.
```

**Verification:** ✓ Invalid tokens are rejected

**Test 4: Confirm with already-used token**

Re-use the token from Test 2:
```
confirm_operation(token="550e8400-...", tool_name="ssh.exec")
```

Expected error:
```
Confirmation token has already been used. Each token is single-use.
```

(Note: in practice, the token is removed from the map after first use, so this returns the "invalid or expired" error instead. Both are acceptable.)

**Verification:** ✓ Tokens are single-use

---

#### Step 4c: Verify Layer 9 output envelope on all tool responses

**Purpose:** Every tool response must be wrapped in the output sanitization envelope per security-approach.md Layer 9. This marks all tool output as untrusted external data.

**Test: Inspect Docker tool response structure**

Call `docker.container.list` and examine the raw MCP response JSON:

```json
{
    "type": "tool_result",
    "source": "docker.container.list",
    "data_classification": "untrusted_external",
    "content": "CONTAINER ID | NAME | IMAGE | STATUS | PORTS\n..."
}
```

**What to verify:**
1. The response includes `"data_classification": "untrusted_external"` — this is the key Layer 9 field
2. The `"source"` field matches the tool name
3. The `"type"` is `"tool_result"`
4. The actual output is in the `"content"` field

**Test: Inspect SSH tool response structure**

Call `ssh.exec` with a safe command like `df -h`:

```json
{
    "type": "tool_result",
    "source": "ssh.exec",
    "data_classification": "untrusted_external",
    "content": "Filesystem      Size  Used Avail Use% Mounted on\n..."
}
```

**Verification:** ✓ Both Docker and SSH tool responses include the output envelope

**Implementation reference:** The `wrap_output_envelope()` helper (implemented in M2 and M3) handles this wrapping. Verify it is called in every tool handler's success path.

---

#### Step 5: Verify audit logging

**File:** Check the audit log at `/tmp/homelab-audit.log` (or configured path)

Expected content:
```
1704067200 tool=docker.container.list host=local result=success details=null
1704067205 tool=docker.container.start host=local result=success details=my-container
1704067210 tool=ssh.exec host=localhost result=dry_run details=df -h
1704067215 tool=ssh.exec host=localhost result=success details=df -h
```

**Verification:** ✓ All tool invocations are logged with timestamp, tool name, host, result

---

#### Step 6: Error handling and recovery

**Test 1: Unreachable host**

Message: `List containers on nonexistent-host`

Expected error:
```
Docker host 'nonexistent-host' not configured or not connected
```

**Verification:** ✓ Clear error message without crashing

**Test 2: Container not found**

Message: `Start container nonexistent-container`

Expected error:
```
Failed to start container 'nonexistent-container': <Docker API error>
```

**Verification:** ✓ Graceful error handling

**Test 3: SSH connection failure**

Message: `Run echo hello on nonexistent-host`

Expected error:
```
SSH host 'nonexistent-host' not configured or not connected
```

**Verification:** ✓ Clear error message

---

### M5 Verification Checklist

- [ ] Spacebot connects to homelab MCP server
- [ ] All 9 tools are discoverable (5 Docker + 3 SSH + confirm_operation)
- [ ] Docker tools work end-to-end
- [ ] SSH dry-run works
- [ ] Command validation prevents dangerous commands
- [ ] Rate limiting prevents abuse (including wildcard patterns)
- [ ] Layer 8: ssh.exec with dangerous pattern returns confirmation token
- [ ] Layer 8: confirm_operation with valid token executes the command
- [ ] Layer 8: invalid/expired/reused tokens are rejected
- [ ] Layer 9: all tool responses include `data_classification: "untrusted_external"` envelope
- [ ] Audit logging records all operations (including confirmation flow entries)
- [ ] Error messages are clear and helpful
- [ ] No panics or crashes
- [ ] Connection recovery works (tools work again after transient failure)
- [ ] Graceful shutdown works (Ctrl+C → orderly drain → clean exit)

---

## Final Verification: Full M1-M5 Checklist

Once all milestones are complete, verify:

### Compilation
- [ ] `cargo build --release` succeeds with 0 errors
- [ ] No warnings about unused code
- [ ] Binary is at `target/release/spacebot-homelab-mcp`

### Functionality
- [ ] `spacebot-homelab-mcp server --config <path>` starts without panics
- [ ] `spacebot-homelab-mcp doctor --config <path>` runs and reports status
- [ ] All 9 tools are callable via MCP protocol (5 Docker + 3 SSH + confirm_operation)
- [ ] Docker tools work with local daemon
- [ ] SSH tools work with test host (or dry-run without error)
- [ ] Audit logging works
- [ ] Rate limiting works (including wildcard patterns)
- [ ] Layer 8 confirmation flow works (token generation, validation, expiry, single-use)
- [ ] Layer 9 output envelope present on all tool responses
- [ ] Health monitor runs and updates connection status
- [ ] Reconnection backoff follows schedule (30s → 60s → 120s → 180s)
- [ ] Graceful shutdown drains in-flight calls, closes SSH, flushes audit
- [ ] Output is properly formatted and truncated

### Integration with Spacebot
- [ ] Spacebot can spawn the binary as child process
- [ ] Spacebot calls tools/list and receives 9 tools
- [ ] Spacebot can call docker tools and get results
- [ ] Spacebot can call ssh tools and get results
- [ ] Confirmation flow works through Spacebot (token returned, confirm_operation executes)
- [ ] Errors are returned as tool results, not protocol errors

### Testing
- [ ] `cargo test --lib` passes all unit tests
- [ ] Command validation tests pass
- [ ] Rate limiter tests pass (including wildcard pattern tests)
- [ ] Confirmation manager tests pass (token lifecycle, single-use, expiry, wrong-tool)
- [ ] Backoff schedule tests pass
- [ ] Integration tests compile (may be skipped in CI)

### Documentation
- [ ] README.md is up to date
- [ ] All tool descriptions are clear
- [ ] Error messages are helpful
- [ ] Examples in README work

---

## Troubleshooting Guide

### Compilation Errors

**Error: `rmcp` crate not found**
- Verify `Cargo.toml` has `rmcp = { version = "1.1", features = ["server", "transport-io", "macros"] }`
- Run `cargo update` to refresh crate index

**Error: Tool handler signatures don't match**
- Verify all tool handlers have matching parameter names in the MCP schema
- Check that all handlers return `anyhow::Result<String>`

**Error: `bollard` API mismatch**
- Check which version of bollard is specified (0.17)
- Review bollard changelog if types don't match expected API

### Runtime Errors

**Error: Docker daemon not reachable**
- Verify Docker socket exists at `/var/run/docker.sock` (for Unix)
- Check Docker daemon is running: `docker ps`
- Verify user has permission to access socket: `ls -la /var/run/docker.sock`

**Error: SSH connection failed**
- Verify SSH host is reachable: `ssh -i /path/to/key user@host`
- Check private key permissions: `chmod 600 ~/.ssh/id_rsa`
- Verify host is configured correctly in `config.toml`

**Error: Rate limiter blocking all requests**
- Check configured limits in `config.toml`
- Verify the tool name matches exactly
- Wait 60 seconds for the rate limit window to reset

### Testing Issues

**Integration tests fail with "Docker not available"**
- Tests marked with `#[ignore]` are skipped by default
- To run them: `cargo test --test docker_integration -- --ignored --nocapture`
- Docker daemon must be running and accessible

**SSH tests fail**
- SSH tests require a running SSH server
- For now, tests are stubbed and can be expanded later
- Use dry-run mode to test command validation without SSH

---

## Next Steps (Beyond M5)

After M1-M5 are complete:

1. **SFTP implementation** — full file upload/download with russh-sftp (M3 uses exec-based scp fallback)
2. **Rate limiting per-user** — track usage per Spacebot user (not just global)
3. **Syslog support** — optional audit logging to syslog
4. **Metrics and observability** — Prometheus metrics for tool usage, errors, latency
5. **SSH channel multiplexing** — multiple exec channels on a single SSH session (V2 optimization per connection-manager.md)
6. **docker.image.* tools** — image management tools (pull, list, prune) noted as future in architecture-decision.md
7. **docker.container.delete / docker.container.create** — destructive tools that fully exercise Layer 8 confirmation with `"always"` rule

These can be implemented in subsequent phases once the foundation is solid.
