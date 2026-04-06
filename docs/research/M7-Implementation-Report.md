# M7 Implementation Report — `docker.image.*` Tool Namespace

## Summary

Implemented 5 Docker image management tools for the `spacebot-homelab-mcp` MCP server:

| Tool                   | Description                                      | Destructive | Confirmation |
|------------------------|--------------------------------------------------|-------------|--------------|
| `docker.image.list`    | List Docker images with optional filtering       | No          | No           |
| `docker.image.pull`    | Pull an image from a registry (dry_run support)  | No          | No           |
| `docker.image.inspect` | Inspect image metadata with env redaction         | No          | No           |
| `docker.image.delete`  | Delete an image (confirmation + dry_run + force) | Yes         | Yes          |
| `docker.image.prune`   | Prune unused images (confirmation + dry_run)     | Yes         | Yes          |

## Files Changed

### Created
- **`src/tools/docker_image.rs`** — Complete implementation of all 5 tool functions plus 2 confirmed-execution helpers and an env redaction helper. Includes 4 unit tests for `redact_image_env`.

### Modified
- **`src/tools/mod.rs`** — Added `pub mod docker_image;` module declaration.
- **`src/mcp.rs`** — Added:
  - `use crate::tools::docker_image;` import
  - 5 args structs (`DockerImageListArgs`, `DockerImagePullArgs`, `DockerImageInspectArgs`, `DockerImageDeleteArgs`, `DockerImagePruneArgs`)
  - 5 entries in `ALL_TOOLS` constant
  - 5 tool handlers in the `#[tool_router]` impl block
  - 2 confirmation arms in the `confirm_operation` match block (`docker.image.delete`, `docker.image.prune`)
- **`example.config.toml`** — Added:
  - 3 rate limit entries (`docker.image.*`, `docker.image.delete`, `docker.image.prune`)
  - 5 image tools in the commented-out `tools.enabled` list
  - 2 confirmation rule examples (`delete_images`, `prune_images`)

## M6 Conflict Handling

M6 had already modified `src/mcp.rs` and `example.config.toml` before this milestone ran:
- M6 added `DockerContainerDeleteArgs`, `DockerContainerCreateArgs`, 2 tools to `ALL_TOOLS`, 2 handlers, 1 confirmation arm, plus `metrics` integration and updated rate limiter API (`check(tool_name, caller_id)`)
- All M7 additions were placed after M6's entries without touching container-related code
- Image tool handlers follow the same `record_tool_call` pattern established by M6

## Deviations from Provided Code

1. **`repo_tags` type**: bollard 0.17's `ImageSummary.repo_tags` is `Vec<String>` (not `Option<Vec<String>>`). Removed the `.unwrap_or_default()` call.
2. **`space_reclaimed` type**: `ImagePruneResponse.space_reclaimed` is `Option<i64>` (not `i64`). Added `.unwrap_or(0)`.
3. **Metrics integration**: Added `Instant::now()` + `self.record_tool_call()` to all 5 handlers to match M6's established pattern.
4. **Rate limiter API**: Used `self.rate_limiter.check(tool_name, caller_id)` signature (2 args) matching M6's updated API.

## Compilation Status

**M7-owned files compile without errors.** The crate has 17 pre-existing compilation errors in `src/connection.rs` and `src/tools/ssh.rs` (files owned by other milestones), none in M7-owned files. Verified by filtering `cargo check` output — zero errors reference `docker_image`, `mcp.rs`, or `tools/mod.rs`.

## Test Status

Unit tests for `redact_image_env` are included in `src/tools/docker_image.rs` (4 tests). They cannot run until the pre-existing compilation errors in other files are resolved, since this is a `[[bin]]` crate (no `--lib` target).
