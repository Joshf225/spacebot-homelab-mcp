use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::signal;
use tokio::time::{Duration, timeout};
use tracing::{info, warn};

mod audit;
mod config;
mod confirmation;
mod connection;
mod health;
mod mcp;
mod rate_limit;
mod tools;

use audit::AuditLogger;
use config::Config;
use connection::ConnectionManager;
use mcp::HomelabMcpServer;

#[derive(Parser, Debug)]
#[command(name = "spacebot-homelab-mcp")]
#[command(about = "MCP server for Docker and SSH homelab tools")]
enum Cli {
    /// Start the MCP server (stdio transport)
    Server {
        /// Path to config file
        #[arg(long, short)]
        config: Option<PathBuf>,
    },

    /// Validate connections and feature availability
    Doctor {
        /// Path to config file
        #[arg(long, short)]
        config: Option<PathBuf>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("spacebot_homelab_mcp=info".parse()?),
        )
        .with_writer(std::io::stderr)
        .init();

    match Cli::parse() {
        Cli::Server { config } => run_server(config).await?,
        Cli::Doctor { config } => run_doctor(config).await?,
    }

    Ok(())
}

async fn run_server(config_path: Option<PathBuf>) -> Result<()> {
    info!("Starting spacebot-homelab-mcp server");

    let config = Arc::new(Config::load(config_path)?);
    info!(
        "Configuration loaded: {} Docker hosts, {} SSH hosts",
        config.docker.len(),
        config.ssh.hosts.len()
    );

    let manager = Arc::new(ConnectionManager::new((*config).clone()).await?);
    let audit = Arc::new(AuditLogger::new(config.clone()));
    info!("Connection manager initialized");

    let health_handle = manager.spawn_health_monitor();
    info!("Health monitor started");

    let server = HomelabMcpServer::new(config, manager.clone(), audit);
    let transport = rmcp::transport::io::stdio();
    let service = rmcp::serve_server(server, transport).await?;
    let cancellation = service.cancellation_token();
    let wait_for_service = service.waiting();
    tokio::pin!(wait_for_service);

    info!("MCP server started, waiting for messages...");

    tokio::select! {
        result = &mut wait_for_service => {
            match result {
                Ok(reason) => info!(?reason, "MCP server connection closed"),
                Err(error) => warn!("MCP server join error: {}", error),
            }
        }
        _ = wait_for_shutdown_signal() => {
            info!("Shutdown signal received, stopping MCP server");
            cancellation.cancel();

            match timeout(Duration::from_secs(10), &mut wait_for_service).await {
                Ok(Ok(reason)) => info!(?reason, "MCP server stopped cleanly"),
                Ok(Err(error)) => warn!("MCP server join error during shutdown: {}", error),
                Err(_) => warn!("Graceful shutdown timed out after 10 seconds"),
            }
        }
    }

    health_handle.abort();
    manager.close_all().await;
    Ok(())
}

async fn run_doctor(config_path: Option<PathBuf>) -> Result<()> {
    println!("Validating spacebot-homelab-mcp configuration...\n");
    let config = Config::load(config_path)?;
    health::run_diagnostics(&config).await?;
    Ok(())
}

async fn wait_for_shutdown_signal() {
    #[cfg(unix)]
    {
        let mut sigterm = signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to register SIGTERM handler");
        tokio::select! {
            _ = signal::ctrl_c() => {},
            _ = sigterm.recv() => {},
        }
    }

    #[cfg(not(unix))]
    {
        let _ = signal::ctrl_c().await;
    }
}
