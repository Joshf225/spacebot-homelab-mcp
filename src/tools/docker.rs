use anyhow::{Result, anyhow};
use bollard::container::{CreateContainerOptions, Config as ContainerConfig, InspectContainerOptions, ListContainersOptions, LogsOptions, RemoveContainerOptions, StartContainerOptions, StopContainerOptions};
use bollard::models::{HostConfig, PortBinding, RestartPolicy, RestartPolicyNameEnum};
use futures::StreamExt;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

use crate::audit::AuditLogger;
use crate::confirmation::ConfirmationManager;
use crate::connection::ConnectionManager;
use crate::tools::{truncate_output, wrap_output_envelope};

const OUTPUT_MAX_CHARS: usize = 10_000;

pub async fn container_list(
    manager: Arc<ConnectionManager>,
    host: Option<String>,
    all: Option<bool>,
    name_filter: Option<String>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let host = host.unwrap_or_else(|| "local".to_string());
    let result: Result<String> = async {
        let docker = manager.get_docker(&host)?;

        // Use Docker API filters for name filtering (server-side, more efficient)
        let mut filters = HashMap::new();
        if let Some(ref name) = name_filter {
            filters.insert("name".to_string(), vec![name.clone()]);
        }

        let containers = docker
            .as_bollard()
            .list_containers(Some(ListContainersOptions::<String> {
                all: all.unwrap_or(false),
                filters,
                ..Default::default()
            }))
            .await
            .map_err(|error| anyhow!("Failed to list containers: {}", error))?;

        if containers.is_empty() {
            return Ok("No containers found.".to_string());
        }

        let mut lines = vec![format!(
            "Docker host: {}\n\n{:<12}  {:<24}  {:<24}  {:<10}  {}",
            host, "ID", "NAME", "IMAGE", "STATE", "STATUS"
        )];

        for container in containers {
            let id = container
                .id
                .as_deref()
                .map(|id| id.chars().take(12).collect::<String>())
                .unwrap_or_else(|| "<unknown>".to_string());
            let name = container
                .names
                .unwrap_or_default()
                .into_iter()
                .map(|name| name.trim_start_matches('/').to_string())
                .collect::<Vec<_>>()
                .join(", ");
            let image = container.image.unwrap_or_else(|| "<unknown>".to_string());
            let state = container.state.unwrap_or_else(|| "unknown".to_string());
            let status = container.status.unwrap_or_default();

            lines.push(format!(
                "{:<12}  {:<24}  {:<24}  {:<10}  {}",
                id, name, image, state, status
            ));
        }

        Ok(truncate_output(&lines.join("\n"), OUTPUT_MAX_CHARS))
    }
    .await;

    match result {
        Ok(output) => {
            audit
                .log("docker.container.list", &host, "success", None)
                .await
                .ok();
            Ok(wrap_output_envelope("docker.container.list", &output))
        }
        Err(error) => {
            audit
                .log(
                    "docker.container.list",
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

pub async fn container_start(
    manager: Arc<ConnectionManager>,
    host: Option<String>,
    container: String,
    dry_run: Option<bool>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let host = host.unwrap_or_else(|| "local".to_string());

    if dry_run.unwrap_or(false) {
        let output = format!(
            "DRY RUN: Would start container '{}' on Docker host '{}'.",
            container, host
        );
        audit
            .log("docker.container.start", &host, "dry_run", Some(&container))
            .await
            .ok();
        return Ok(wrap_output_envelope("docker.container.start", &output));
    }

    let result: Result<String> = async {
        let docker = manager.get_docker(&host)?;
        docker
            .as_bollard()
            .start_container(&container, None::<StartContainerOptions<String>>)
            .await
            .map_err(|error| anyhow!("Failed to start container '{}': {}", container, error))?;

        Ok(format!("Started container '{}' on Docker host '{}'.", container, host))
    }
    .await;

    match result {
        Ok(output) => {
            audit
                .log("docker.container.start", &host, "success", Some(&container))
                .await
                .ok();
            Ok(wrap_output_envelope("docker.container.start", &output))
        }
        Err(error) => {
            audit
                .log(
                    "docker.container.start",
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

pub async fn container_stop(
    manager: Arc<ConnectionManager>,
    host: Option<String>,
    container: String,
    timeout_secs: Option<u64>,
    dry_run: Option<bool>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let host = host.unwrap_or_else(|| "local".to_string());

    if dry_run.unwrap_or(false) {
        let output = format!(
            "DRY RUN: Would stop container '{}' on Docker host '{}'.",
            container, host
        );
        audit
            .log("docker.container.stop", &host, "dry_run", Some(&container))
            .await
            .ok();
        return Ok(wrap_output_envelope("docker.container.stop", &output));
    }

    let result: Result<String> = async {
        let docker = manager.get_docker(&host)?;
        let options = timeout_secs.map(|timeout| StopContainerOptions {
            t: timeout.min(i64::MAX as u64) as i64,
        });

        docker
            .as_bollard()
            .stop_container(&container, options)
            .await
            .map_err(|error| anyhow!("Failed to stop container '{}': {}", container, error))?;

        Ok(format!("Stopped container '{}' on Docker host '{}'.", container, host))
    }
    .await;

    match result {
        Ok(output) => {
            audit
                .log("docker.container.stop", &host, "success", Some(&container))
                .await
                .ok();
            Ok(wrap_output_envelope("docker.container.stop", &output))
        }
        Err(error) => {
            audit
                .log(
                    "docker.container.stop",
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

pub async fn container_logs(
    manager: Arc<ConnectionManager>,
    host: Option<String>,
    container: String,
    tail: Option<usize>,
    since: Option<String>,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let host = host.unwrap_or_else(|| "local".to_string());
    let result: Result<String> = async {
        let docker = manager.get_docker(&host)?;
        let mut stream = docker.as_bollard().logs(
            &container,
            Some(LogsOptions::<String> {
                follow: false,
                stdout: true,
                stderr: true,
                since: since
                    .as_deref()
                    .and_then(|value| value.parse::<i64>().ok())
                    .unwrap_or(0),
                timestamps: false,
                tail: tail.unwrap_or(100).to_string(),
                ..Default::default()
            }),
        );

        let mut output = String::new();
        while let Some(item) = stream.next().await {
            let item = item.map_err(|error| anyhow!("Failed to stream logs: {}", error))?;
            output.push_str(&item.to_string());
        }

        if output.trim().is_empty() {
            output = "No logs returned.".to_string();
        }

        Ok(truncate_output(&output, OUTPUT_MAX_CHARS))
    }
    .await;

    match result {
        Ok(output) => {
            audit
                .log("docker.container.logs", &host, "success", Some(&container))
                .await
                .ok();
            Ok(wrap_output_envelope("docker.container.logs", &output))
        }
        Err(error) => {
            audit
                .log(
                    "docker.container.logs",
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

pub async fn container_inspect(
    manager: Arc<ConnectionManager>,
    host: Option<String>,
    container: String,
    audit: Arc<AuditLogger>,
) -> Result<String> {
    let host = host.unwrap_or_else(|| "local".to_string());
    let result: Result<String> = async {
        let docker = manager.get_docker(&host)?;
        let details = docker
            .as_bollard()
            .inspect_container(&container, None::<InspectContainerOptions>)
            .await
            .map_err(|error| anyhow!("Failed to inspect container '{}': {}", container, error))?;

        let mut value = serde_json::to_value(details)?;
        redact_env_values(&mut value);
        let pretty = serde_json::to_string_pretty(&value)?;
        Ok(truncate_output(&pretty, OUTPUT_MAX_CHARS))
    }
    .await;

    match result {
        Ok(output) => {
            audit
                .log("docker.container.inspect", &host, "success", Some(&container))
                .await
                .ok();
            Ok(wrap_output_envelope("docker.container.inspect", &output))
        }
        Err(error) => {
            audit
                .log(
                    "docker.container.inspect",
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

fn redact_env_values(value: &mut Value) {
    if let Some(env_values) = value
        .get_mut("Config")
        .and_then(|config| config.get_mut("Env"))
        .and_then(Value::as_array_mut)
    {
        for env_value in env_values {
            if let Some(entry) = env_value.as_str() {
                let redacted = entry
                    .split_once('=')
                    .map(|(key, _)| format!("{}=<redacted>", key))
                    .unwrap_or_else(|| entry.to_string());
                *env_value = Value::String(redacted);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_redact_env_values() {
        let mut value = serde_json::json!({
            "Config": {
                "Env": [
                    "SECRET=password123",
                    "PATH=/usr/bin"
                ]
            }
        });

        redact_env_values(&mut value);

        let env = value["Config"]["Env"].as_array().unwrap();
        assert_eq!(env[0], "SECRET=<redacted>");
        assert_eq!(env[1], "PATH=<redacted>");
    }

    #[test]
    fn test_container_name_validation() {
        // Valid names
        assert!("my-container".chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-'));
        assert!("webapp.v2".chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-'));

        // Invalid names
        assert!(!"my container".chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-'));
        assert!(!"rm -rf /".chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-'));
    }
}
