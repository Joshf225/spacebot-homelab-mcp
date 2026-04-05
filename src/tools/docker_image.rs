use anyhow::{Result, anyhow};
use bollard::image::{
    CreateImageOptions, ListImagesOptions, PruneImagesOptions, RemoveImageOptions,
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

            let repo_tags = image.repo_tags.join(", ");
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
        let reclaimed = response.space_reclaimed.unwrap_or(0);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_redact_image_env_config() {
        let mut value = serde_json::json!({
            "Config": {
                "Env": [
                    "SECRET=password123",
                    "PATH=/usr/bin"
                ]
            }
        });

        redact_image_env(&mut value);

        let env = value["Config"]["Env"].as_array().unwrap();
        assert_eq!(env[0], "SECRET=<redacted>");
        assert_eq!(env[1], "PATH=<redacted>");
    }

    #[test]
    fn test_redact_image_env_container_config() {
        let mut value = serde_json::json!({
            "ContainerConfig": {
                "Env": [
                    "DB_PASSWORD=hunter2",
                    "HOME=/root"
                ]
            }
        });

        redact_image_env(&mut value);

        let env = value["ContainerConfig"]["Env"].as_array().unwrap();
        assert_eq!(env[0], "DB_PASSWORD=<redacted>");
        assert_eq!(env[1], "HOME=<redacted>");
    }

    #[test]
    fn test_redact_image_env_both_sections() {
        let mut value = serde_json::json!({
            "Config": {
                "Env": ["A=1"]
            },
            "ContainerConfig": {
                "Env": ["B=2"]
            }
        });

        redact_image_env(&mut value);

        assert_eq!(value["Config"]["Env"][0], "A=<redacted>");
        assert_eq!(value["ContainerConfig"]["Env"][0], "B=<redacted>");
    }

    #[test]
    fn test_redact_image_env_no_env() {
        let mut value = serde_json::json!({
            "Config": {
                "Image": "nginx"
            }
        });

        // Should not panic
        redact_image_env(&mut value);
    }
}
