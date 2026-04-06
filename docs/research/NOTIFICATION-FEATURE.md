# Homelab MCP Desktop Notifications — Implementation Plan

## Overview

Native OS desktop notifications for the homelab MCP server, triggered on successful connection (tools ready) and on failure (with error detail). Designed to be completely isolated from Spacebot — no changes to the Spacebot codebase — while being architecturally ready for future integration into Spacebot's UI notification system.

## Design Decisions

| Decision | Resolution | Rationale |
|----------|-----------|-----------|
| Scope | Homelab-specific only | Notifications are branded for the homelab MCP, not a generic MCP feature |
| Isolation | Entirely within the homelab-mcp project | Zero Spacebot changes. If Spacebot later adds MCP notification support, this is ready to plug in |
| Delivery | Native OS desktop notifications | macOS (osascript), Linux (notify-send/D-Bus), Windows (WinRT). No extra browser tabs or dashboards |
| Triggers | Connected + Failed only | "Initializing" and "Retrying" are transient noise. "Disconnected" is expected on shutdown |
| Content | Rich — title, body, tool count/error detail, system sound | Confirms tools loaded correctly, not just that the process started |
| Implementation site | Inline in the MCP server startup path (`run_server` in `main.rs`) | Only two events to notify on, both happen during init. No background monitor needed |
| Platform support | `notify-rust` crate behind a `notifications` cargo feature flag | Cross-platform, optional, easy to disable when Spacebot integrates |
| Architecture | `Notification` struct + `NotificationDispatcher` trait | `DesktopDispatcher` now, `McpProtocolDispatcher` later. Call sites unchanged |
| Failure rate limiting | First failure + final exhaustion only | Max 2 failure notifications per startup cycle |
| Cross-process dedup | Time-based throttle via temp file (60s window) | Handles Spacebot's retry logic re-spawning the process multiple times |

## Architecture

```
                      Notification { title, body, severity, sound }
                                      |
                         NotificationDispatcher (trait)
                          /                        \
              DesktopDispatcher                McpProtocolDispatcher
              (notify-rust)                    (future — MCP notifications/)
                    |
              ThrottleGuard
              (temp file at /tmp/spacebot-homelab-mcp-notify)
```

### Notification Struct

```rust
pub enum Severity {
    Success,
    Error,
}

pub struct Notification {
    pub title: String,
    pub body: String,
    pub severity: Severity,
    pub sound: bool,
}
```

### NotificationDispatcher Trait

```rust
pub trait NotificationDispatcher: Send + Sync {
    fn dispatch(&self, notification: &Notification) -> Result<(), NotificationError>;
}
```

### DesktopDispatcher

Uses `notify-rust` to send OS-native notifications. Wraps the call with throttle logic.

### ThrottleGuard

Reads/writes a timestamp to `/tmp/spacebot-homelab-mcp-notify`. Rules:
- **Failure notifications**: Skip if a failure notification was sent within the last 60 seconds.
- **Success notifications**: Always fire. Clear the throttle file.
- File is best-effort — if the temp file can't be read/written, the notification fires anyway.

## Notification Content

### On Success (Connected)

```
Title:    Homelab MCP
Body:     Connected — 9 tools available
          Docker (5) | SSH (3) | Confirm (1)
Sound:    Subtle (e.g., "Glass" on macOS / default on Linux)
```

The tool count is derived from the `ToolRouter` at runtime, not hardcoded. The category breakdown (Docker/SSH/Confirm) is derived from tool name prefixes.

### On Failure

```
Title:    Homelab MCP
Body:     Connection failed: SSH key authentication failed (check SSH_KEY_PASSPHRASE)
Sound:    Alert (e.g., "Basso" on macOS / critical on Linux)
```

The error message comes from the `anyhow::Error` that caused the failure — `Config::load`, `ConnectionManager::new`, or `rmcp::serve_server`.

## Implementation Steps

### Step 1: Add Dependencies

In `Cargo.toml`, add `notify-rust` behind a feature flag:

```toml
[features]
default = ["notifications"]
notifications = ["dep:notify-rust"]

[dependencies]
notify-rust = { version = "4", optional = true }
```

### Step 2: Create `src/notifications.rs`

This single file contains all notification logic:

1. **`Severity` enum** — `Success`, `Error`
2. **`Notification` struct** — `title`, `body`, `severity`, `sound`
3. **`NotificationDispatcher` trait** — `fn dispatch(&self, notification: &Notification) -> Result<()>`
4. **`DesktopDispatcher` struct** — implements the trait using `notify-rust`
   - Maps `Severity::Success` to a subtle sound and default urgency
   - Maps `Severity::Error` to an alert sound and critical urgency
5. **`ThrottleGuard`** — manages the temp file for cross-process deduplication
   - `should_notify_failure() -> bool` — returns false if last failure notification was <60s ago
   - `record_failure()` — writes current timestamp to temp file
   - `clear()` — removes the temp file (called on success)
6. **`notify_connected(tool_count: usize, tool_summary: &str)`** — builds and dispatches the success notification
7. **`notify_failed(error: &str)`** — builds and dispatches the failure notification (with throttle check)
8. **No-op stubs** when the `notifications` feature is disabled (all functions become `fn f(...) {}`)

### Step 3: Integrate into `main.rs` — `run_server`

The two notification call sites in `run_server()`:

```rust
async fn run_server(config_path: Option<PathBuf>) -> Result<()> {
    info!("Starting spacebot-homelab-mcp server");

    // --- Config + Connection setup ---
    let config = match Config::load(config_path) {
        Ok(config) => Arc::new(config),
        Err(error) => {
            notifications::notify_failed(&format!("Config error: {error}"));
            return Err(error);
        }
    };

    let manager = match ConnectionManager::new((*config).clone()).await {
        Ok(manager) => Arc::new(manager),
        Err(error) => {
            notifications::notify_failed(&format!("Connection error: {error}"));
            return Err(error);
        }
    };

    // ... health monitor, server setup ...

    let server = HomelabMcpServer::new(config, manager.clone(), audit);

    // Count tools from the server's tool router before entering the MCP loop
    let tool_count = server.tool_count();
    let tool_summary = server.tool_summary();

    let transport = rmcp::transport::io::stdio();
    let service = rmcp::serve_server(server, transport).await?;

    // --- Success notification ---
    notifications::notify_connected(tool_count, &tool_summary);

    // ... existing select! loop ...
}
```

### Step 4: Add Tool Introspection to `HomelabMcpServer`

Add two methods to `HomelabMcpServer` for the notification content:

```rust
impl HomelabMcpServer {
    /// Total number of registered tools.
    pub fn tool_count(&self) -> usize {
        // Derived from the tool_router or a static count based on
        // enabled tools in config.tools
        self.config.tools.enabled_count()
    }

    /// Human-readable summary, e.g. "Docker (5) | SSH (3) | Confirm (1)"
    pub fn tool_summary(&self) -> String {
        // Group enabled tools by prefix (docker.*, ssh.*, confirm_*)
        // and format as "Category (N) | Category (N)"
    }
}
```

If the `ToolRouter` doesn't expose a count method, derive it from `config.tools` (the enabled tool list).

### Step 5: Register the Module

In `main.rs`, add `mod notifications;` alongside the existing module declarations.

## File Changes Summary

| File | Change |
|------|--------|
| `Cargo.toml` | Add `notify-rust` optional dep, add `notifications` feature |
| `src/notifications.rs` | **New file** — `Notification`, `NotificationDispatcher`, `DesktopDispatcher`, `ThrottleGuard`, public helper functions |
| `src/main.rs` | Add `mod notifications;`, refactor `run_server` to call `notify_connected` / `notify_failed` at the two call sites |
| `src/mcp.rs` | Add `tool_count()` and `tool_summary()` methods to `HomelabMcpServer` |

## Future Integration Path

When Spacebot adds generic MCP notification support (e.g., an `McpStatusChanged` SSE event and a toast in the UI):

1. Add an `McpProtocolDispatcher` that implements `NotificationDispatcher` by emitting MCP protocol `notifications/` messages.
2. The call sites in `main.rs` don't change — only the dispatcher implementation swaps.
3. Disable the `notifications` feature flag to remove the desktop notification dependency.
4. The `Notification` struct and trait remain as the stable interface.

## Testing

- **Manual**: Start the MCP server with a valid config — confirm the "Connected" desktop notification appears with correct tool count. Start with an invalid SSH config — confirm the "Failed" notification appears with the error.
- **Throttle**: Kill and restart the MCP server rapidly (<60s apart) — confirm only one failure notification appears per 60s window. Then start with a valid config — confirm the success notification fires regardless of throttle state.
- **Feature flag**: Build with `--no-default-features` — confirm the binary compiles and runs without `notify-rust`, with no notifications.
