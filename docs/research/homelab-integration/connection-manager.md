# Connection Manager Design

**Context:** The MCP server (`spacebot-homelab-mcp`) manages persistent connections to Docker daemons and SSH hosts. Unlike Spacebot's built-in tools (shell, file) which are stateless per-invocation, homelab tools require connection pooling, reconnection, and shared state across tool calls.

This document specifies how the connection manager works inside the MCP server process.

---

## Architecture

```
spacebot-homelab-mcp process
│
├── MCP Protocol Handler (rmcp server)
│   └── Routes tool calls to tool implementations
│
├── ConnectionManager (shared state)
│   │
│   ├── DockerClients: HashMap<String, DockerClient>
│   │   └── "local" → bollard::Docker (unix socket)
│   │   └── "vps"   → bollard::Docker (TCP+TLS)
│   │
│   └── SshPool: HashMap<String, SshHostPool>
│       └── "home"     → Pool<russh::Session> (max 3)
│       └── "proxmox"  → Pool<russh::Session> (max 2)
│
└── HealthMonitor (background task)
    └── Periodic health checks per connection
    └── Updates ConnectionManager health status
```

### Ownership

The `ConnectionManager` is created at startup and shared via `Arc<ConnectionManager>`. Each MCP tool receives a reference to it. The MCP server is single-process, so there are no cross-process synchronization concerns — only cross-task (tokio) synchronization.

```rust
pub struct ConnectionManager {
    docker: DashMap<String, DockerHandle>,
    ssh: DashMap<String, SshHostPool>,
    health: DashMap<String, ConnectionHealth>,
    config: Arc<Config>,
}

pub struct ConnectionHealth {
    status: ConnectionStatus,
    last_success: Option<Instant>,
    last_error: Option<String>,
    consecutive_failures: u32,
}

pub enum ConnectionStatus {
    Connected,
    Degraded { reason: String },
    Disconnected,
    Connecting,
}
```

`DashMap` is used instead of `RwLock<HashMap>` because tool calls are concurrent and read-heavy. Individual connection handles have their own internal synchronization.

---

## Docker Client Management

### Client lifecycle

Docker connections are simpler than SSH — `bollard::Docker` maintains its own connection state and reconnects automatically for most transports.

```rust
pub struct DockerHandle {
    client: bollard::Docker,
    transport: DockerTransport,
}

pub enum DockerTransport {
    UnixSocket { path: PathBuf },
    Tcp { host: String, tls: Option<TlsConfig> },
}
```

**Initialization:**
1. At startup, create a `bollard::Docker` client for each configured Docker host
2. Validate connectivity with `docker.ping().await`
3. If ping fails, mark as `Disconnected` — do not block other connections

**Per-tool-call:**
1. Tool implementation calls `connection_manager.docker("local")`
2. Returns `&DockerHandle` or an error with health status
3. If the client returns a connection error mid-call, mark as `Degraded` and let the health monitor handle reconnection

**Reconnection:**
- The health monitor pings each Docker client every 30 seconds
- On failure: increment `consecutive_failures`, update status
- On recovery: reset status to `Connected`, log the recovery
- No exponential backoff needed — `bollard` handles TCP reconnection internally for most cases

---

## SSH Connection Pool

SSH connections are stateful, expensive to establish (handshake + auth), and can go stale. A pool is necessary.

### Pool design

```rust
pub struct SshHostPool {
    host: SshHostConfig,
    sessions: Arc<Mutex<VecDeque<PooledSession>>>,
    max_sessions: usize,
    active_count: Arc<AtomicUsize>,
}

pub struct PooledSession {
    session: russh::client::Handle<ClientHandler>,
    created_at: Instant,
    last_used: Instant,
}
```

**Why not use `deadpool` or `bb8`?** These are generic pool libraries that work well for database connections. SSH sessions have quirks (channel multiplexing, keepalive, session-level errors that invalidate the whole session) that make a purpose-built pool simpler to reason about. The pool is small (2-5 sessions per host) and the logic is straightforward.

### Checkout / return flow

```
Tool call: ssh.exec(host="nas", command="zpool status")
│
├── pool.checkout("nas")
│   ├── Try to take an idle session from the queue
│   │   ├── Found one → validate it's still alive (send keepalive)
│   │   │   ├── Alive → return it
│   │   │   └── Dead → drop it, try next / create new
│   │   └── Queue empty → create new session if under max_sessions
│   │       ├── Under limit → connect, authenticate, return
│   │       └── At limit → wait with timeout (5s)
│   │           ├── Session returned → use it
│   │           └── Timeout → return error
│
├── Execute command on the session's exec channel
│   ├── Success → return output
│   └── Channel error → mark session as broken
│
└── pool.return(session)
    ├── Session healthy → push back to queue, update last_used
    └── Session broken → drop it, decrement active_count
```

### Session validation

Before returning a pooled session, verify it's still alive:

```rust
impl SshHostPool {
    async fn validate_session(&self, session: &PooledSession) -> bool {
        // 1. Check age — sessions older than max_lifetime are discarded
        if session.created_at.elapsed() > self.max_lifetime {
            return false;
        }

        // 2. Check idle time — stale sessions may have been disconnected by the server
        if session.last_used.elapsed() > self.max_idle_time {
            // Try a keepalive ping
            match session.session.keepalive().await {
                Ok(_) => true,
                Err(_) => false,
            }
        } else {
            true
        }
    }
}
```

### Configuration

```toml
[ssh.pool]
max_sessions_per_host = 3      # Max concurrent SSH sessions to one host
max_lifetime = "30m"           # Recreate sessions after this duration
max_idle_time = "5m"           # Keepalive check if idle longer than this
connect_timeout = "10s"        # Timeout for new SSH connections
checkout_timeout = "5s"        # Timeout waiting for a pooled session
keepalive_interval = "60s"     # Background keepalive ping interval
```

### Channel multiplexing

A single SSH session can open multiple exec channels. For simple command execution, one session can handle sequential commands without needing multiple sessions. The pool exists for **concurrent** tool calls — if two workers need to SSH into the same host simultaneously, they get separate sessions (or separate channels on the same session).

For V1, each checkout gets exclusive use of a session. Channel multiplexing within a session is a V2 optimization.

---

## Reconnection and Retry Logic

### Retry policy for tool calls

Tool calls do **not** retry automatically by default. The MCP server returns the error to the LLM, which can decide whether to retry. This is intentional — the LLM has context about whether a retry makes sense.

Exception: **connection establishment** retries transparently. If a pooled session is dead and a new connection attempt fails, the pool tries once more before returning an error. This handles transient network issues without the LLM needing to know.

```rust
pub struct RetryPolicy {
    max_connect_retries: u32,     // Default: 1 (so 2 total attempts)
    connect_retry_delay: Duration, // Default: 1s
    // Tool-level retries: 0 (errors go to LLM)
}
```

### Health monitor (background task)

A tokio task runs periodically and updates connection health:

```rust
async fn health_monitor(manager: Arc<ConnectionManager>) {
    let mut interval = tokio::time::interval(Duration::from_secs(30));
    loop {
        interval.tick().await;

        // Check Docker clients
        for entry in manager.docker.iter() {
            let (name, handle) = entry.pair();
            match handle.client.ping().await {
                Ok(_) => manager.mark_healthy(name),
                Err(e) => manager.mark_unhealthy(name, e.to_string()),
            }
        }

        // Check SSH pools — send keepalive on idle sessions
        for entry in manager.ssh.iter() {
            let (name, pool) = entry.pair();
            pool.cleanup_stale_sessions().await;
            match pool.check_connectivity().await {
                Ok(_) => manager.mark_healthy(name),
                Err(e) => manager.mark_unhealthy(name, e.to_string()),
            }
        }
    }
}
```

### Reconnection backoff

When the health monitor detects a down host, it tracks consecutive failures and uses exponential backoff for reconnection attempts:

```
Failure 1: retry immediately on next health check (30s)
Failure 2: skip 1 health check cycle (60s)
Failure 3: skip 3 cycles (120s)
Failure 4+: skip 5 cycles (180s) — capped
```

When a tool call comes in for a disconnected host, the MCP server returns:
```json
{
    "error": "SSH host 'nas' is unreachable. Status: Disconnected. Last error: connection refused. Last successful connection: 12 minutes ago. The connection manager will retry automatically."
}
```

---

## Shutdown

On SIGTERM or SIGINT:
1. Stop accepting new MCP tool calls
2. Wait for in-flight tool calls to complete (with 10s timeout)
3. Close all SSH sessions gracefully (send disconnect)
4. Drop Docker clients (connection cleanup is handled by bollard)
5. Flush audit log
6. Exit

```rust
async fn shutdown(manager: Arc<ConnectionManager>) {
    info!("Shutting down connection manager");

    // Close SSH sessions
    for entry in manager.ssh.iter() {
        let (name, pool) = entry.pair();
        if let Err(e) = pool.close_all().await {
            warn!("Error closing SSH pool for '{}': {}", name, e);
        }
    }

    info!("All connections closed");
}
```

---

## Error Taxonomy

All connection errors map to structured MCP error responses:

| Error Category | Example | MCP Response |
|---------------|---------|--------------|
| Host unreachable | SSH connection refused | `"SSH host 'nas' is unreachable"` + health status |
| Authentication failed | Wrong key / user | `"Authentication failed for host 'nas'. Check credentials."` |
| Pool exhausted | All sessions in use | `"All SSH sessions to 'nas' are in use. Try again shortly."` |
| Command timeout | SSH command hung | `"Command timed out after 30s on host 'nas'"` |
| Docker API error | Container not found | Bollard error message passed through |
| Permission denied | Sudo not configured | `"Permission denied on host 'nas': <stderr output>"` |

Errors are returned as MCP tool results (not protocol-level errors) so the LLM can reason about them and take corrective action.

---

## Testing Strategy

### Unit tests
- Pool checkout/return with mock sessions
- Session validation logic (age, idle time, keepalive)
- Health status transitions
- Retry backoff timing

### Integration tests
- Docker client against a local Docker daemon (CI has Docker)
- SSH pool against a test SSH server (use `testcontainers` to spin up an OpenSSH container)
- Concurrent tool calls to verify pool doesn't deadlock
- Connection failure simulation (stop the SSH container mid-test)

### Manual validation
- `spacebot-homelab-mcp doctor` covers the startup validation path
- Unplug/replug a host to verify graceful degradation and recovery
