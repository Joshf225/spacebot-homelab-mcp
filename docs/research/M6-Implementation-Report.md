# M6 Implementation Report: docker.container.delete & docker.container.create

## What Was Implemented

### 1. `docker.container.delete` tool
- **Dry-run support** (Layer 3): Preview deletion without executing
- **Confirmation flow** (Layer 8): Integrates with `ConfirmationManager` for irreversible operations
- **Pre-flight safety checks**:
  - Verifies container exists before attempting deletion
  - Blocks deletion of running containers unless `force=true`
  - Blocks deletion of containers with attached volumes unless `force=true`
- **Data safety**: Anonymous volumes are NOT removed by default (`v: false`)
- **Audit logging**: All operations (dry_run, confirmation_required, success, error) are logged
- **`container_delete_confirmed`**: Separate function for post-confirmation execution, called directly by `confirm_operation`

### 2. `docker.container.create` tool
- **Dry-run support** (Layer 3): Preview creation configuration without executing
- **Container name validation**: Rejects names with invalid characters (injection prevention)
- **Port mapping**: Accepts `{ "host_port": "container_port" }` format, auto-appends `/tcp` protocol
- **Environment variables**: Accepts `["KEY=value"]` format
- **Volume binds**: Accepts `["/host/path:/container/path"]` format
- **Restart policy**: Supports `no`, `always`, `unless-stopped`, `on-failure` (with 3 retries)
- **Create-only**: Container is created but NOT started (explicit start required)
- **Audit logging**: All operations logged

### 3. Confirmation flow for delete
- Added `docker.container.delete` arm to `confirm_operation` match block
- Deserializes `DockerContainerDeleteArgs` from stored params
- Calls `container_delete_confirmed` directly after confirmation

## Files Changed

| File | Lines Before | Lines After | Lines Added |
|------|-------------|-------------|-------------|
| `src/tools/docker.rs` | 365 | 727 | +362 |
| `src/mcp.rs` | 494 | 576 | +82 |
| `example.config.toml` | 137 | 143 | +6 |
| **Total** | | | **+450** |

### Detailed changes per file:

**`src/tools/docker.rs`**:
- Added imports: `CreateContainerOptions`, `Config as ContainerConfig`, `RemoveContainerOptions` from `bollard::container`; `HostConfig`, `PortBinding`, `RestartPolicy`, `RestartPolicyNameEnum` from `bollard::models`; `ConfirmationManager` from crate
- Added `container_delete()` function (~90 lines)
- Added `container_delete_confirmed()` function (~90 lines)
- Added `container_create()` function (~170 lines)
- Added `test_container_name_validation` unit test

**`src/mcp.rs`**:
- Added `use std::collections::HashMap` import
- Added `DockerContainerDeleteArgs` struct (6 fields)
- Added `DockerContainerCreateArgs` struct (8 fields)
- Added 2 entries to `ALL_TOOLS` constant
- Added `docker_container_delete` tool handler with metrics recording
- Added `docker_container_create` tool handler with metrics recording
- Added `"docker.container.delete"` arm in `confirm_operation` match block

**`example.config.toml`**:
- Added `docker.container.delete` and `docker.container.create` to commented tools list
- Added `[confirm.delete_containers]` example rule

## Deviations from Plan

1. **Metrics integration**: The codebase had been updated by a parallel milestone (M7) to include `record_tool_call` for metrics. The new tool handlers follow the same pattern with `Instant::now()` + `self.record_tool_call()`.

2. **Rate limiter API change**: Another parallel milestone changed `rate_limiter.check()` to accept a `caller_id` parameter. The new code follows the updated API.

3. **`Config` vs `ContainerConfig`**: Bollard 0.17 uses `bollard::container::Config<T>` for container configuration (not `ContainerConfig`). Imported as `Config as ContainerConfig` to keep code readable.

4. **`PortBinding` field names**: Bollard stubs use `host_ip` and `host_port` (snake_case), matching the code as written.

## Test Results

- **Unit test** `test_container_name_validation`: Ready to run (validates name character restrictions)
- **Compilation**: All M6-owned files (`docker.rs`, `mcp.rs`) compile without errors
- **Build blocked by**: Parallel milestones have incomplete work in `connection.rs` (missing `metrics` module, `PooledSession` type, `VecDeque`/`AtomicUsize` imports), `main.rs` (missing metrics argument), and `ssh.rs` (type inference issues). **Zero errors originate from M6 files.**

## Known Issues / TODOs

- `container_create` does not require confirmation (by design -- creation is non-destructive and reversible via delete)
- Volume removal is intentionally disabled (`v: false` in `RemoveContainerOptions`) for data safety
- Container name validation is basic (alphanumeric + `_.-`); Docker allows `/` prefix for names which we don't support
- No `docker.container.start` is auto-called after create (by design -- explicit start is safer)
