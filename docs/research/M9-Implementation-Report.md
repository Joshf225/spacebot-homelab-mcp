# M9 Implementation Report — Prometheus Metrics & HTTP Endpoint

## Summary

M9 adds Prometheus-compatible metrics collection and an optional HTTP `/metrics` endpoint to the `spacebot-homelab-mcp` server.

## Files Modified

| File | Change |
|---|---|
| `Cargo.toml` | Added `prometheus` (0.13) and `axum` (0.7, optional) dependencies; added `metrics` feature (default-enabled) |
| `src/config.rs` | Added `MetricsConfig` struct with `enabled` and `listen` fields; added `metrics` field to `Config` |
| `src/metrics.rs` | **NEW** — `Metrics` struct with 9 metric families (tool calls, duration, SSH pool gauges, Docker/SSH health, confirmation tokens); `spawn_metrics_server` function |
| `src/mcp.rs` | Added `metrics: Option<Arc<Metrics>>` field; updated `new()` to accept metrics; added `record_tool_call` helper; instrumented all 9 tool handlers + `confirm_operation` |
| `src/connection.rs` | Added `metrics: Option<Arc<Metrics>>` field to `ConnectionManager`; updated `new()` to accept metrics; instrumented Docker and SSH health checks in `spawn_health_monitor`; updated test struct literal |
| `src/main.rs` | Added `mod metrics;`; create metrics before ConnectionManager; pass to both ConnectionManager and HomelabMcpServer; start metrics HTTP server if configured; abort on shutdown |
| `example.config.toml` | Added commented `[metrics]` config section |

## Metrics Registered

All metric names are prefixed with `homelab_`:

| Metric | Type | Labels | Description |
|---|---|---|---|
| `tool_calls_total` | Counter | tool, status | Total MCP tool invocations |
| `tool_duration_seconds` | Histogram | tool | Tool call duration in seconds |
| `ssh_pool_active_sessions` | Gauge | host | Active SSH sessions |
| `ssh_pool_idle_sessions` | Gauge | host | Idle SSH sessions |
| `ssh_pool_total_sessions` | Gauge | host | Total SSH sessions |
| `docker_connection_healthy` | Gauge | host | Docker health (1=up, 0=down) |
| `ssh_connection_healthy` | Gauge | host | SSH health (1=up, 0=down) |
| `confirmation_tokens_issued_total` | Counter | tool | Confirmation tokens issued |
| `confirmation_tokens_resolved_total` | Counter | outcome | Tokens confirmed/expired/rejected |

## Tool Handler Instrumentation

All 9 existing tool handlers are instrumented with `Instant::now()` timing and `record_tool_call()`:
- `docker.container.list`
- `docker.container.start`
- `docker.container.stop`
- `docker.container.logs`
- `docker.container.inspect`
- `ssh.exec`
- `ssh.upload`
- `ssh.download`
- `confirm_operation`

## Compilation Status

**M9 files compile cleanly** — zero errors in `src/metrics.rs`, `src/mcp.rs`, `src/main.rs`, `src/config.rs`, or `example.config.toml`.

**Pre-existing M10 conflicts prevent full compilation.** There are 56 errors in `src/connection.rs` (lines 255-662) and `src/tools/ssh.rs` caused by M10's incomplete `SshPool` refactor: the struct was changed to use `Vec<SharedSession>` but the method implementations still reference `VecDeque`, `PooledSession`, `active_count`, and `total_count`. These are entirely within M10's ownership and do not involve M9 code.

**Tests cannot run** due to the M10 compilation errors. Once M10 resolves its `SshPool` struct/method mismatch, all tests should pass.

## Conflict Awareness

- **M8**: Compatible. M8 changed `RateLimiter::from_config` signature and `rate_limiter.check` to accept `caller_id`. M9 works with these signatures as-is.
- **M10**: M10 rewrote `SshPool` struct (SharedSession, channel multiplexing) but left old method bodies referencing `PooledSession`/`AtomicUsize`. M9 only adds the `metrics` field and health monitor instrumentation, which are orthogonal to M10's pool changes.
- **M6/M7**: Not yet merged. When additional tool handlers are added, they should follow the same `record_tool_call` pattern.
