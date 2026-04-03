use std::sync::Arc;

use rmcp::{ServerHandler, tool, tool_handler, tool_router};
use rmcp::handler::server::{router::tool::ToolRouter, wrapper::Parameters};
use rmcp::model::{Implementation, ServerCapabilities, ServerInfo};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::audit::AuditLogger;
use crate::config::Config;
use crate::confirmation::ConfirmationManager;
use crate::connection::ConnectionManager;
use crate::rate_limit::RateLimiter;
use crate::tools::{docker, ssh};

#[derive(Debug, Deserialize, JsonSchema)]
struct DockerContainerListArgs {
    pub host: Option<String>,
    pub all: Option<bool>,
    pub name_filter: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct DockerContainerActionArgs {
    pub host: Option<String>,
    pub container: String,
    pub dry_run: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct DockerContainerStopArgs {
    pub host: Option<String>,
    pub container: String,
    pub timeout: Option<u64>,
    pub dry_run: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct DockerContainerLogsArgs {
    pub host: Option<String>,
    pub container: String,
    pub tail: Option<usize>,
    pub since: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SshExecArgs {
    pub host: String,
    pub command: String,
    pub timeout: Option<u64>,
    pub dry_run: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SshFileTransferArgs {
    pub host: String,
    pub local_path: String,
    pub remote_path: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SshDownloadArgs {
    pub host: String,
    pub remote_path: String,
    pub local_path: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ConfirmOperationArgs {
    pub token: String,
    pub tool_name: String,
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for HomelabMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                env!("CARGO_PKG_NAME"),
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions("Homelab Docker and SSH tools for Spacebot")
    }
}

/// MCP server with Docker, SSH, rate limiting, and confirmation flow.
#[derive(Clone)]
pub struct HomelabMcpServer {
    config: Arc<Config>,
    manager: Arc<ConnectionManager>,
    audit: Arc<AuditLogger>,
    rate_limiter: Arc<RateLimiter>,
    confirmation: Arc<ConfirmationManager>,
    tool_router: ToolRouter<Self>,
}

impl HomelabMcpServer {
    pub fn new(config: Arc<Config>, manager: Arc<ConnectionManager>, audit: Arc<AuditLogger>) -> Self {
        let rate_limiter = Arc::new(RateLimiter::from_config(&config.rate_limits.limits));
        let confirmation = Arc::new(ConfirmationManager::new(
            config.confirm.clone().unwrap_or_default(),
        ));

        Self {
            config,
            manager,
            audit,
            rate_limiter,
            confirmation,
            tool_router: Self::tool_router(),
        }
    }

    fn ensure_tool_available(&self, tool_name: &str) -> Result<(), String> {
        if !self.config.tools.is_enabled(tool_name) {
            return Err(format!(
                "Tool '{}' is disabled by configuration.",
                tool_name
            ));
        }

        self.rate_limiter
            .check(tool_name)
            .map_err(|error| error.to_string())
    }
}

#[tool_router(router = tool_router)]
impl HomelabMcpServer {
    #[tool(name = "docker.container.list", description = "List Docker containers")]
    async fn docker_container_list(
        &self,
        Parameters(args): Parameters<DockerContainerListArgs>,
    ) -> Result<String, String> {
        self.ensure_tool_available("docker.container.list")?;
        docker::container_list(
            self.manager.clone(),
            args.host,
            args.all,
            args.name_filter,
            self.audit.clone(),
        )
        .await
        .map_err(|error| error.to_string())
    }

    #[tool(name = "docker.container.start", description = "Start a Docker container")]
    async fn docker_container_start(
        &self,
        Parameters(args): Parameters<DockerContainerActionArgs>,
    ) -> Result<String, String> {
        self.ensure_tool_available("docker.container.start")?;
        docker::container_start(
            self.manager.clone(),
            args.host,
            args.container,
            args.dry_run,
            self.audit.clone(),
        )
        .await
        .map_err(|error| error.to_string())
    }

    #[tool(name = "docker.container.stop", description = "Stop a Docker container")]
    async fn docker_container_stop(
        &self,
        Parameters(args): Parameters<DockerContainerStopArgs>,
    ) -> Result<String, String> {
        self.ensure_tool_available("docker.container.stop")?;
        docker::container_stop(
            self.manager.clone(),
            args.host,
            args.container,
            args.timeout,
            args.dry_run,
            self.audit.clone(),
        )
        .await
        .map_err(|error| error.to_string())
    }

    #[tool(name = "docker.container.logs", description = "Get Docker container logs")]
    async fn docker_container_logs(
        &self,
        Parameters(args): Parameters<DockerContainerLogsArgs>,
    ) -> Result<String, String> {
        self.ensure_tool_available("docker.container.logs")?;
        docker::container_logs(
            self.manager.clone(),
            args.host,
            args.container,
            args.tail,
            args.since,
            self.audit.clone(),
        )
        .await
        .map_err(|error| error.to_string())
    }

    #[tool(name = "docker.container.inspect", description = "Inspect a Docker container")]
    async fn docker_container_inspect(
        &self,
        Parameters(args): Parameters<DockerContainerActionArgs>,
    ) -> Result<String, String> {
        self.ensure_tool_available("docker.container.inspect")?;
        docker::container_inspect(
            self.manager.clone(),
            args.host,
            args.container,
            self.audit.clone(),
        )
        .await
        .map_err(|error| error.to_string())
    }

    #[tool(name = "ssh.exec", description = "Execute a command on a remote host via SSH")]
    async fn ssh_exec(
        &self,
        Parameters(args): Parameters<SshExecArgs>,
    ) -> Result<String, String> {
        self.ensure_tool_available("ssh.exec")?;
        ssh::exec(
            self.manager.clone(),
            self.confirmation.clone(),
            args.host,
            args.command,
            args.timeout,
            args.dry_run,
            self.audit.clone(),
        )
        .await
        .map_err(|error| error.to_string())
    }

    #[tool(name = "ssh.upload", description = "Upload a file to a remote host via SFTP")]
    async fn ssh_upload(
        &self,
        Parameters(args): Parameters<SshFileTransferArgs>,
    ) -> Result<String, String> {
        self.ensure_tool_available("ssh.upload")?;
        ssh::upload(
            self.manager.clone(),
            args.host,
            args.local_path,
            args.remote_path,
            self.audit.clone(),
        )
        .await
        .map_err(|error| error.to_string())
    }

    #[tool(name = "ssh.download", description = "Download a file from a remote host via SFTP")]
    async fn ssh_download(
        &self,
        Parameters(args): Parameters<SshDownloadArgs>,
    ) -> Result<String, String> {
        self.ensure_tool_available("ssh.download")?;
        ssh::download(
            self.manager.clone(),
            args.host,
            args.remote_path,
            args.local_path,
            self.audit.clone(),
        )
        .await
        .map_err(|error| error.to_string())
    }

    #[tool(name = "confirm_operation", description = "Confirm a previously requested destructive operation")]
    async fn confirm_operation(
        &self,
        Parameters(args): Parameters<ConfirmOperationArgs>,
    ) -> Result<String, String> {
        // confirm_operation is exempt from tools.enabled check — it must always be
        // available, otherwise a confirmation flow would break if the operator forgot
        // to include it in the enabled list. Rate limiting still applies.
        self.rate_limiter
            .check("confirm_operation")
            .map_err(|error| error.to_string())?;

        let original_params_json = self
            .confirmation
            .confirm(&args.token, &args.tool_name)
            .await
            .map_err(|error| error.to_string())?;

        self.audit
            .log(
                "confirm_operation",
                "n/a",
                "confirmed",
                Some(&format!("tool={} token={}", args.tool_name, args.token)),
            )
            .await
            .ok();

        match args.tool_name.as_str() {
            "ssh.exec" => {
                let params: SshExecArgs = serde_json::from_str(&original_params_json)
                    .map_err(|error| error.to_string())?;
                ssh::exec_confirmed(
                    self.manager.clone(),
                    params.host,
                    params.command,
                    params.timeout,
                    self.audit.clone(),
                )
                .await
                .map_err(|error| error.to_string())
            }
            "docker.container.start" => {
                let params: DockerContainerActionArgs =
                    serde_json::from_str(&original_params_json)
                        .map_err(|error| error.to_string())?;
                self.audit
                    .log(
                        "docker.container.start",
                        params.host.as_deref().unwrap_or("local"),
                        "confirmed_exec",
                        Some(&params.container),
                    )
                    .await
                    .ok();
                docker::container_start(
                    self.manager.clone(),
                    params.host,
                    params.container,
                    None,
                    self.audit.clone(),
                )
                .await
                .map_err(|error| error.to_string())
            }
            "docker.container.stop" => {
                let params: DockerContainerStopArgs =
                    serde_json::from_str(&original_params_json)
                        .map_err(|error| error.to_string())?;
                self.audit
                    .log(
                        "docker.container.stop",
                        params.host.as_deref().unwrap_or("local"),
                        "confirmed_exec",
                        Some(&params.container),
                    )
                    .await
                    .ok();
                docker::container_stop(
                    self.manager.clone(),
                    params.host,
                    params.container,
                    params.timeout,
                    None,
                    self.audit.clone(),
                )
                .await
                .map_err(|error| error.to_string())
            }
            _ => Err(format!(
                "Confirmation not supported for tool '{}'.",
                args.tool_name
            )),
        }
    }
}
