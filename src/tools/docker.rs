use anyhow::{Result, anyhow};
use bollard::container::{InspectContainerOptions, ListContainersOptions, LogsOptions, StartContainerOptions, StopContainerOptions};
use futures::StreamExt;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

use crate::audit::AuditLogger;
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
}
