# M6-M10 Implementation Plan: Post-PoC Enhancements

This document provides detailed, step-by-step implementation plans for milestones M6 through M10 — the post-PoC enhancements for `spacebot-homelab-mcp`. These build on the fully implemented M1-M5 foundation.

**Source of truth references:**
- `spacebot/homelab-integration/security-approach.md` — Layer 3 (safety gates), Layer 7 (rate limiting), Layer 8 (confirmation)
- `spacebot/homelab-integration/architecture-decision.md` — `docker.image.*` namespace (line 147)
- `spacebot/homelab-integration/connection-manager.md` — SSH channel multiplexing (lines 185-189)

**Prerequisite:** M1-M5 fully implemented (9 tools working, 33 tests passing).

---

# Milestone 6: Destructive Docker Tools (2-3 days)

**Goal:** Add `docker.container.delete` and `docker.container.create` — the first truly destructive Docker tools. These fully exercise Layer 8 `"always"` confirmation and Layer 3 safety gates (`dry_run`/`force`).

**Design doc backing:**
- `security-approach.md` lines 58-60: tools listed as never-enabled-by-default
- `security-approach.md` lines 75-108: full implementation sketch for `docker.container.delete`
- `security-approach.md` lines 111-114: `dry_run`/`force` required on all destructive tools
- `security-approach.md` lines 280-282: `docker.container.delete = "always"` confirmation

**Files modified:**
| File | Change |
|------|--------|
| `src/tools/docker.rs` | Add `container_delete` and `container_create` functions |
| `src/mcp.rs` | Add args structs, tool registrations, confirm_operation arms |
| `example.config.toml` | Add example config for new tools |

---

## Step 1: Add `container_delete` to `tools/docker.rs`

Add the function after the existing `container_inspect` function. This implements the full safety gate flow from security-approach.md Layer 3.

```rust
use bollard::container::RemoveContainerOptions;

pub async fn container_delete(
    manager: Arc<ConnectionManager>,
    confirmation: Arc<ConfirmationManager>,
    host: Option<String>,
    container: String,
    dry_run: Option<bool>,
    force: Option<bool>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let host = host.unwrap_or_else(|| "local".to_string());
    let force = force.unwrap_or(false);

    // Layer 3: dry_run support
    if dry_run.unwrap_or(false) {
        let output = format!(
            "DRY RUN: Would delete container '{}' on Docker host '{}'. \
             force={}. Set dry_run=false to execute.",
            container, host, force
        );
        audit
            .log("docker.container.delete", &host, "dry_run", Some(&container))
            .await
            .ok();
        return Ok(wrap_output_envelope("docker.container.delete", &output));
    }

    // Layer 8: Confirmation flow — must happen BEFORE any execution
    let params_json = serde_json::json!({
        "host": host,
        "container": container,
        "force": force,
    })
    .to_string();

    if let Some(response) = confirmation
        .check_and_maybe_require(
            "docker.container.delete",
            None, // No command text for docker tools — confirmation is "always"
            &format!(
                "About to DELETE container '{}' on Docker host '{}'. This is irreversible.",
                container, host
            ),
            &params_json,
        )
        .await?
    {
        audit
            .log(
                "docker.container.delete",
                &host,
                "confirmation_required",
                Some(&container),
            )
            .await
            .ok();
        return Ok(response);
    }

    // Execution proceeds only after confirmation (or if no confirmation rule configured)
    container_delete_confirmed(manager, host, container, force, audit).await
}

/// Execute container delete after confirmation has been satisfied.
/// Called directly by `confirm_operation` for confirmed tokens.
pub async fn container_delete_confirmed(
    manager: Arc<ConnectionManager>,
    host: String,
    container: String,
    force: bool,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let result: Result<String> = async {
        let docker = manager.get_docker(&host)?;

        // Pre-flight: verify container exists and get its state
        let details = docker
            .as_bollard()
            .inspect_container(&container, None::<InspectContainerOptions>)
            .await
            .map_err(|error| {
                anyhow!("Container '{}' not found or inaccessible: {}", container, error)
            })?;

        // Pre-flight: check if container is running
        let is_running = details
            .state
            .as_ref()
            .and_then(|state| state.running)
            .unwrap_or(false);

        if is_running && !force {
            return Err(anyhow!(
                "Container '{}' is currently running. Stop it first, or set force=true to force-remove.",
                container
            ));
        }

        // Pre-flight: warn about attached volumes (per security-approach.md lines 96-102)
        let volume_count = details
            .mounts
            .as_ref()
            .map(|mounts| mounts.len())
            .unwrap_or(0);

        if volume_count > 0 && !force {
            return Err(anyhow!(
                "Container '{}' has {} volume(s) attached. Data may be lost. \
                 Set force=true to override.",
                container,
                volume_count
            ));
        }

        // Execute deletion
        // force: true sends SIGKILL if running (when force param is set)
        // v: false — do NOT remove anonymous volumes by default (data safety)
        docker
            .as_bollard()
            .remove_container(
                &container,
                Some(RemoveContainerOptions {
                    force,
                    v: false,
                    ..Default::default()
                }),
            )
            .await
            .map_err(|error| {
                anyhow!("Failed to delete container '{}': {}", container, error)
            })?;

        let mut output = format!("Deleted container '{}' on Docker host '{}'.", container, host);
        if volume_count > 0 {
            output.push_str(&format!(
                " Note: {} volume(s) were attached. Anonymous volumes were NOT removed.",
                volume_count
            ));
        }

        Ok(output)
    }
    .await;

    match result {
        Ok(output) => {
            audit
                .log("docker.container.delete", &host, "success", Some(&container))
                .await
                .ok();
            Ok(wrap_output_envelope("docker.container.delete", &output))
        }
        Err(error) => {
            audit
                .log(
                    "docker.container.delete",
                    &host,
                    "error",
                    Some(&error.to_string()),
                )
                .await
                .ok();
            Err(error)
        }
    }
}
```

**Key design decisions:**
- `v: false` on `RemoveContainerOptions` — anonymous volumes are NOT removed to prevent accidental data loss. This is a deliberate safety choice.
- Running containers require `force=true` (matching the security-approach.md sketch).
- Volume-attached containers require `force=true` (matching lines 96-102 of security-approach.md).
- The `container_delete_confirmed` function is public so that `confirm_operation` in `mcp.rs` can call it after token validation.

**Add import at the top of `tools/docker.rs`:**
```rust
use bollard::container::RemoveContainerOptions;
use crate::confirmation::ConfirmationManager;
```

---

## Step 2: Add `container_create` to `tools/docker.rs`

`docker.container.create` is mentioned in security-approach.md (line 60) but has no implementation sketch. This design follows the patterns established by the existing tools.

```rust
use bollard::container::{CreateContainerOptions, Config as ContainerConfig};
use bollard::models::{HostConfig, PortBinding, RestartPolicy, RestartPolicyNameEnum};

pub async fn container_create(
    manager: Arc<ConnectionManager>,
    host: Option<String>,
    image: String,
    name: String,
    ports: Option<HashMap<String, String>>,
    env: Option<Vec<String>>,
    volumes: Option<Vec<String>>,
    restart_policy: Option<String>,
    dry_run: Option<bool>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let host = host.unwrap_or_else(|| "local".to_string());

    // Validate name: Docker container names must match [a-zA-Z0-9][a-zA-Z0-9_.-]
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-') {
        return Err(anyhow!(
            "Invalid container name '{}'. Must contain only alphanumeric characters, underscores, dots, or hyphens.",
            name
        ));
    }

    // Layer 3: dry_run support
    if dry_run.unwrap_or(false) {
        let output = format!(
            "DRY RUN: Would create container '{}' from image '{}' on Docker host '{}'.\n\
             Ports: {}\n\
             Env vars: {} configured\n\
             Volumes: {}\n\
             Restart policy: {}",
            name,
            image,
            host,
            ports
                .as_ref()
                .map(|p| format!("{:?}", p))
                .unwrap_or_else(|| "none".to_string()),
            env.as_ref().map(|e| e.len()).unwrap_or(0),
            volumes
                .as_ref()
                .map(|v| format!("{:?}", v))
                .unwrap_or_else(|| "none".to_string()),
            restart_policy.as_deref().unwrap_or("no"),
        );
        audit
            .log("docker.container.create", &host, "dry_run", Some(&name))
            .await
            .ok();
        return Ok(wrap_output_envelope("docker.container.create", &output));
    }

    let result: Result<String> = async {
        let docker = manager.get_docker(&host)?;

        // Build port bindings: "8080:80" → container port 80/tcp → host port 8080
        let mut port_bindings: HashMap<String, Option<Vec<PortBinding>>> = HashMap::new();
        let mut exposed_ports: HashMap<String, HashMap<(), ()>> = HashMap::new();

        if let Some(ref port_map) = ports {
            for (host_port, container_port) in port_map {
                let container_key = if container_port.contains('/') {
                    container_port.clone()
                } else {
                    format!("{}/tcp", container_port)
                };

                exposed_ports.insert(container_key.clone(), HashMap::new());
                port_bindings.insert(
                    container_key,
                    Some(vec![PortBinding {
                        host_ip: Some("0.0.0.0".to_string()),
                        host_port: Some(host_port.clone()),
                    }]),
                );
            }
        }

        // Build volume binds: "/host/path:/container/path" format
        let binds = volumes.clone();

        // Build restart policy
        let restart = restart_policy.as_deref().map(|policy| {
            match policy {
                "always" => RestartPolicy {
                    name: Some(RestartPolicyNameEnum::ALWAYS),
                    maximum_retry_count: None,
                },
                "unless-stopped" => RestartPolicy {
                    name: Some(RestartPolicyNameEnum::UNLESS_STOPPED),
                    maximum_retry_count: None,
                },
                "on-failure" => RestartPolicy {
                    name: Some(RestartPolicyNameEnum::ON_FAILURE),
                    maximum_retry_count: Some(3),
                },
                _ => RestartPolicy {
                    name: Some(RestartPolicyNameEnum::NO),
                    maximum_retry_count: None,
                },
            }
        });

        let host_config = HostConfig {
            port_bindings: if port_bindings.is_empty() {
                None
            } else {
                Some(port_bindings)
            },
            binds,
            restart_policy: restart,
            ..Default::default()
        };

        let container_config = ContainerConfig {
            image: Some(image.clone()),
            env: env.clone(),
            exposed_ports: if exposed_ports.is_empty() {
                None
            } else {
                Some(exposed_ports)
            },
            host_config: Some(host_config),
            ..Default::default()
        };

        let response = docker
            .as_bollard()
            .create_container(
                Some(CreateContainerOptions {
                    name: name.as_str(),
                    platform: None,
                }),
                container_config,
            )
            .await
            .map_err(|error| {
                anyhow!("Failed to create container '{}': {}", name, error)
            })?;

        let mut output = format!(
            "Created container '{}' (ID: {}) from image '{}' on Docker host '{}'.",
            name,
            response.id.chars().take(12).collect::<String>(),
            image,
            host
        );

        if !response.warnings.is_empty() {
            output.push_str("\nWarnings:");
            for warning in &response.warnings {
                output.push_str(&format!("\n  - {}", warning));
            }
        }

        output.push_str("\n\nContainer created but NOT started. Use docker.container.start to start it.");

        Ok(output)
    }
    .await;

    match result {
        Ok(output) => {
            audit
                .log("docker.container.create", &host, "success", Some(&name))
                .await
                .ok();
            Ok(wrap_output_envelope("docker.container.create", &output))
        }
        Err(error) => {
            audit
                .log(
                    "docker.container.create",
                    &host,
                    "error",
                    Some(&error.to_string()),
                )
                .await
                .ok();
            Err(error)
        }
    }
}
```

**Key design decisions:**
- Container is created but NOT started. Starting is a separate operation via the existing `docker.container.start` tool. This matches Docker CLI behavior (`docker create` vs `docker run`).
- Port mapping uses `HashMap<String, String>` where key=host_port, value=container_port. This is LLM-friendly.
- Volume binds use Docker format: `"/host/path:/container/path"`.
- Restart policy supports: `"always"`, `"unless-stopped"`, `"on-failure"`, `"no"` (default).
- Name validation prevents injection (alphanumeric + `_.-` only).
- No confirmation flow on `docker.container.create` — creating a container is not inherently destructive. The design docs only specify confirmation for `delete`.

---

## Step 3: Add MCP args structs and tool registrations in `mcp.rs`

### 3a. Add new args structs (after existing structs, ~line 72)

```rust
#[derive(Debug, Deserialize, JsonSchema)]
struct DockerContainerDeleteArgs {
    /// Docker host name (defaults to "local")
    pub host: Option<String>,
    /// Container name or ID to delete
    pub container: String,
    /// Preview the operation without executing (recommended: use dry_run=true first)
    pub dry_run: Option<bool>,
    /// Override safety checks (required for running containers or containers with volumes)
    pub force: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct DockerContainerCreateArgs {
    /// Docker host name (defaults to "local")
    pub host: Option<String>,
    /// Docker image to use (e.g. "nginx:latest")
    pub image: String,
    /// Container name
    pub name: String,
    /// Port mappings: { "host_port": "container_port" } (e.g. { "8080": "80" })
    pub ports: Option<HashMap<String, String>>,
    /// Environment variables (e.g. ["KEY=value", "DEBUG=1"])
    pub env: Option<Vec<String>>,
    /// Volume binds (e.g. ["/host/path:/container/path"])
    pub volumes: Option<Vec<String>>,
    /// Restart policy: "no", "always", "unless-stopped", "on-failure"
    pub restart_policy: Option<String>,
    /// Preview the operation without executing
    pub dry_run: Option<bool>,
}
```

**Note:** Add `use std::collections::HashMap;` to the imports in `mcp.rs` if not already present.

### 3b. Update `ALL_TOOLS` constant

```rust
const ALL_TOOLS: &'static [(&'static str, &'static str)] = &[
    ("docker.container.list",    "Docker"),
    ("docker.container.start",   "Docker"),
    ("docker.container.stop",    "Docker"),
    ("docker.container.logs",    "Docker"),
    ("docker.container.inspect", "Docker"),
    ("docker.container.delete",  "Docker"),  // NEW
    ("docker.container.create",  "Docker"),  // NEW
    ("ssh.exec",                 "SSH"),
    ("ssh.upload",               "SSH"),
    ("ssh.download",             "SSH"),
    ("confirm_operation",        "Confirm"),
];
```

### 3c. Add tool handlers in `#[tool_router]` impl

```rust
#[tool(
    name = "docker.container.delete",
    description = "Delete a Docker container. IMPORTANT: Use dry_run=true first to preview. \
                   Requires force=true for running containers or containers with attached volumes. \
                   This operation is irreversible and requires confirmation."
)]
async fn docker_container_delete(
    &self,
    Parameters(args): Parameters<DockerContainerDeleteArgs>,
) -> Result<String, String> {
    self.ensure_tool_available("docker.container.delete")?;
    docker::container_delete(
        self.manager.clone(),
        self.confirmation.clone(),
        args.host,
        args.container,
        args.dry_run,
        args.force,
        self.audit.clone(),
    )
    .await
    .map_err(|error| error.to_string())
}

#[tool(
    name = "docker.container.create",
    description = "Create a new Docker container (does NOT start it). \
                   Use docker.container.start after creation to run it. \
                   Use dry_run=true first to preview the configuration."
)]
async fn docker_container_create(
    &self,
    Parameters(args): Parameters<DockerContainerCreateArgs>,
) -> Result<String, String> {
    self.ensure_tool_available("docker.container.create")?;
    docker::container_create(
        self.manager.clone(),
        args.host,
        args.image,
        args.name,
        args.ports,
        args.env,
        args.volumes,
        args.restart_policy,
        args.dry_run,
        self.audit.clone(),
    )
    .await
    .map_err(|error| error.to_string())
}
```

### 3d. Add `docker.container.delete` arm to `confirm_operation` handler

In the `confirm_operation` method's match block (after the existing `docker.container.stop` arm), add:

```rust
"docker.container.delete" => {
    let params: DockerContainerDeleteArgs =
        serde_json::from_str(&original_params_json)
            .map_err(|error| error.to_string())?;
    self.audit
        .log(
            "docker.container.delete",
            params.host.as_deref().unwrap_or("local"),
            "confirmed_exec",
            Some(&params.container),
        )
        .await
        .ok();
    docker::container_delete_confirmed(
        self.manager.clone(),
        params.host.unwrap_or_else(|| "local".to_string()),
        params.container,
        params.force.unwrap_or(false),
        self.audit.clone(),
    )
    .await
    .map_err(|error| error.to_string())
}
```

---

## Step 4: Update `example.config.toml`

Add the new tools to the commented-out `[tools]` section and the rate limits:

```toml
# [tools]
# enabled = [
#     "docker.container.list",
#     "docker.container.start",
#     "docker.container.stop",
#     "docker.container.logs",
#     "docker.container.inspect",
#     "docker.container.delete",   # NEW — destructive, requires confirmation
#     "docker.container.create",   # NEW
#     "ssh.exec",
#     "ssh.upload",
#     "ssh.download",
# ]
```

Add confirmation rule to the `[confirm]` section:
```toml
# Confirmation rules for destructive operations (Layer 8 security)
[confirm]
"docker.container.delete" = "always"
```

**Important:** The confirm config format uses tool names as keys. The value `"always"` maps to `ConfirmRule::Always("always".to_string())`.

Rate limits already cover `docker.container.delete` via the existing `"docker.container.delete" = { per_minute = 1 }` line.

---

## Step 5: Unit tests

Add to `src/tools/docker.rs` `#[cfg(test)] mod tests`:

```rust
#[test]
fn test_container_name_validation() {
    // Valid names
    assert!("my-container".chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-'));
    assert!("webapp.v2".chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-'));

    // Invalid names
    assert!(!"my container".chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-'));
    assert!(!"rm -rf /".chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-'));
}
```

**Note:** Full integration tests for delete/create require a running Docker daemon and are added in M5-style validation (run with `--ignored`).

---

## Step 6: Verification

```bash
cargo check                              # Compiles without errors
cargo test --lib docker::tests           # Unit tests pass
cargo test --lib confirmation::tests     # Confirmation still works
cargo build --release                    # Binary builds
```

Manually test the confirmation flow:
1. Configure `[confirm] "docker.container.delete" = "always"`
2. Call `docker.container.delete` → should return `confirmation_required` with a token
3. Call `confirm_operation` with the token → should execute the delete

---

## M6 Deliverables

- [x] `docker.container.delete` — with dry_run, force, pre-flight checks, confirmation flow
- [x] `docker.container.create` — with dry_run, port/volume/env/restart config
- [x] MCP tool registrations (11 tools total: 7 Docker + 3 SSH + confirm_operation)
- [x] confirm_operation wired for docker.container.delete
- [x] example.config.toml updated
- [x] Unit tests

---
---

# Milestone 7: Docker Image Tools (2-3 days)

**Goal:** Add the `docker.image.*` tool namespace — image management (list, pull, inspect, delete, prune).

**Design doc backing:**
- `architecture-decision.md` line 147: `docker.image.*` in tool namespace diagram
- `security-approach.md` lines 61-62: `docker.image.delete` and `docker.image.prune` commented out (never-enabled-by-default)
- `security-approach.md` line 282: `docker.image.delete = "always"` confirmation

**Files modified/created:**
| File | Change |
|------|--------|
| `src/tools/docker_image.rs` | **New file** — all 5 image tool handlers |
| `src/tools/mod.rs` | Add `pub mod docker_image;` |
| `src/mcp.rs` | Add args structs, tool registrations, confirm_operation arms |
| `example.config.toml` | Add example config for new tools |

---

## Step 1: Create `src/tools/docker_image.rs`

Create a new file for image tools. This keeps the docker container tools and image tools in separate, focused files.

```rust
use anyhow::{Result, anyhow};
use bollard::image::{
    CreateImageOptions, ListImagesOptions, RemoveImageOptions,
};
use bollard::models::ImageInspect;
use futures::StreamExt;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

use crate::audit::AuditLogger;
use crate::confirmation::ConfirmationManager;
use crate::connection::ConnectionManager;
use crate::tools::{truncate_output, wrap_output_envelope};

const OUTPUT_MAX_CHARS: usize = 10_000;
```

---

## Step 2: Implement `image_list`

```rust
pub async fn image_list(
    manager: Arc<ConnectionManager>,
    host: Option<String>,
    all: Option<bool>,
    name_filter: Option<String>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let host = host.unwrap_or_else(|| "local".to_string());
    let result: Result<String> = async {
        let docker = manager.get_docker(&host)?;

        let mut filters = HashMap::new();
        if let Some(ref name) = name_filter {
            filters.insert("reference".to_string(), vec![name.clone()]);
        }

        let images = docker
            .as_bollard()
            .list_images(Some(ListImagesOptions::<String> {
                all: all.unwrap_or(false),
                filters,
                ..Default::default()
            }))
            .await
            .map_err(|error| anyhow!("Failed to list images: {}", error))?;

        if images.is_empty() {
            return Ok("No images found.".to_string());
        }

        let mut lines = vec![format!(
            "Docker host: {}\n\n{:<16}  {:<40}  {:<12}  {}",
            host, "ID", "REPOSITORY:TAG", "SIZE", "CREATED"
        )];

        for image in images {
            let id = image
                .id
                .chars()
                .skip(7) // skip "sha256:" prefix
                .take(12)
                .collect::<String>();

            let repo_tags = image
                .repo_tags
                .unwrap_or_default()
                .join(", ");
            let repo_tags = if repo_tags.is_empty() {
                "<none>".to_string()
            } else {
                repo_tags
            };

            let size_mb = image.size as f64 / 1_000_000.0;
            let size_str = format!("{:.1} MB", size_mb);

            let created = image.created;

            lines.push(format!(
                "{:<16}  {:<40}  {:<12}  {}",
                id, repo_tags, size_str, created
            ));
        }

        Ok(truncate_output(&lines.join("\n"), OUTPUT_MAX_CHARS))
    }
    .await;

    match result {
        Ok(output) => {
            audit
                .log("docker.image.list", &host, "success", None)
                .await
                .ok();
            Ok(wrap_output_envelope("docker.image.list", &output))
        }
        Err(error) => {
            audit
                .log("docker.image.list", &host, "error", Some(&error.to_string()))
                .await
                .ok();
            Err(error)
        }
    }
}
```

---

## Step 3: Implement `image_pull`

```rust
pub async fn image_pull(
    manager: Arc<ConnectionManager>,
    host: Option<String>,
    image: String,
    dry_run: Option<bool>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let host = host.unwrap_or_else(|| "local".to_string());

    if dry_run.unwrap_or(false) {
        let output = format!(
            "DRY RUN: Would pull image '{}' on Docker host '{}'.",
            image, host
        );
        audit
            .log("docker.image.pull", &host, "dry_run", Some(&image))
            .await
            .ok();
        return Ok(wrap_output_envelope("docker.image.pull", &output));
    }

    let result: Result<String> = async {
        let docker = manager.get_docker(&host)?;

        // Parse image:tag — default to "latest" if no tag specified
        let (from_image, tag) = if let Some(colon_pos) = image.rfind(':') {
            // Only split on colon if what follows doesn't contain '/' (not a port)
            let after_colon = &image[colon_pos + 1..];
            if after_colon.contains('/') {
                (image.as_str(), "latest")
            } else {
                (&image[..colon_pos], after_colon)
            }
        } else {
            (image.as_str(), "latest")
        };

        let options = CreateImageOptions {
            from_image,
            tag,
            ..Default::default()
        };

        let mut stream = docker.as_bollard().create_image(Some(options), None, None);
        let mut last_status = String::new();
        let mut layer_count = 0;

        while let Some(item) = stream.next().await {
            let info = item.map_err(|error| anyhow!("Image pull failed: {}", error))?;
            if let Some(status) = info.status {
                last_status = status;
            }
            if info.progress.is_some() {
                layer_count += 1;
            }
        }

        Ok(format!(
            "Pulled image '{}' on Docker host '{}'. Status: {}. Layers processed: {}.",
            image, host, last_status, layer_count
        ))
    }
    .await;

    match result {
        Ok(output) => {
            audit
                .log("docker.image.pull", &host, "success", Some(&image))
                .await
                .ok();
            Ok(wrap_output_envelope("docker.image.pull", &output))
        }
        Err(error) => {
            audit
                .log("docker.image.pull", &host, "error", Some(&error.to_string()))
                .await
                .ok();
            Err(error)
        }
    }
}
```

---

## Step 4: Implement `image_inspect`

```rust
pub async fn image_inspect(
    manager: Arc<ConnectionManager>,
    host: Option<String>,
    image: String,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let host = host.unwrap_or_else(|| "local".to_string());
    let result: Result<String> = async {
        let docker = manager.get_docker(&host)?;

        let details: ImageInspect = docker
            .as_bollard()
            .inspect_image(&image)
            .await
            .map_err(|error| anyhow!("Failed to inspect image '{}': {}", image, error))?;

        let mut value = serde_json::to_value(details)?;
        // Redact any env values in the image config (same pattern as container inspect)
        redact_image_env(&mut value);

        let pretty = serde_json::to_string_pretty(&value)?;
        Ok(truncate_output(&pretty, OUTPUT_MAX_CHARS))
    }
    .await;

    match result {
        Ok(output) => {
            audit
                .log("docker.image.inspect", &host, "success", Some(&image))
                .await
                .ok();
            Ok(wrap_output_envelope("docker.image.inspect", &output))
        }
        Err(error) => {
            audit
                .log(
                    "docker.image.inspect",
                    &host,
                    "error",
                    Some(&error.to_string()),
                )
                .await
                .ok();
            Err(error)
        }
    }
}

fn redact_image_env(value: &mut Value) {
    // Image config env is at .Config.Env or .ContainerConfig.Env
    for key in &["Config", "ContainerConfig"] {
        if let Some(env_values) = value
            .get_mut(key)
            .and_then(|config| config.get_mut("Env"))
            .and_then(Value::as_array_mut)
        {
            for env_value in env_values {
                if let Some(entry) = env_value.as_str() {
                    let redacted = entry
                        .split_once('=')
                        .map(|(k, _)| format!("{}=<redacted>", k))
                        .unwrap_or_else(|| entry.to_string());
                    *env_value = Value::String(redacted);
                }
            }
        }
    }
}
```

---

## Step 5: Implement `image_delete`

```rust
pub async fn image_delete(
    manager: Arc<ConnectionManager>,
    confirmation: Arc<ConfirmationManager>,
    host: Option<String>,
    image: String,
    force: Option<bool>,
    dry_run: Option<bool>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let host = host.unwrap_or_else(|| "local".to_string());
    let force = force.unwrap_or(false);

    if dry_run.unwrap_or(false) {
        let output = format!(
            "DRY RUN: Would delete image '{}' on Docker host '{}'. force={}.",
            image, host, force
        );
        audit
            .log("docker.image.delete", &host, "dry_run", Some(&image))
            .await
            .ok();
        return Ok(wrap_output_envelope("docker.image.delete", &output));
    }

    // Layer 8: Confirmation flow
    let params_json = serde_json::json!({
        "host": host,
        "image": image,
        "force": force,
    })
    .to_string();

    if let Some(response) = confirmation
        .check_and_maybe_require(
            "docker.image.delete",
            None,
            &format!(
                "About to DELETE image '{}' on Docker host '{}'. This is irreversible.",
                image, host
            ),
            &params_json,
        )
        .await?
    {
        audit
            .log("docker.image.delete", &host, "confirmation_required", Some(&image))
            .await
            .ok();
        return Ok(response);
    }

    image_delete_confirmed(manager, host, image, force, audit).await
}

pub async fn image_delete_confirmed(
    manager: Arc<ConnectionManager>,
    host: String,
    image: String,
    force: bool,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let result: Result<String> = async {
        let docker = manager.get_docker(&host)?;

        let results = docker
            .as_bollard()
            .remove_image(
                &image,
                Some(RemoveImageOptions {
                    force,
                    noprune: false,
                }),
                None,
            )
            .await
            .map_err(|error| anyhow!("Failed to delete image '{}': {}", image, error))?;

        let deleted_count = results
            .iter()
            .filter(|r| r.deleted.is_some())
            .count();
        let untagged_count = results
            .iter()
            .filter(|r| r.untagged.is_some())
            .count();

        Ok(format!(
            "Deleted image '{}' on Docker host '{}'. {} layers deleted, {} tags removed.",
            image, host, deleted_count, untagged_count
        ))
    }
    .await;

    match result {
        Ok(output) => {
            audit
                .log("docker.image.delete", &host, "success", Some(&image))
                .await
                .ok();
            Ok(wrap_output_envelope("docker.image.delete", &output))
        }
        Err(error) => {
            audit
                .log("docker.image.delete", &host, "error", Some(&error.to_string()))
                .await
                .ok();
            Err(error)
        }
    }
}
```

---

## Step 6: Implement `image_prune`

```rust
use bollard::image::PruneImagesOptions;

pub async fn image_prune(
    manager: Arc<ConnectionManager>,
    confirmation: Arc<ConfirmationManager>,
    host: Option<String>,
    all: Option<bool>,
    dry_run: Option<bool>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let host = host.unwrap_or_else(|| "local".to_string());
    let prune_all = all.unwrap_or(false);

    if dry_run.unwrap_or(false) {
        let scope = if prune_all {
            "ALL unused images (including tagged)"
        } else {
            "dangling (untagged) images only"
        };
        let output = format!(
            "DRY RUN: Would prune {} on Docker host '{}'.",
            scope, host
        );
        audit
            .log("docker.image.prune", &host, "dry_run", None)
            .await
            .ok();
        return Ok(wrap_output_envelope("docker.image.prune", &output));
    }

    // Layer 8: Confirmation flow
    let params_json = serde_json::json!({
        "host": host,
        "all": prune_all,
    })
    .to_string();

    let scope_desc = if prune_all { "all unused" } else { "dangling" };
    if let Some(response) = confirmation
        .check_and_maybe_require(
            "docker.image.prune",
            None,
            &format!(
                "About to PRUNE {} images on Docker host '{}'. This is irreversible.",
                scope_desc, host
            ),
            &params_json,
        )
        .await?
    {
        audit
            .log("docker.image.prune", &host, "confirmation_required", None)
            .await
            .ok();
        return Ok(response);
    }

    image_prune_confirmed(manager, host, prune_all, audit).await
}

pub async fn image_prune_confirmed(
    manager: Arc<ConnectionManager>,
    host: String,
    prune_all: bool,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let result: Result<String> = async {
        let docker = manager.get_docker(&host)?;

        let mut filters = HashMap::new();
        if !prune_all {
            filters.insert("dangling".to_string(), vec!["true".to_string()]);
        }

        let response = docker
            .as_bollard()
            .prune_images(Some(PruneImagesOptions { filters }))
            .await
            .map_err(|error| anyhow!("Failed to prune images: {}", error))?;

        let deleted_count = response
            .images_deleted
            .as_ref()
            .map(|images| images.len())
            .unwrap_or(0);
        let reclaimed = response.space_reclaimed;
        let reclaimed_mb = reclaimed as f64 / 1_000_000.0;

        Ok(format!(
            "Pruned {} images on Docker host '{}'. Reclaimed {:.1} MB.",
            deleted_count, host, reclaimed_mb
        ))
    }
    .await;

    match result {
        Ok(output) => {
            audit
                .log("docker.image.prune", &host, "success", None)
                .await
                .ok();
            Ok(wrap_output_envelope("docker.image.prune", &output))
        }
        Err(error) => {
            audit
                .log("docker.image.prune", &host, "error", Some(&error.to_string()))
                .await
                .ok();
            Err(error)
        }
    }
}
```

---

## Step 7: Register image tools in `mcp.rs`

### 7a. Add new args structs

```rust
#[derive(Debug, Deserialize, JsonSchema)]
struct DockerImageListArgs {
    /// Docker host name (defaults to "local")
    pub host: Option<String>,
    /// Include intermediate images
    pub all: Option<bool>,
    /// Filter images by reference (e.g. "nginx", "myregistry.com/app")
    pub name_filter: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct DockerImagePullArgs {
    /// Docker host name (defaults to "local")
    pub host: Option<String>,
    /// Image to pull (e.g. "nginx:latest", "postgres:16-alpine")
    pub image: String,
    /// Preview the operation without executing
    pub dry_run: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct DockerImageInspectArgs {
    /// Docker host name (defaults to "local")
    pub host: Option<String>,
    /// Image name or ID to inspect
    pub image: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct DockerImageDeleteArgs {
    /// Docker host name (defaults to "local")
    pub host: Option<String>,
    /// Image name or ID to delete
    pub image: String,
    /// Force removal even if image is in use by containers
    pub force: Option<bool>,
    /// Preview the operation without executing (recommended: use dry_run=true first)
    pub dry_run: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct DockerImagePruneArgs {
    /// Docker host name (defaults to "local")
    pub host: Option<String>,
    /// Prune ALL unused images (not just dangling/untagged ones)
    pub all: Option<bool>,
    /// Preview the operation without executing
    pub dry_run: Option<bool>,
}
```

### 7b. Add import

```rust
use crate::tools::docker_image;
```

### 7c. Update `ALL_TOOLS`

Add after the `docker.container.*` entries:
```rust
("docker.image.list",    "Docker"),
("docker.image.pull",    "Docker"),
("docker.image.inspect", "Docker"),
("docker.image.delete",  "Docker"),
("docker.image.prune",   "Docker"),
```

### 7d. Add tool handlers in `#[tool_router]` impl

```rust
#[tool(name = "docker.image.list", description = "List Docker images")]
async fn docker_image_list(
    &self,
    Parameters(args): Parameters<DockerImageListArgs>,
) -> Result<String, String> {
    self.ensure_tool_available("docker.image.list")?;
    docker_image::image_list(
        self.manager.clone(),
        args.host,
        args.all,
        args.name_filter,
        self.audit.clone(),
    )
    .await
    .map_err(|error| error.to_string())
}

#[tool(name = "docker.image.pull", description = "Pull a Docker image from a registry")]
async fn docker_image_pull(
    &self,
    Parameters(args): Parameters<DockerImagePullArgs>,
) -> Result<String, String> {
    self.ensure_tool_available("docker.image.pull")?;
    docker_image::image_pull(
        self.manager.clone(),
        args.host,
        args.image,
        args.dry_run,
        self.audit.clone(),
    )
    .await
    .map_err(|error| error.to_string())
}

#[tool(name = "docker.image.inspect", description = "Inspect a Docker image's metadata")]
async fn docker_image_inspect(
    &self,
    Parameters(args): Parameters<DockerImageInspectArgs>,
) -> Result<String, String> {
    self.ensure_tool_available("docker.image.inspect")?;
    docker_image::image_inspect(
        self.manager.clone(),
        args.host,
        args.image,
        self.audit.clone(),
    )
    .await
    .map_err(|error| error.to_string())
}

#[tool(
    name = "docker.image.delete",
    description = "Delete a Docker image. IMPORTANT: Use dry_run=true first to preview. \
                   This operation is irreversible and requires confirmation."
)]
async fn docker_image_delete(
    &self,
    Parameters(args): Parameters<DockerImageDeleteArgs>,
) -> Result<String, String> {
    self.ensure_tool_available("docker.image.delete")?;
    docker_image::image_delete(
        self.manager.clone(),
        self.confirmation.clone(),
        args.host,
        args.image,
        args.force,
        args.dry_run,
        self.audit.clone(),
    )
    .await
    .map_err(|error| error.to_string())
}

#[tool(
    name = "docker.image.prune",
    description = "Remove unused Docker images. By default removes only dangling (untagged) images. \
                   Set all=true to remove all unused images. Use dry_run=true first to preview. \
                   This operation is irreversible and requires confirmation."
)]
async fn docker_image_prune(
    &self,
    Parameters(args): Parameters<DockerImagePruneArgs>,
) -> Result<String, String> {
    self.ensure_tool_available("docker.image.prune")?;
    docker_image::image_prune(
        self.manager.clone(),
        self.confirmation.clone(),
        args.host,
        args.all,
        args.dry_run,
        self.audit.clone(),
    )
    .await
    .map_err(|error| error.to_string())
}
```

### 7e. Add confirm_operation arms for `docker.image.delete` and `docker.image.prune`

In the `confirm_operation` match block:

```rust
"docker.image.delete" => {
    let params: DockerImageDeleteArgs =
        serde_json::from_str(&original_params_json)
            .map_err(|error| error.to_string())?;
    self.audit
        .log(
            "docker.image.delete",
            params.host.as_deref().unwrap_or("local"),
            "confirmed_exec",
            Some(&params.image),
        )
        .await
        .ok();
    docker_image::image_delete_confirmed(
        self.manager.clone(),
        params.host.unwrap_or_else(|| "local".to_string()),
        params.image,
        params.force.unwrap_or(false),
        self.audit.clone(),
    )
    .await
    .map_err(|error| error.to_string())
}
"docker.image.prune" => {
    let params: DockerImagePruneArgs =
        serde_json::from_str(&original_params_json)
            .map_err(|error| error.to_string())?;
    self.audit
        .log(
            "docker.image.prune",
            params.host.as_deref().unwrap_or("local"),
            "confirmed_exec",
            None,
        )
        .await
        .ok();
    docker_image::image_prune_confirmed(
        self.manager.clone(),
        params.host.unwrap_or_else(|| "local".to_string()),
        params.all.unwrap_or(false),
        self.audit.clone(),
    )
    .await
    .map_err(|error| error.to_string())
}
```

---

## Step 8: Update `tools/mod.rs`

```rust
/// Docker container tools
pub mod docker;
/// Docker image tools
pub mod docker_image;
/// SSH tools
pub mod ssh;
```

---

## Step 9: Update `example.config.toml`

Add image-related confirmation and rate limits:

```toml
[confirm]
"docker.container.delete" = "always"
"docker.image.delete" = "always"
"docker.image.prune" = "always"

[rate_limits.limits]
"docker.container.*" = { per_minute = 5 }
"docker.image.*" = { per_minute = 5 }
"docker.image.delete" = { per_minute = 1 }
"docker.image.prune" = { per_minute = 1 }
"ssh.exec" = { per_minute = 10 }
```

---

## Step 10: Verification

```bash
cargo check
cargo test --lib
cargo build --release
```

After M7, the tool count is **16 tools**: 7 Docker container + 5 Docker image + 3 SSH + confirm_operation.

---

## M7 Deliverables

- [x] `docker.image.list` — list images with filtering
- [x] `docker.image.pull` — pull image with dry_run
- [x] `docker.image.inspect` — image metadata with env redaction
- [x] `docker.image.delete` — with confirmation, dry_run, force
- [x] `docker.image.prune` — with confirmation, dry_run, all/dangling toggle
- [x] MCP tool registrations (16 tools total)
- [x] confirm_operation wired for image.delete and image.prune
- [x] Config updated with rate limits and confirmation rules

---
---

# Milestone 8: Per-User Rate Limiting (1-2 days)

**Goal:** Extend the rate limiter to track usage per-caller rather than globally, enabling fair usage across multiple users sharing the same Spacebot agent.

**Design doc backing:** None — this is a novel feature not mentioned in any design document.

**Context:** The MCP server is spawned 1:1 per Spacebot agent. Multiple users can interact with the same agent through different channels (Telegram DMs, Discord threads). All tool calls arrive through the same MCP connection, indistinguishable by default.

**Files modified:**
| File | Change |
|------|--------|
| `src/rate_limit.rs` | Add `PerUserRateLimiter`, caller-aware window tracking |
| `src/config.rs` | Add `rate_limit_mode` config field |
| `src/mcp.rs` | Thread caller context through to rate limiter |
| `example.config.toml` | Add `mode` config example |

---

## Step 1: Extend config with rate limit mode

In `src/config.rs`, update `RateLimitConfig`:

```rust
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RateLimitConfig {
    /// Rate limiting mode: "global" (default) or "per_caller"
    #[serde(default)]
    pub mode: RateLimitMode,
    #[serde(default)]
    pub limits: HashMap<String, RateLimit>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RateLimitMode {
    #[default]
    Global,
    PerCaller,
}
```

**Config example:**
```toml
[rate_limits]
mode = "per_caller"  # or "global" (default)

[rate_limits.limits]
"docker.container.*" = { per_minute = 5 }
```

---

## Step 2: Refactor `RateLimiter` to support per-caller tracking

The key insight: In `per_caller` mode, the rate limit window key becomes `"{caller_id}:{rate_key}"` instead of just `"{rate_key}"`. In `global` mode, the existing behavior is preserved (no caller prefix).

In `src/rate_limit.rs`:

```rust
use crate::config::{RateLimit, RateLimitMode};

pub struct RateLimiter {
    windows: Arc<DashMap<String, Vec<Instant>>>,
    exact_limits: Arc<DashMap<String, u32>>,
    wildcard_limits: Arc<Vec<(String, u32)>>,
    mode: RateLimitMode,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self {
            windows: Arc::new(DashMap::new()),
            exact_limits: Arc::new(DashMap::new()),
            wildcard_limits: Arc::new(Vec::new()),
            mode: RateLimitMode::Global,
        }
    }

    pub fn from_config(limits: &HashMap<String, RateLimit>, mode: RateLimitMode) -> Self {
        let exact = DashMap::new();
        let mut wildcards = Vec::new();

        for (pattern, entry) in limits {
            if pattern.contains('*') {
                wildcards.push((pattern.trim_end_matches('*').to_string(), entry.per_minute));
            } else {
                exact.insert(pattern.clone(), entry.per_minute);
            }
        }

        wildcards.sort_by(|a, b| b.0.len().cmp(&a.0.len()));

        Self {
            windows: Arc::new(DashMap::new()),
            exact_limits: Arc::new(exact),
            wildcard_limits: Arc::new(wildcards),
            mode,
        }
    }

    /// Check rate limit for a tool call. In `per_caller` mode, the caller_id
    /// is used to scope the rate limit window. In `global` mode, caller_id is ignored.
    pub fn check(&self, tool_name: &str, caller_id: Option<&str>) -> Result<()> {
        let (rate_key, limit) = match self.resolve_limit(tool_name) {
            Some(limit) => limit,
            None => return Ok(()),
        };

        // In per_caller mode, prefix the window key with caller_id
        let window_key = match (&self.mode, caller_id) {
            (RateLimitMode::PerCaller, Some(id)) if !id.is_empty() => {
                format!("{}:{}", id, rate_key)
            }
            _ => rate_key.clone(),
        };

        let now = Instant::now();
        let window_start = now - Duration::from_secs(60);
        let mut entry = self.windows.entry(window_key).or_default();

        entry.retain(|instant| *instant > window_start);

        if entry.len() >= limit as usize {
            let retry_after = entry
                .first()
                .map(|oldest| 60u64.saturating_sub(oldest.elapsed().as_secs()))
                .unwrap_or(60);

            return Err(anyhow!(
                "Rate limit exceeded for {}. Limit: {}/min. Retry after {}s.",
                tool_name,
                limit,
                retry_after
            ));
        }

        entry.push(now);
        Ok(())
    }

    // resolve_limit remains unchanged
}
```

**Important:** The `check` signature changes from `check(&self, tool_name: &str)` to `check(&self, tool_name: &str, caller_id: Option<&str>)`. All existing call sites need updating.

---

## Step 3: Extract caller identity from MCP context

The MCP protocol's `initialize` method includes client info (name, version). We can use this as a basic caller identifier. For Spacebot, the client name would be consistent but the worker ID might differ.

A more practical approach: accept an optional `_caller` field convention in tool arguments. But this is invasive (changes every tool schema) and fragile (LLM can omit it).

**Recommended approach:** Use a convention where Spacebot includes a caller ID in the MCP server's environment or startup args. Since each MCP server process serves one agent:

1. If Spacebot sends a per-user identifier as an env var or arg, use it
2. Otherwise, all calls share the same `"default"` caller ID (equivalent to global mode)

For now, the implementation supports caller-aware rate limiting at the infrastructure level. The actual caller_id extraction is a TODO that depends on MCP protocol evolution or Spacebot adding caller context.

In `src/mcp.rs`, update `ensure_tool_available`:

```rust
fn ensure_tool_available(&self, tool_name: &str) -> Result<(), String> {
    if !self.config.tools.is_enabled(tool_name) {
        return Err(format!("Tool '{}' is disabled by configuration.", tool_name));
    }

    // TODO: Extract caller_id from MCP request context when available.
    // For now, all callers share the same identity (global behavior).
    let caller_id: Option<&str> = None;

    self.rate_limiter
        .check(tool_name, caller_id)
        .map_err(|error| error.to_string())
}
```

And update the `confirm_operation` handler's rate limit check:

```rust
self.rate_limiter
    .check("confirm_operation", None)
    .map_err(|error| error.to_string())?;
```

---

## Step 4: Update `from_config` call site

In `HomelabMcpServer::new` in `mcp.rs`:

```rust
let rate_limiter = Arc::new(RateLimiter::from_config(
    &config.rate_limits.limits,
    config.rate_limits.mode.clone(),
));
```

---

## Step 5: Update existing tests and add per-caller tests

Update existing tests in `rate_limit.rs` to pass `None` as caller_id:

```rust
#[test]
fn test_exact_limit() {
    let limiter = RateLimiter::new();
    limiter.exact_limits.insert("test.tool".to_string(), 3);

    assert!(limiter.check("test.tool", None).is_ok());
    assert!(limiter.check("test.tool", None).is_ok());
    assert!(limiter.check("test.tool", None).is_ok());
    assert!(limiter.check("test.tool", None).is_err());
}
```

Add new per-caller tests:

```rust
#[test]
fn test_per_caller_independent_windows() {
    let limiter = RateLimiter {
        windows: Arc::new(DashMap::new()),
        exact_limits: Arc::new({
            let map = DashMap::new();
            map.insert("test.tool".to_string(), 2);
            map
        }),
        wildcard_limits: Arc::new(Vec::new()),
        mode: RateLimitMode::PerCaller,
    };

    // User A uses 2 calls — hits limit
    assert!(limiter.check("test.tool", Some("user_a")).is_ok());
    assert!(limiter.check("test.tool", Some("user_a")).is_ok());
    assert!(limiter.check("test.tool", Some("user_a")).is_err());

    // User B still has their own quota
    assert!(limiter.check("test.tool", Some("user_b")).is_ok());
    assert!(limiter.check("test.tool", Some("user_b")).is_ok());
    assert!(limiter.check("test.tool", Some("user_b")).is_err());
}

#[test]
fn test_per_caller_none_falls_back_to_global() {
    let limiter = RateLimiter {
        windows: Arc::new(DashMap::new()),
        exact_limits: Arc::new({
            let map = DashMap::new();
            map.insert("test.tool".to_string(), 2);
            map
        }),
        wildcard_limits: Arc::new(Vec::new()),
        mode: RateLimitMode::PerCaller,
    };

    // Calls without caller_id share a single window
    assert!(limiter.check("test.tool", None).is_ok());
    assert!(limiter.check("test.tool", None).is_ok());
    assert!(limiter.check("test.tool", None).is_err());
}

#[test]
fn test_global_mode_ignores_caller_id() {
    let limiter = RateLimiter {
        windows: Arc::new(DashMap::new()),
        exact_limits: Arc::new({
            let map = DashMap::new();
            map.insert("test.tool".to_string(), 2);
            map
        }),
        wildcard_limits: Arc::new(Vec::new()),
        mode: RateLimitMode::Global,
    };

    // In global mode, different caller_ids still share one window
    assert!(limiter.check("test.tool", Some("user_a")).is_ok());
    assert!(limiter.check("test.tool", Some("user_b")).is_ok());
    assert!(limiter.check("test.tool", Some("user_c")).is_err());
}
```

---

## Step 6: Update `example.config.toml`

```toml
[rate_limits]
# Mode: "global" (default) — all callers share one rate limit window
#        "per_caller" — each caller gets their own rate limit window
# mode = "per_caller"

[rate_limits.limits]
"docker.container.*" = { per_minute = 5 }
"docker.image.*" = { per_minute = 5 }
"ssh.exec" = { per_minute = 10 }
```

---

## Step 7: Verification

```bash
cargo test --lib rate_limit
# All tests pass, including new per-caller tests
cargo check
cargo build --release
```

---

## M8 Deliverables

- [x] `RateLimitMode` enum: `Global` (default) | `PerCaller`
- [x] `RateLimiter::check` accepts optional `caller_id`
- [x] Per-caller windows: `"{caller_id}:{rate_key}"` scoping
- [x] Global mode backward compatible (no behavior change)
- [x] Config: `[rate_limits] mode = "per_caller"` opt-in
- [x] 3 new unit tests for per-caller behavior
- [x] Existing tests updated for new `check` signature
- [x] TODO placeholder for MCP-level caller extraction

---
---

# Milestone 9: Metrics and Observability (2-3 days)

**Goal:** Add Prometheus-compatible metrics for tool usage, connection health, and pool statistics, exposed via an optional HTTP endpoint.

**Design doc backing:** None — this is a novel feature not mentioned in any design document. The existing observability is limited to audit logging and tracing.

**Files modified/created:**
| File | Change |
|------|--------|
| `Cargo.toml` | Add `prometheus` and `hyper`/`axum` dependencies |
| `src/metrics.rs` | **New file** — metrics definitions and HTTP server |
| `src/config.rs` | Add `[metrics]` config section |
| `src/mcp.rs` | Instrument tool handlers with metrics |
| `src/connection.rs` | Add pool gauge updates |
| `src/main.rs` | Start metrics server if configured |

---

## Step 1: Add dependencies to `Cargo.toml`

```toml
# Metrics
prometheus = { version = "0.13", default-features = false }

# HTTP server for metrics endpoint (lightweight)
axum = { version = "0.7", default-features = false, features = ["http1", "tokio"], optional = true }

[features]
default = ["notifications", "metrics"]
notifications = ["dep:notify-rust"]
metrics = ["dep:axum"]
```

**Why `axum`:** The MCP server communicates over stdio and has no HTTP server. Metrics need an HTTP endpoint for Prometheus to scrape. `axum` is a lightweight choice that's already in the tokio ecosystem. Making it optional behind a feature flag keeps the binary small when metrics aren't needed.

---

## Step 2: Add metrics config to `config.rs`

```rust
#[derive(Debug, Clone, Default, Deserialize)]
pub struct MetricsConfig {
    /// Enable Prometheus metrics endpoint
    #[serde(default)]
    pub enabled: bool,
    /// Listen address for the metrics HTTP server (default: "127.0.0.1:9090")
    #[serde(default = "default_metrics_listen")]
    pub listen: String,
}

fn default_metrics_listen() -> String {
    "127.0.0.1:9090".to_string()
}
```

Add to the `Config` struct:
```rust
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    // ... existing fields ...
    #[serde(default)]
    pub metrics: MetricsConfig,
}
```

---

## Step 3: Create `src/metrics.rs`

```rust
use prometheus::{
    Encoder, HistogramOpts, HistogramVec, IntCounterVec, IntGaugeVec, Opts, Registry, TextEncoder,
};
use std::sync::Arc;

/// Central metrics registry for the MCP server.
#[derive(Clone)]
pub struct Metrics {
    pub registry: Registry,

    /// Total tool invocations (labels: tool, status)
    pub tool_calls_total: IntCounterVec,

    /// Tool call duration in seconds (labels: tool)
    pub tool_duration_seconds: HistogramVec,

    /// SSH pool active sessions (labels: host)
    pub ssh_pool_active: IntGaugeVec,

    /// SSH pool idle sessions (labels: host)
    pub ssh_pool_idle: IntGaugeVec,

    /// SSH pool total sessions (labels: host)
    pub ssh_pool_total: IntGaugeVec,

    /// Docker connection health (labels: host; 1=connected, 0=disconnected)
    pub docker_health: IntGaugeVec,

    /// SSH connection health (labels: host; 1=connected, 0=disconnected)
    pub ssh_health: IntGaugeVec,

    /// Confirmation tokens issued
    pub confirmation_tokens_issued: IntCounterVec,

    /// Confirmation tokens confirmed/expired/rejected
    pub confirmation_tokens_resolved: IntCounterVec,
}

impl Metrics {
    pub fn new() -> Self {
        let registry = Registry::new_custom(Some("homelab".to_string()), None)
            .expect("metrics registry");

        let tool_calls_total = IntCounterVec::new(
            Opts::new("tool_calls_total", "Total MCP tool invocations"),
            &["tool", "status"],
        )
        .expect("tool_calls_total metric");

        let tool_duration_seconds = HistogramVec::new(
            HistogramOpts::new("tool_duration_seconds", "Tool call duration in seconds")
                .buckets(vec![0.01, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0]),
            &["tool"],
        )
        .expect("tool_duration_seconds metric");

        let ssh_pool_active = IntGaugeVec::new(
            Opts::new("ssh_pool_active_sessions", "Active (checked out) SSH sessions"),
            &["host"],
        )
        .expect("ssh_pool_active metric");

        let ssh_pool_idle = IntGaugeVec::new(
            Opts::new("ssh_pool_idle_sessions", "Idle (available) SSH sessions"),
            &["host"],
        )
        .expect("ssh_pool_idle metric");

        let ssh_pool_total = IntGaugeVec::new(
            Opts::new("ssh_pool_total_sessions", "Total SSH sessions (active + idle)"),
            &["host"],
        )
        .expect("ssh_pool_total metric");

        let docker_health = IntGaugeVec::new(
            Opts::new("docker_connection_healthy", "Docker connection health (1=up, 0=down)"),
            &["host"],
        )
        .expect("docker_health metric");

        let ssh_health = IntGaugeVec::new(
            Opts::new("ssh_connection_healthy", "SSH connection health (1=up, 0=down)"),
            &["host"],
        )
        .expect("ssh_health metric");

        let confirmation_tokens_issued = IntCounterVec::new(
            Opts::new("confirmation_tokens_issued_total", "Confirmation tokens issued"),
            &["tool"],
        )
        .expect("confirmation_tokens_issued metric");

        let confirmation_tokens_resolved = IntCounterVec::new(
            Opts::new(
                "confirmation_tokens_resolved_total",
                "Confirmation tokens resolved",
            ),
            &["outcome"], // "confirmed", "expired", "rejected"
        )
        .expect("confirmation_tokens_resolved metric");

        // Register all metrics
        for collector in [
            Box::new(tool_calls_total.clone()) as Box<dyn prometheus::core::Collector>,
            Box::new(tool_duration_seconds.clone()),
            Box::new(ssh_pool_active.clone()),
            Box::new(ssh_pool_idle.clone()),
            Box::new(ssh_pool_total.clone()),
            Box::new(docker_health.clone()),
            Box::new(ssh_health.clone()),
            Box::new(confirmation_tokens_issued.clone()),
            Box::new(confirmation_tokens_resolved.clone()),
        ] {
            registry.register(collector).expect("register metric");
        }

        Self {
            registry,
            tool_calls_total,
            tool_duration_seconds,
            ssh_pool_active,
            ssh_pool_idle,
            ssh_pool_total,
            docker_health,
            ssh_health,
            confirmation_tokens_issued,
            confirmation_tokens_resolved,
        }
    }

    /// Encode all metrics in Prometheus text format.
    pub fn encode(&self) -> String {
        let encoder = TextEncoder::new();
        let metric_families = self.registry.gather();
        let mut buffer = Vec::new();
        encoder.encode(&metric_families, &mut buffer).ok();
        String::from_utf8(buffer).unwrap_or_default()
    }
}

/// Start the optional metrics HTTP server.
/// Returns a JoinHandle that can be aborted on shutdown.
#[cfg(feature = "metrics")]
pub fn spawn_metrics_server(
    listen: &str,
    metrics: Arc<Metrics>,
) -> tokio::task::JoinHandle<()> {
    use axum::{Router, routing::get, extract::State};

    let app = Router::new()
        .route("/metrics", get(|State(metrics): State<Arc<Metrics>>| async move {
            metrics.encode()
        }))
        .with_state(metrics);

    let listener_addr: std::net::SocketAddr = listen
        .parse()
        .expect("invalid metrics listen address");

    tokio::spawn(async move {
        let listener = tokio::net::TcpListener::bind(listener_addr)
            .await
            .expect("bind metrics server");
        tracing::info!("Metrics server listening on http://{}/metrics", listener_addr);
        axum::serve(listener, app).await.ok();
    })
}
```

---

## Step 4: Instrument tool handlers in `mcp.rs`

Add `metrics: Arc<Metrics>` field to `HomelabMcpServer`. Create a helper method for recording tool metrics:

```rust
use crate::metrics::Metrics;
use std::time::Instant;

#[derive(Clone)]
pub struct HomelabMcpServer {
    // ... existing fields ...
    metrics: Option<Arc<Metrics>>,
}

impl HomelabMcpServer {
    /// Record tool call metrics (duration + outcome).
    fn record_tool_call(&self, tool_name: &str, start: Instant, is_error: bool) {
        if let Some(ref metrics) = self.metrics {
            let status = if is_error { "error" } else { "success" };
            metrics
                .tool_calls_total
                .with_label_values(&[tool_name, status])
                .inc();
            metrics
                .tool_duration_seconds
                .with_label_values(&[tool_name])
                .observe(start.elapsed().as_secs_f64());
        }
    }
}
```

Then wrap each tool handler. Example for `docker_container_list`:

```rust
#[tool(name = "docker.container.list", description = "List Docker containers")]
async fn docker_container_list(
    &self,
    Parameters(args): Parameters<DockerContainerListArgs>,
) -> Result<String, String> {
    self.ensure_tool_available("docker.container.list")?;
    let start = Instant::now();
    let result = docker::container_list(
        self.manager.clone(),
        args.host,
        args.all,
        args.name_filter,
        self.audit.clone(),
    )
    .await
    .map_err(|error| error.to_string());
    self.record_tool_call("docker.container.list", start, result.is_err());
    result
}
```

**Apply this pattern to all tool handlers.** Each handler wraps the call with `let start = Instant::now()` and `self.record_tool_call(...)` after the result.

---

## Step 5: Instrument connection health in `connection.rs`

In the health monitor loop (`spawn_health_monitor`), after `mark_healthy` / `mark_unhealthy` calls, update the health gauges:

```rust
// After Docker health check:
if let Some(ref metrics) = self.metrics {
    let value = if is_healthy { 1 } else { 0 };
    metrics.docker_health.with_label_values(&[&name]).set(value);
}

// After SSH health check + cleanup:
if let Some(ref metrics) = self.metrics {
    let value = if is_healthy { 1 } else { 0 };
    metrics.ssh_health.with_label_values(&[&name]).set(value);

    // Update pool gauges
    let active = pool.active_count.load(Ordering::Relaxed) as i64;
    let total = pool.total_count.load(Ordering::Relaxed) as i64;
    let idle = total - active;
    metrics.ssh_pool_active.with_label_values(&[&name]).set(active);
    metrics.ssh_pool_idle.with_label_values(&[&name]).set(idle);
    metrics.ssh_pool_total.with_label_values(&[&name]).set(total);
}
```

**Note:** `ConnectionManager` needs an `Option<Arc<Metrics>>` field. Add it to `new()` and pass it from `main.rs`.

---

## Step 6: Start metrics server in `main.rs`

In `run_server`, after creating the `HomelabMcpServer`:

```rust
// Start metrics server if configured
#[cfg(feature = "metrics")]
let metrics_handle = if config.metrics.enabled {
    let metrics = Arc::new(crate::metrics::Metrics::new());
    // Pass metrics to server and connection manager
    Some(crate::metrics::spawn_metrics_server(&config.metrics.listen, metrics.clone()))
} else {
    None
};
```

And on shutdown, abort the metrics server:

```rust
#[cfg(feature = "metrics")]
if let Some(handle) = metrics_handle {
    handle.abort();
}
```

---

## Step 7: Update `example.config.toml`

```toml
# Metrics (optional, requires "metrics" feature)
# [metrics]
# enabled = true
# listen = "127.0.0.1:9090"  # Prometheus scrape endpoint
```

---

## Step 8: Verification

```bash
cargo check
cargo test --lib
cargo build --release

# If metrics enabled:
curl http://127.0.0.1:9090/metrics
# Should return Prometheus text format with homelab_* metrics
```

---

## M9 Deliverables

- [x] `Metrics` struct with Prometheus counters, histograms, gauges
- [x] Optional HTTP metrics endpoint via `axum`
- [x] Tool call instrumentation (count + duration)
- [x] SSH pool gauge updates (active/idle/total per host)
- [x] Docker/SSH connection health gauges
- [x] Confirmation token counters
- [x] Feature-gated behind `metrics` feature flag
- [x] Config: `[metrics] enabled = true, listen = "127.0.0.1:9090"`

---
---

# Milestone 10: SSH Channel Multiplexing (2-3 days)

**Goal:** Change the SSH pool from exclusive session checkout to shared channel multiplexing, allowing multiple concurrent commands on a single SSH session.

**Design doc backing:**
- `connection-manager.md` lines 185-189: "For V1, each checkout gets exclusive use of a session. Channel multiplexing within a session is a V2 optimization."

**Why this matters:** Currently, if `max_sessions_per_host = 3` and 3 concurrent tool calls arrive for the same host, the 4th blocks until one completes. With channel multiplexing, a single session can serve many concurrent exec channels, making the pool more efficient. The session limit becomes about transport capacity, not concurrency.

**Files modified:**
| File | Change |
|------|--------|
| `src/connection.rs` | Refactor `SshPool` from session-exclusive to channel-shared |
| `src/tools/ssh.rs` | Update exec/upload/download to use new channel API |
| `src/config.rs` | Add `max_channels_per_session` config |

---

## Step 1: Add config for channel multiplexing

In `src/config.rs`, add to `SshPoolConfig`:

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct SshPoolConfig {
    // ... existing fields ...

    /// Maximum concurrent channels per SSH session.
    /// Default: 10. Set to 1 to disable multiplexing (V1 behavior).
    #[serde(default = "default_max_channels")]
    pub max_channels_per_session: usize,
}

fn default_max_channels() -> usize {
    10
}
```

Update the `Default` impl:
```rust
impl Default for SshPoolConfig {
    fn default() -> Self {
        Self {
            // ... existing fields ...
            max_channels_per_session: default_max_channels(),
        }
    }
}
```

---

## Step 2: Refactor `SshPool` in `connection.rs`

### 2a. Replace `PooledSession` with `SharedSession`

The core change: sessions are no longer exclusively checked out. Instead, they track active channel count and are shared across concurrent callers.

```rust
/// A shared SSH session that can serve multiple concurrent channels.
struct SharedSession {
    handle: russh::client::Handle<SshClientHandler>,
    created_at: Instant,
    last_used: Instant,
    /// Number of channels currently open on this session.
    active_channels: usize,
}
```

### 2b. Replace `VecDeque<PooledSession>` with `Vec<SharedSession>`

```rust
pub struct SshPool {
    sessions: Arc<Mutex<Vec<SharedSession>>>,
    max_sessions: usize,
    max_channels_per_session: usize,
    session_available: Arc<Notify>,
    host_config: Arc<SshHost>,
    pool_config: SshPoolConfig,
}
```

**Removed fields:** `active_count` and `total_count` atomics are no longer needed — all state is tracked inside the `Vec<SharedSession>` under the mutex.

### 2c. New public API: `acquire_channel` / `release_channel`

Replace the old `checkout()` / `return_session()` API:

```rust
/// An acquired channel from the pool. Must be released via `release_channel`.
pub struct AcquiredChannel {
    pub handle: russh::client::Handle<SshClientHandler>,
    session_index: usize,
    host_name: String,
}

impl SshPool {
    /// Acquire a channel from the pool. If an existing session has capacity,
    /// opens a new channel on it. Otherwise creates a new session (if under limit)
    /// or waits for capacity.
    pub async fn acquire_channel(&self) -> Result<AcquiredChannel> {
        loop {
            {
                let mut sessions = self.sessions.lock().await;

                // First pass: find an existing session with channel capacity
                // Prefer the session with the fewest active channels (load balancing)
                let best_index = sessions
                    .iter()
                    .enumerate()
                    .filter(|(_, session)| {
                        session.active_channels < self.max_channels_per_session
                            && self.validate_session_age(session)
                    })
                    .min_by_key(|(_, session)| session.active_channels)
                    .map(|(index, _)| index);

                if let Some(index) = best_index {
                    sessions[index].active_channels += 1;
                    sessions[index].last_used = Instant::now();

                    return Ok(AcquiredChannel {
                        handle: sessions[index].handle.clone(),
                        session_index: index,
                        host_name: self.host_config.host.clone(),
                    });
                }

                // Second pass: can we create a new session?
                if sessions.len() < self.max_sessions {
                    drop(sessions); // Release lock during connection
                    let session = match self.create_shared_session().await {
                        Ok(session) => session,
                        Err(first_error) => {
                            warn!(
                                "SSH connection to {} failed, retrying once: {}",
                                self.host_config.host, first_error
                            );
                            tokio::time::sleep(Duration::from_secs(1)).await;
                            self.create_shared_session().await.map_err(|retry_error| {
                                anyhow!(
                                    "SSH connection failed after retry. First: {}. Retry: {}",
                                    first_error,
                                    retry_error
                                )
                            })?
                        }
                    };

                    let mut sessions = self.sessions.lock().await;
                    let index = sessions.len();
                    sessions.push(session);
                    sessions[index].active_channels += 1;

                    return Ok(AcquiredChannel {
                        handle: sessions[index].handle.clone(),
                        session_index: index,
                        host_name: self.host_config.host.clone(),
                    });
                }

                // All sessions at capacity and at session limit — need to wait
            }

            let wait_duration = Duration::from_secs(self.pool_config.checkout_timeout_secs);
            tokio::time::timeout(wait_duration, self.session_available.notified())
                .await
                .map_err(|_| {
                    anyhow!(
                        "All SSH sessions to '{}' are at channel capacity ({} sessions x {} channels). Try again shortly.",
                        self.host_config.host,
                        self.max_sessions,
                        self.max_channels_per_session
                    )
                })?;
        }
    }

    /// Release a channel back to the pool. If the channel errored (broken=true),
    /// the session's active channel count is decremented but the session may be
    /// marked for cleanup.
    pub async fn release_channel(&self, channel: AcquiredChannel, broken: bool) {
        let mut sessions = self.sessions.lock().await;

        if let Some(session) = sessions.get_mut(channel.session_index) {
            session.active_channels = session.active_channels.saturating_sub(1);
            session.last_used = Instant::now();

            if broken && session.active_channels == 0 {
                // Session is broken and has no other users — remove it
                sessions.remove(channel.session_index);
                info!(
                    "Removed broken SSH session for {} (index {})",
                    channel.host_name, channel.session_index
                );
            }
        }

        self.session_available.notify_one();
    }

    fn validate_session_age(&self, session: &SharedSession) -> bool {
        let max_lifetime = Duration::from_secs(self.pool_config.max_lifetime_secs);
        session.created_at.elapsed() <= max_lifetime
    }

    async fn create_shared_session(&self) -> Result<SharedSession> {
        // Same logic as current create_session(), but returns SharedSession
        let config = russh::client::Config {
            inactivity_timeout: Some(Duration::from_secs(
                self.pool_config.connect_timeout_secs,
            )),
            keepalive_interval: Some(Duration::from_secs(
                self.pool_config.keepalive_interval_secs,
            )),
            keepalive_max: 3,
            ..Default::default()
        };

        let port = self.host_config.port.unwrap_or(22);
        let connect_duration = Duration::from_secs(self.pool_config.connect_timeout_secs);

        let handler = SshClientHandler::new(self.host_config.host.clone(), port);
        let mut handle = tokio::time::timeout(connect_duration, async {
            russh::client::connect(
                Arc::new(config),
                (self.host_config.host.as_str(), port),
                handler,
            )
            .await
        })
        .await
        .map_err(|_| {
            anyhow!(
                "SSH connection timed out after {}s to {}:{}",
                self.pool_config.connect_timeout_secs,
                self.host_config.host,
                port
            )
        })?
        .map_err(|error| {
            anyhow!(
                "SSH connection failed to {}:{}: {}",
                self.host_config.host,
                port,
                error
            )
        })?;

        let key = russh::keys::load_secret_key(
            &self.host_config.private_key_path,
            self.host_config.private_key_passphrase.as_deref(),
        )
        .map_err(|error| {
            anyhow!(
                "Failed to load SSH key {:?}: {:?}",
                self.host_config.private_key_path,
                error
            )
        })?;

        let authenticated = handle
            .authenticate_publickey(&self.host_config.user, Arc::new(key))
            .await
            .map_err(|error| {
                anyhow!(
                    "SSH authentication failed for {}@{}:{}: {}",
                    self.host_config.user,
                    self.host_config.host,
                    port,
                    error
                )
            })?;

        if !authenticated {
            return Err(anyhow!(
                "SSH authentication rejected for {}@{}:{}.",
                self.host_config.user,
                self.host_config.host,
                port
            ));
        }

        Ok(SharedSession {
            handle,
            created_at: Instant::now(),
            last_used: Instant::now(),
            active_channels: 0,
        })
    }

    /// Clean up stale sessions. Only removes sessions with zero active channels
    /// that have exceeded their lifetime or idle time.
    pub async fn cleanup_stale_sessions(&self) {
        let max_lifetime = Duration::from_secs(self.pool_config.max_lifetime_secs);
        let max_idle = Duration::from_secs(self.pool_config.max_idle_time_secs);

        let mut sessions = self.sessions.lock().await;
        let before = sessions.len();

        sessions.retain(|session| {
            // Never remove sessions with active channels
            if session.active_channels > 0 {
                return true;
            }
            // Remove if expired
            session.created_at.elapsed() <= max_lifetime
                && session.last_used.elapsed() <= max_idle
        });

        let removed = before.saturating_sub(sessions.len());
        if removed > 0 {
            info!(
                "Cleaned up {} stale SSH sessions for {}",
                removed, self.host_config.host
            );
        }
    }

    pub async fn check_connectivity(&self) -> Result<()> {
        {
            let sessions = self.sessions.lock().await;
            for session in sessions.iter() {
                if self.validate_session_age(session) {
                    return Ok(());
                }
            }
        }

        let session = self.create_shared_session().await?;
        session
            .handle
            .disconnect(
                russh::Disconnect::ByApplication,
                "health check complete",
                "en",
            )
            .await
            .ok();
        Ok(())
    }

    pub async fn close_all(&self) {
        let mut sessions = self.sessions.lock().await;
        sessions.clear();
    }
}
```

---

## Step 3: Update `ConnectionManager` convenience methods

```rust
impl ConnectionManager {
    pub async fn ssh_acquire_channel(&self, host: &str) -> Result<AcquiredChannel> {
        let pool = self
            .ssh_pools
            .get(host)
            .map(|entry| entry.clone())
            .ok_or_else(|| {
                anyhow!(self.disconnected_error_message(&format!("ssh:{}", host), host))
            })?;

        pool.acquire_channel().await
    }

    pub async fn ssh_release_channel(
        &self,
        host: &str,
        channel: AcquiredChannel,
        broken: bool,
    ) {
        if let Some(pool) = self.ssh_pools.get(host) {
            pool.release_channel(channel, broken).await;
        }
    }
}
```

**Remove** the old `ssh_checkout` and `ssh_return` methods. Or keep them as deprecated wrappers if needed for backward compatibility during migration.

---

## Step 4: Update `tools/ssh.rs` to use channel API

### 4a. Update `exec_confirmed`

Replace:
```rust
let session = manager.ssh_checkout(&host).await?;
```

With:
```rust
let acquired = manager.ssh_acquire_channel(&host).await?;
```

And replace the channel opening:
```rust
// OLD:
let mut channel = session.handle.channel_open_session().await...
```
With:
```rust
// NEW: Open a channel on the shared session handle
let mut channel = acquired.handle.channel_open_session().await
    .map_err(|error| anyhow!("Failed to open SSH channel: {}", error))?;
```

And replace the return:
```rust
// OLD:
manager.ssh_return(&host, session, broken).await;
```
With:
```rust
// NEW:
manager.ssh_release_channel(&host, acquired, broken).await;
```

### 4b. Update `upload` and `download`

Same pattern — replace `ssh_checkout/ssh_return` with `ssh_acquire_channel/ssh_release_channel`:

```rust
// upload
let acquired = manager.ssh_acquire_channel(&host).await?;
let channel = acquired.handle.channel_open_session().await...
// ... sftp operations ...
manager.ssh_release_channel(&host, acquired, broken).await;
```

```rust
// download
let acquired = manager.ssh_acquire_channel(&host).await?;
let channel = acquired.handle.channel_open_session().await...
// ... sftp operations ...
manager.ssh_release_channel(&host, acquired, broken).await;
```

---

## Step 5: Update `example.config.toml`

```toml
[ssh.pool]
max_sessions_per_host = 3
max_channels_per_session = 10   # NEW: max concurrent channels per session
max_lifetime_secs = 1800
max_idle_time_secs = 300
connect_timeout_secs = 10
checkout_timeout_secs = 5
keepalive_interval_secs = 60
```

---

## Step 6: Update unit tests in `connection.rs`

The `test_disconnected_error_mentions_last_success` test constructs a `ConnectionManager` directly. It doesn't use `ssh_checkout`/`ssh_return`, so it should still work. But update any tests that reference the old API.

Add a new unit test for channel multiplexing:

```rust
#[test]
fn test_shared_session_channel_accounting() {
    // Verify the channel tracking math
    let session = SharedSession {
        handle: unimplemented!(), // Can't construct in unit test
        created_at: Instant::now(),
        last_used: Instant::now(),
        active_channels: 0,
    };
    assert_eq!(session.active_channels, 0);

    // This is primarily tested via integration tests since SharedSession
    // requires a real russh handle. The logic is straightforward:
    // - acquire_channel increments active_channels
    // - release_channel decrements active_channels
    // - cleanup_stale_sessions skips sessions with active_channels > 0
}
```

**Note:** Full multiplexing tests require an SSH server and are integration tests (run with `--ignored`).

---

## Step 7: Backward compatibility

Setting `max_channels_per_session = 1` restores V1 behavior (exclusive session checkout). This is the safe fallback if multiplexing causes issues.

---

## Step 8: Verification

```bash
cargo check
cargo test --lib
cargo build --release
```

Integration test: Run multiple concurrent `ssh.exec` calls to the same host and verify they complete without waiting for each other (if channels > 1).

---

## M10 Deliverables

- [x] `SharedSession` replaces `PooledSession` — tracks active channel count
- [x] `acquire_channel()` / `release_channel()` replace `checkout()` / `return_session()`
- [x] Load balancing: prefers sessions with fewest active channels
- [x] Stale cleanup skips sessions with active channels
- [x] `max_channels_per_session = 1` restores V1 exclusive behavior
- [x] All SSH tools updated to use channel API
- [x] Config: `max_channels_per_session` (default: 10)
- [x] Backward compatible (set to 1 for V1 behavior)

---
---

# Dependency Changes Summary

| Milestone | New Dependencies |
|-----------|-----------------|
| M6 | None (bollard already has `RemoveContainerOptions`, `CreateContainerOptions`) |
| M7 | None (bollard already has image APIs) |
| M8 | None (refactors existing code) |
| M9 | `prometheus = "0.13"`, `axum = "0.7"` (optional, behind `metrics` feature) |
| M10 | None (refactors existing code) |

---

# Execution Order

M6 and M7 can run in parallel (independent tool additions). M8, M9, and M10 are independent of each other but all depend on M6/M7 being done first (so tool counts are correct).

**Recommended order:**
1. **M6** — Destructive Docker container tools (highest design-doc backing, unblocks confirmation testing)
2. **M7** — Docker image tools (completes the `docker.*` namespace)
3. **M10** — SSH channel multiplexing (deepest refactor, benefits from being done before M8/M9 to avoid rework)
4. **M8** — Per-user rate limiting (rate limiter refactor, small scope)
5. **M9** — Metrics (additive, no conflicts, can be done last)

---

# Post-M10 Considerations

Features mentioned in design docs but not planned here:
- **Remote log aggregator** (security-approach.md line 213): `remote = { url, token_env }` audit transport
- **Spacebot secret store integration** (security-approach.md line 33): depends on upstream API
- **TCP/network MCP transport** (poc-specification.md line 54): "not needed for V1"
- **Expand MCP tools beyond workers** (architecture-decision.md line 179): Spacebot-side Phase 2 change
- **Port to feature flags / in-tree** (architecture-decision.md line 180): Spacebot-side Phase 3 change
