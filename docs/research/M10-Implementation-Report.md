# M10 Implementation Report — SSH Channel Multiplexing

## Summary

Refactored the SSH connection pool from exclusive session checkout (`PooledSession` + `ssh_checkout`/`ssh_return`) to shared channel multiplexing (`SharedSession` + `AcquiredChannel` + `ssh_acquire_channel`/`ssh_release_channel`). Multiple concurrent operations can now share a single SSH session, reducing connection overhead and improving throughput.

## Files Changed

### `src/config.rs`
- Added `max_channels_per_session: usize` field to `SshPoolConfig` with `#[serde(default = "default_max_channels")]`
- Added `default_max_channels()` returning `10`
- Updated `Default` impl for `SshPoolConfig`

### `src/connection.rs`
- **Removed**: `PooledSession` struct, `VecDeque` import, `AtomicUsize`/`Ordering` imports, `active_count`/`total_count` atomics on `SshPool`
- **Added**: `SharedSession` (private) with `active_channels: usize` field
- **Added**: `AcquiredChannel` (public) with `Arc<Handle>`, `session_index`, `host_name`
- **Replaced pool internals**: `VecDeque<PooledSession>` → `Vec<SharedSession>`; added `max_channels_per_session` field
- **`acquire_channel()`**: Load-balances across sessions (min active channels), creates new sessions when under limit, waits with timeout when at capacity
- **`release_channel()`**: Decrements `active_channels`, notifies waiters; if `broken`, removes session only when no other channels are active
- **`validate_session_age()`**: Simplified from `validate_session()` — checks lifetime only (no idle-time check, since shared sessions are rarely idle)
- **`create_shared_session()`**: Renamed from `create_session()`, wraps handle in `Arc` for shared ownership
- **`cleanup_stale_sessions()`**: Simplified — retains sessions with `active_channels > 0`, removes expired/idle sessions without keepalive probing
- **`check_connectivity()`**: Updated to work with new session structure
- **`close_all()`**: Clears all sessions
- **`session_count()` / `active_channel_count()`**: New public helpers for observability
- **`ssh_acquire_channel()` / `ssh_release_channel()`**: New `ConnectionManager` convenience methods replacing `ssh_checkout`/`ssh_return`
- **M9 preservation**: All M9 changes preserved — `metrics` field, `ConnectionManager::new` signature, health monitor metrics instrumentation, test with `metrics: None`

### `src/tools/ssh.rs`
- `exec_confirmed()`: `ssh_checkout` → `ssh_acquire_channel`, `session` → `acquired`, `ssh_return` → `ssh_release_channel`
- `upload()`: Same migration
- `download()`: Same migration
- No logic changes — only API rename/migration

### `example.config.toml`
- Added `max_channels_per_session = 10` to `[ssh.pool]` section with comment

## Design Decisions

1. **`Arc<Handle>` wrapping**: `russh::client::Handle` does not implement `Clone`. Wrapping in `Arc` allows the pool to retain ownership while lending a reference to `AcquiredChannel` callers. The `Arc::clone()` is cheap (pointer + atomic increment).

2. **Load balancing**: `acquire_channel` picks the session with the fewest active channels, spreading load evenly. This avoids hot-spotting a single session.

3. **Broken session handling**: When `release_channel` is called with `broken = true`, the session is only removed if it has no other active channels. If other channels are still using it, we just decrement the count and let it drain naturally.

4. **No idle-time eviction for active sessions**: `validate_session_age` only checks max lifetime, not idle time. Shared sessions with `active_channels > 0` are never considered idle. `cleanup_stale_sessions` handles idle eviction for sessions with 0 active channels.

5. **Backward-compatible config**: `max_channels_per_session` uses `serde(default)`, so existing configs without this field default to 10.

## Verification

- `cargo check`: Passes with only dead-code warnings (M9 metrics fields not yet wired, and `session_count`/`active_channel_count` reserved for future use)
- `cargo test`: All 41 tests pass (38 unit + 3 integration)
- No pre-existing compile errors encountered
