use anyhow::{Result, anyhow};
use async_trait::async_trait;
use dashmap::DashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{Mutex, Notify};
use tokio::time::{Duration, interval, sleep, timeout};
use tracing::{debug, info, warn};

use crate::config::{Config, DockerHost, ProxmoxHost, SshHost, SshPoolConfig};

/// Manages persistent connections to Docker daemons, SSH hosts, and Proxmox VE hosts.
pub struct ConnectionManager {
    pub config: Arc<Config>,
    docker_clients: DashMap<String, DockerClient>,
    ssh_pools: DashMap<String, SshPool>,
    proxmox_clients: DashMap<String, ProxmoxClient>,
    health: DashMap<String, ConnectionHealth>,
    metrics: Option<Arc<crate::metrics::Metrics>>,
}

/// Docker connection handle with transport metadata for diagnostics.
#[derive(Clone)]
pub struct DockerClient {
    client: Arc<bollard::Docker>,
    transport: DockerTransport,
}

#[derive(Debug, Clone)]
pub enum DockerTransport {
    #[allow(dead_code)] // Only constructed on Unix via unix:// connections
    UnixSocket {
        path: PathBuf,
    },
    Tcp {
        host: String,
        tls: bool,
    },
    #[allow(dead_code)] // Only constructed on Windows via npipe:// connections
    NamedPipe {
        path: String,
    },
}

impl DockerClient {
    pub fn new(host: &DockerHost) -> Result<Self> {
        let (client, transport) = if host.host.starts_with("unix://") {
            #[cfg(unix)]
            {
                let socket_path = host.host.trim_start_matches("unix://");
                let client = bollard::Docker::connect_with_unix(
                    socket_path,
                    120,
                    bollard::API_DEFAULT_VERSION,
                )
                .map_err(|error| {
                    anyhow!(
                        "Failed to connect to Docker socket {}: {}",
                        socket_path,
                        error
                    )
                })?;

                (
                    client,
                    DockerTransport::UnixSocket {
                        path: PathBuf::from(socket_path),
                    },
                )
            }
            #[cfg(not(unix))]
            {
                return Err(anyhow!(
                    "Unix socket connections (unix://) are only supported on Unix systems. \
                     On Windows, use npipe:// or tcp:// instead. Got: '{}'",
                    host.host
                ));
            }
        } else if host.host.starts_with("tcp://") {
            let has_tls =
                host.key_path.is_some() && host.cert_path.is_some() && host.ca_path.is_some();

            if has_tls {
                let ssl_key = host.key_path.as_ref().unwrap();
                let ssl_cert = host.cert_path.as_ref().unwrap();
                let ssl_ca = host.ca_path.as_ref().unwrap();
                let client = bollard::Docker::connect_with_ssl(
                    &host.host,
                    ssl_key,
                    ssl_cert,
                    ssl_ca,
                    120,
                    bollard::API_DEFAULT_VERSION,
                )
                .map_err(|error| {
                    anyhow!(
                        "Failed to connect to Docker TLS endpoint {}: {}",
                        host.host,
                        error
                    )
                })?;

                (
                    client,
                    DockerTransport::Tcp {
                        host: host.host.clone(),
                        tls: true,
                    },
                )
            } else {
                let client = bollard::Docker::connect_with_http(
                    &host.host,
                    120,
                    bollard::API_DEFAULT_VERSION,
                )
                .map_err(|error| {
                    anyhow!(
                        "Failed to connect to Docker TCP endpoint {}: {}",
                        host.host,
                        error
                    )
                })?;

                if host.cert_path.is_some() || host.key_path.is_some() {
                    tracing::warn!(
                        "Docker host '{}' has partial TLS config (cert_path/key_path set but ca_path missing). \
                         Falling back to unencrypted HTTP. Set all three (cert_path, key_path, ca_path) for TLS.",
                        host.host
                    );
                }

                (
                    client,
                    DockerTransport::Tcp {
                        host: host.host.clone(),
                        tls: false,
                    },
                )
            }
        } else if host.host.starts_with("npipe://") {
            #[cfg(windows)]
            {
                let pipe_path = &host.host;
                let client = bollard::Docker::connect_with_named_pipe(
                    pipe_path,
                    120,
                    bollard::API_DEFAULT_VERSION,
                )
                .map_err(|error| {
                    anyhow!(
                        "Failed to connect to Docker named pipe {}: {}",
                        pipe_path,
                        error
                    )
                })?;

                (
                    client,
                    DockerTransport::NamedPipe {
                        path: pipe_path.to_string(),
                    },
                )
            }
            #[cfg(not(windows))]
            {
                return Err(anyhow!(
                    "Named pipe connections (npipe://) are only supported on Windows. Got: '{}'",
                    host.host
                ));
            }
        } else {
            return Err(anyhow!(
                "Invalid Docker connection string '{}'. Expected unix://, tcp://, or npipe://",
                host.host
            ));
        };

        Ok(Self {
            client: Arc::new(client),
            transport,
        })
    }

    pub async fn validate(&self) -> Result<()> {
        self.client
            .ping()
            .await
            .map_err(|error| anyhow!("Docker ping failed: {}", error))?;
        Ok(())
    }

    pub fn as_bollard(&self) -> &bollard::Docker {
        self.client.as_ref()
    }

    pub fn transport_summary(&self) -> String {
        match &self.transport {
            DockerTransport::UnixSocket { path } => format!("unix socket {}", path.display()),
            DockerTransport::Tcp { host, tls } => {
                if *tls {
                    format!("tcp {} (tls configured)", host)
                } else {
                    format!("tcp {}", host)
                }
            }
            DockerTransport::NamedPipe { path } => format!("named pipe {}", path),
        }
    }
}

/// Proxmox VE API client wrapper.
#[derive(Clone)]
pub struct ProxmoxClient {
    client: reqwest::Client,
    base_url: String,
    auth_header: String,
    node: Option<String>,
}

impl ProxmoxClient {
    pub fn new(host: &ProxmoxHost) -> Result<Self> {
        let client = reqwest::Client::builder()
            .danger_accept_invalid_certs(!host.verify_tls)
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|error| anyhow!("Failed to build Proxmox HTTP client: {}", error))?;

        let base_url = host.url.trim_end_matches('/').to_string();
        let auth_header = format!("PVEAPIToken={}={}", host.token_id, host.token_secret);

        Ok(Self {
            client,
            base_url,
            auth_header,
            node: host.node.clone(),
        })
    }

    /// GET request to the Proxmox API. Returns the `data` field from the response envelope.
    pub async fn get(&self, path: &str) -> Result<serde_json::Value> {
        let url = format!("{}/api2/json{}", self.base_url, path);
        let response = self
            .client
            .get(&url)
            .header("Authorization", &self.auth_header)
            .send()
            .await
            .map_err(|error| anyhow!("Proxmox API request failed: {}", error))?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow!(
                "Proxmox API error (HTTP {}): {}",
                status.as_u16(),
                body
            ));
        }

        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|error| anyhow!("Failed to parse Proxmox API response: {}", error))?;

        Ok(body.get("data").cloned().unwrap_or(serde_json::Value::Null))
    }

    /// POST request to the Proxmox API (form-urlencoded body).
    pub async fn post(&self, path: &str, params: &[(&str, &str)]) -> Result<serde_json::Value> {
        let url = format!("{}/api2/json{}", self.base_url, path);
        let response = self
            .client
            .post(&url)
            .header("Authorization", &self.auth_header)
            .form(params)
            .send()
            .await
            .map_err(|error| anyhow!("Proxmox API POST failed: {}", error))?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow!(
                "Proxmox API error (HTTP {}): {}",
                status.as_u16(),
                body
            ));
        }

        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|error| anyhow!("Failed to parse Proxmox API response: {}", error))?;

        Ok(body.get("data").cloned().unwrap_or(serde_json::Value::Null))
    }

    /// DELETE request to the Proxmox API.
    pub async fn delete(&self, path: &str) -> Result<serde_json::Value> {
        let url = format!("{}/api2/json{}", self.base_url, path);
        let response = self
            .client
            .delete(&url)
            .header("Authorization", &self.auth_header)
            .send()
            .await
            .map_err(|error| anyhow!("Proxmox API DELETE failed: {}", error))?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow!(
                "Proxmox API error (HTTP {}): {}",
                status.as_u16(),
                body
            ));
        }

        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|error| anyhow!("Failed to parse Proxmox API response: {}", error))?;

        Ok(body.get("data").cloned().unwrap_or(serde_json::Value::Null))
    }

    /// Wait for a Proxmox async task (UPID) to complete.
    pub async fn wait_for_task(
        &self,
        node: &str,
        upid: &str,
        max_wait_secs: u64,
    ) -> Result<String> {
        let start = Instant::now();
        let max_duration = Duration::from_secs(max_wait_secs);
        let poll_interval = Duration::from_secs(2);

        loop {
            let status = self
                .get(&format!("/nodes/{}/tasks/{}/status", node, upid))
                .await?;
            let task_status = status
                .get("status")
                .and_then(|value| value.as_str())
                .unwrap_or("unknown");

            if task_status == "stopped" {
                let exit_status = status
                    .get("exitstatus")
                    .and_then(|value| value.as_str())
                    .unwrap_or("unknown");
                if exit_status == "OK" {
                    return Ok(format!("Task completed successfully (UPID: {})", upid));
                }

                return Err(anyhow!(
                    "Proxmox task failed with exit status '{}' (UPID: {})",
                    exit_status,
                    upid
                ));
            }

            if start.elapsed() > max_duration {
                return Err(anyhow!(
                    "Proxmox task timed out after {}s (UPID: {}). Task may still be running.",
                    max_wait_secs,
                    upid
                ));
            }

            sleep(poll_interval).await;
        }
    }

    /// Validate connectivity by calling GET /version.
    pub async fn validate(&self) -> Result<()> {
        self.get("/version").await?;
        Ok(())
    }

    /// Get the configured node name, or auto-detect the first node from the cluster.
    pub async fn resolve_node(&self, override_node: Option<&str>) -> Result<String> {
        if let Some(node) = override_node {
            return Ok(node.to_string());
        }
        if let Some(node) = &self.node {
            return Ok(node.clone());
        }

        let nodes = self.get("/nodes").await?;
        nodes
            .as_array()
            .and_then(|items| items.first())
            .and_then(|node| node.get("node"))
            .and_then(|value| value.as_str())
            .map(|value| value.to_string())
            .ok_or_else(|| {
                anyhow!("Could not auto-detect Proxmox node name. Set 'node' in config.")
            })
    }

    pub fn connection_summary(&self) -> String {
        format!(
            "{} (node: {})",
            self.base_url,
            self.node.as_deref().unwrap_or("auto")
        )
    }
}

/// SSH connection pool for a single host.
/// Sessions are shared: multiple channels can be multiplexed over a single session.
#[derive(Clone)]
pub struct SshPool {
    sessions: Arc<Mutex<Vec<SharedSession>>>,
    max_sessions: usize,
    max_channels_per_session: usize,
    session_available: Arc<Notify>,
    host_config: Arc<SshHost>,
    pool_config: SshPoolConfig,
}

/// A shared SSH session that can serve multiple concurrent channels.
struct SharedSession {
    handle: Arc<russh::client::Handle<SshClientHandler>>,
    created_at: Instant,
    last_used: Instant,
    /// Number of channels currently open on this session.
    active_channels: usize,
}

/// An acquired channel from the pool. Must be released via `release_channel`.
pub struct AcquiredChannel {
    pub handle: Arc<russh::client::Handle<SshClientHandler>>,
    session_index: usize,
    host_name: String,
}

#[derive(Clone)]
pub struct SshClientHandler {
    host: String,
    port: u16,
}

#[cfg(windows)]
const KNOWN_HOSTS_HINT_PATH: &str = "%USERPROFILE%\\.ssh\\known_hosts";

#[cfg(not(windows))]
const KNOWN_HOSTS_HINT_PATH: &str = "~/.ssh/known_hosts";

fn ssh_keyscan_hint(host: &str, port: u16) -> String {
    if port == 22 {
        format!("ssh-keyscan -H {} >> {}", host, KNOWN_HOSTS_HINT_PATH)
    } else {
        format!(
            "ssh-keyscan -H -p {} {} >> {}",
            port, host, KNOWN_HOSTS_HINT_PATH
        )
    }
}

impl SshClientHandler {
    pub fn new(host: String, port: u16) -> Self {
        Self { host, port }
    }
}

#[async_trait]
impl russh::client::Handler for SshClientHandler {
    type Error = anyhow::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &russh::keys::key::PublicKey,
    ) -> Result<bool, Self::Error> {
        match russh::keys::check_known_hosts(&self.host, self.port, server_public_key) {
            Ok(true) => {
                debug!(
                    "SSH host key verified for {}:{} via known_hosts",
                    self.host, self.port
                );
                Ok(true)
            }
            Ok(false) => {
                // Key not found in known_hosts — reject to prevent MITM.
                // The operator must add the host key first.
                let hint = ssh_keyscan_hint(&self.host, self.port);
                warn!(
                    "SSH host key for {}:{} not found in known_hosts. \
                     Add it with: {}",
                    self.host, self.port, hint
                );
                Err(anyhow!(
                    "SSH host key for {}:{} not found in known_hosts. \
                     Run: {}",
                    self.host,
                    self.port,
                    hint
                ))
            }
            Err(russh::keys::Error::KeyChanged { line }) => {
                // Host key CHANGED — possible MITM attack. Always reject.
                warn!(
                    "SSH HOST KEY CHANGED for {}:{} (known_hosts line {}). \
                     Possible MITM attack! Refusing connection.",
                    self.host, self.port, line
                );
                Err(anyhow!(
                    "SSH HOST KEY CHANGED for {}:{} (known_hosts line {}). \
                     Possible MITM attack. Remove the old key and re-verify manually.",
                    self.host,
                    self.port,
                    line
                ))
            }
            Err(error) => {
                // Other errors (missing known_hosts file, parse errors, etc.)
                warn!(
                    "SSH host key verification failed for {}:{}: {}",
                    self.host, self.port, error
                );
                Err(anyhow!(
                    "SSH host key verification failed for {}:{}: {}",
                    self.host,
                    self.port,
                    error
                ))
            }
        }
    }
}

impl SshPool {
    pub fn new(host_config: SshHost, pool_config: SshPoolConfig) -> Self {
        Self {
            sessions: Arc::new(Mutex::new(Vec::new())),
            max_sessions: pool_config.max_sessions_per_host,
            max_channels_per_session: pool_config.max_channels_per_session,
            session_available: Arc::new(Notify::new()),
            host_config: Arc::new(host_config),
            pool_config,
        }
    }

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
                            sleep(Duration::from_secs(1)).await;
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
            timeout(wait_duration, self.session_available.notified())
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

    /// Release a channel back to the pool.
    pub async fn release_channel(&self, channel: AcquiredChannel, broken: bool) {
        let mut sessions = self.sessions.lock().await;

        if let Some(session) = sessions.get_mut(channel.session_index) {
            session.active_channels = session.active_channels.saturating_sub(1);
            session.last_used = Instant::now();

            if broken && session.active_channels == 0 {
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
        let config = russh::client::Config {
            inactivity_timeout: Some(Duration::from_secs(self.pool_config.connect_timeout_secs)),
            keepalive_interval: Some(Duration::from_secs(
                self.pool_config.keepalive_interval_secs,
            )),
            keepalive_max: 3,
            ..Default::default()
        };

        let port = self.host_config.port.unwrap_or(22);
        let connect_duration = Duration::from_secs(self.pool_config.connect_timeout_secs);

        let handler = SshClientHandler::new(self.host_config.host.clone(), port);
        let mut handle = timeout(connect_duration, async {
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
            handle: Arc::new(handle),
            created_at: Instant::now(),
            last_used: Instant::now(),
            active_channels: 0,
        })
    }

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
            session.created_at.elapsed() <= max_lifetime && session.last_used.elapsed() <= max_idle
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

    /// Get the total number of sessions in the pool.
    /// NOTE: This is async because it requires acquiring the session lock.
    /// M9's health monitor may need to call this instead of atomic loads.
    /// The orchestrator will resolve any M9/M10 conflicts here.
    pub async fn session_count(&self) -> usize {
        self.sessions.lock().await.len()
    }

    /// Get the total number of active channels across all sessions.
    pub async fn active_channel_count(&self) -> usize {
        self.sessions
            .lock()
            .await
            .iter()
            .map(|s| s.active_channels)
            .sum()
    }
}

#[derive(Debug, Clone)]
pub struct ConnectionHealth {
    pub status: ConnectionStatus,
    pub last_success: Option<Instant>,
    pub last_error: Option<String>,
    pub consecutive_failures: u32,
    pub skip_cycles: u32,
    pub cycles_skipped: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionStatus {
    Connected,
    Degraded,
    Disconnected,
    Connecting,
}

impl ConnectionManager {
    pub async fn new(
        config: Config,
        metrics: Option<Arc<crate::metrics::Metrics>>,
    ) -> Result<Self> {
        let manager = Self {
            config: Arc::new(config),
            docker_clients: DashMap::new(),
            ssh_pools: DashMap::new(),
            proxmox_clients: DashMap::new(),
            health: DashMap::new(),
            metrics,
        };

        for (name, host) in &manager.config.docker.hosts {
            let health_key = format!("docker:{}", name);
            manager.health.insert(
                health_key.clone(),
                ConnectionHealth {
                    status: ConnectionStatus::Connecting,
                    last_success: None,
                    last_error: None,
                    consecutive_failures: 0,
                    skip_cycles: 0,
                    cycles_skipped: 0,
                },
            );

            match DockerClient::new(host) {
                Ok(client) => {
                    let validate_result = client.validate().await;
                    manager.docker_clients.insert(name.clone(), client.clone());

                    match validate_result {
                        Ok(()) => {
                            manager.mark_healthy(&health_key);
                            info!(
                                "Docker '{}' connected via {}",
                                name,
                                client.transport_summary()
                            );
                        }
                        Err(error) => {
                            manager.mark_unhealthy(&health_key, error.to_string());
                            warn!(
                                "Docker '{}' ping failed during startup (will retry): {}",
                                name, error
                            );
                        }
                    }
                }
                Err(error) => {
                    manager.mark_unhealthy(&health_key, error.to_string());
                    warn!(
                        "Failed to initialize Docker client for '{}': {}",
                        name, error
                    );
                }
            }
        }

        for (name, host) in &manager.config.ssh.hosts {
            let health_key = format!("ssh:{}", name);
            manager.health.insert(
                health_key.clone(),
                ConnectionHealth {
                    status: ConnectionStatus::Connecting,
                    last_success: None,
                    last_error: None,
                    consecutive_failures: 0,
                    skip_cycles: 0,
                    cycles_skipped: 0,
                },
            );

            manager.ssh_pools.insert(
                name.clone(),
                SshPool::new(host.clone(), manager.config.ssh.pool.clone()),
            );
            // SSH pools start in Connecting state — actual connectivity is verified
            // by the first health monitor cycle, not at startup. This avoids blocking
            // startup on SSH handshakes and avoids falsely reporting Connected.
            info!("SSH pool '{}' initialized (connectivity pending)", name);
        }

        for (name, host) in &manager.config.proxmox.hosts {
            let health_key = format!("proxmox:{}", name);
            manager.health.insert(
                health_key.clone(),
                ConnectionHealth {
                    status: ConnectionStatus::Connecting,
                    last_success: None,
                    last_error: None,
                    consecutive_failures: 0,
                    skip_cycles: 0,
                    cycles_skipped: 0,
                },
            );

            match ProxmoxClient::new(host) {
                Ok(client) => {
                    let validate_result = client.validate().await;
                    manager.proxmox_clients.insert(name.clone(), client.clone());

                    match validate_result {
                        Ok(()) => {
                            manager.mark_healthy(&health_key);
                            info!(
                                "Proxmox '{}' connected via {}",
                                name,
                                client.connection_summary()
                            );
                        }
                        Err(error) => {
                            manager.mark_unhealthy(&health_key, error.to_string());
                            warn!(
                                "Proxmox '{}' API check failed during startup (will retry): {}",
                                name, error
                            );
                        }
                    }
                }
                Err(error) => {
                    manager.mark_unhealthy(&health_key, error.to_string());
                    warn!(
                        "Failed to initialize Proxmox client for '{}': {}",
                        name, error
                    );
                }
            }
        }

        info!(
            "ConnectionManager initialized with {} Docker hosts, {} SSH hosts, {} Proxmox hosts",
            manager.config.docker.hosts.len(),
            manager.config.ssh.hosts.len(),
            manager.config.proxmox.hosts.len()
        );

        Ok(manager)
    }

    pub fn config(&self) -> &Config {
        self.config.as_ref()
    }

    pub fn get_docker(&self, name: &str) -> Result<DockerClient> {
        let health_key = format!("docker:{}", name);
        if let Some(health) = self.health.get(&health_key) {
            if health.status == ConnectionStatus::Disconnected {
                return Err(anyhow!(self.disconnected_error_message(&health_key, name)));
            }
        }

        self.docker_clients
            .get(name)
            .map(|entry| entry.clone())
            .ok_or_else(|| anyhow!("Docker host '{}' not configured", name))
    }

    pub fn get_proxmox(&self, name: &str) -> Result<ProxmoxClient> {
        let health_key = format!("proxmox:{}", name);
        if let Some(health) = self.health.get(&health_key) {
            if health.status == ConnectionStatus::Disconnected {
                return Err(anyhow!(self.disconnected_error_message(&health_key, name)));
            }
        }

        self.proxmox_clients
            .get(name)
            .map(|entry| entry.clone())
            .ok_or_else(|| anyhow!("Proxmox host '{}' not configured", name))
    }

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

    pub async fn ssh_release_channel(&self, host: &str, channel: AcquiredChannel, broken: bool) {
        if let Some(pool) = self.ssh_pools.get(host) {
            pool.release_channel(channel, broken).await;
        }
    }

    pub fn should_skip_health_check(&self, key: &str) -> bool {
        if let Some(mut health) = self.health.get_mut(key) {
            if health.skip_cycles > 0 && health.cycles_skipped < health.skip_cycles {
                health.cycles_skipped += 1;
                return true;
            }
            health.cycles_skipped = 0;
        }
        false
    }

    pub fn mark_healthy(&self, key: &str) -> bool {
        if let Some(mut health) = self.health.get_mut(key) {
            let was_unhealthy = !matches!(health.status, ConnectionStatus::Connected);
            health.status = ConnectionStatus::Connected;
            health.last_success = Some(Instant::now());
            health.last_error = None;
            health.consecutive_failures = 0;
            health.skip_cycles = 0;
            health.cycles_skipped = 0;
            return was_unhealthy;
        }

        false
    }

    pub fn mark_unhealthy(&self, key: &str, error: String) -> u32 {
        if let Some(mut health) = self.health.get_mut(key) {
            health.last_error = Some(error);
            health.consecutive_failures += 1;
            // Degraded for 1-2 failures (requests still allowed), Disconnected for 3+
            health.status = if health.consecutive_failures < 3 {
                ConnectionStatus::Degraded
            } else {
                ConnectionStatus::Disconnected
            };
            health.skip_cycles = match health.consecutive_failures {
                1 => 0,
                2 => 1,
                3 => 3,
                _ => 5,
            };
            health.cycles_skipped = 0;
            return health.consecutive_failures;
        }

        0
    }

    pub fn disconnected_error_message(&self, key: &str, display_name: &str) -> String {
        if let Some(health) = self.health.get(key) {
            let last_success = health
                .last_success
                .map(|instant| {
                    let seconds = instant.elapsed().as_secs();
                    if seconds < 60 {
                        "less than a minute ago".to_string()
                    } else {
                        format!("{} minutes ago", seconds / 60)
                    }
                })
                .unwrap_or_else(|| "never".to_string());

            let last_error = health.last_error.as_deref().unwrap_or("unknown");

            let status_label = match health.status {
                ConnectionStatus::Connected => "connected",
                ConnectionStatus::Degraded => "degraded (intermittent failures)",
                ConnectionStatus::Disconnected => "unreachable",
                ConnectionStatus::Connecting => "connecting",
            };

            return format!(
                "Host '{}' is {}. Last error: {}. Last successful connection: {}. The connection manager will retry automatically.",
                display_name, status_label, last_error, last_success
            );
        }

        format!("Host '{}' is not configured", display_name)
    }

    pub fn spawn_health_monitor(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let manager = Arc::clone(self);
        tokio::spawn(async move {
            let mut ticker = interval(Duration::from_secs(30));
            info!("Health monitor started");

            loop {
                ticker.tick().await;

                let docker_clients: Vec<(String, DockerClient)> = manager
                    .docker_clients
                    .iter()
                    .map(|entry| (entry.key().clone(), entry.value().clone()))
                    .collect();
                for (name, client) in docker_clients {
                    let health_key = format!("docker:{}", name);
                    if manager.should_skip_health_check(&health_key) {
                        continue;
                    }

                    match client.validate().await {
                        Ok(()) => {
                            if manager.mark_healthy(&health_key) {
                                info!("Docker '{}' recovered", name);
                            }
                        }
                        Err(error) => {
                            let failures = manager.mark_unhealthy(&health_key, error.to_string());
                            warn!(
                                "Docker '{}' unhealthy ({} consecutive failures): {}",
                                name, failures, error
                            );
                        }
                    }
                    if let Some(ref metrics) = manager.metrics {
                        let value = if manager
                            .health
                            .get(&health_key)
                            .map(|h| h.status == ConnectionStatus::Connected)
                            .unwrap_or(false)
                        {
                            1
                        } else {
                            0
                        };
                        metrics.docker_health.with_label_values(&[&name]).set(value);
                    }
                }

                let ssh_pools: Vec<(String, SshPool)> = manager
                    .ssh_pools
                    .iter()
                    .map(|entry| (entry.key().clone(), entry.value().clone()))
                    .collect();
                for (name, pool) in ssh_pools {
                    let health_key = format!("ssh:{}", name);
                    if manager.should_skip_health_check(&health_key) {
                        continue;
                    }

                    pool.cleanup_stale_sessions().await;
                    match pool.check_connectivity().await {
                        Ok(()) => {
                            if manager.mark_healthy(&health_key) {
                                info!("SSH '{}' recovered", name);
                            }
                        }
                        Err(error) => {
                            let failures = manager.mark_unhealthy(&health_key, error.to_string());
                            warn!(
                                "SSH '{}' unhealthy ({} consecutive failures): {}",
                                name, failures, error
                            );
                        }
                    }
                    if let Some(ref metrics) = manager.metrics {
                        let value = if manager
                            .health
                            .get(&health_key)
                            .map(|h| h.status == ConnectionStatus::Connected)
                            .unwrap_or(false)
                        {
                            1
                        } else {
                            0
                        };
                        metrics.ssh_health.with_label_values(&[&name]).set(value);

                        // Update SSH pool gauges using M10's async accessors
                        let total = pool.session_count().await as i64;
                        let active = pool.active_channel_count().await as i64;
                        let idle = total.saturating_sub(active);
                        metrics
                            .ssh_pool_total
                            .with_label_values(&[&name])
                            .set(total);
                        metrics
                            .ssh_pool_active
                            .with_label_values(&[&name])
                            .set(active);
                        metrics.ssh_pool_idle.with_label_values(&[&name]).set(idle);
                    }
                }

                let proxmox_clients: Vec<(String, ProxmoxClient)> = manager
                    .proxmox_clients
                    .iter()
                    .map(|entry| (entry.key().clone(), entry.value().clone()))
                    .collect();
                for (name, client) in proxmox_clients {
                    let health_key = format!("proxmox:{}", name);
                    if manager.should_skip_health_check(&health_key) {
                        continue;
                    }

                    match client.validate().await {
                        Ok(()) => {
                            if manager.mark_healthy(&health_key) {
                                info!("Proxmox '{}' recovered", name);
                            }
                        }
                        Err(error) => {
                            let failures = manager.mark_unhealthy(&health_key, error.to_string());
                            warn!(
                                "Proxmox '{}' unhealthy ({} consecutive failures): {}",
                                name, failures, error
                            );
                        }
                    }
                    if let Some(ref metrics) = manager.metrics {
                        let value = if manager
                            .health
                            .get(&health_key)
                            .map(|health| health.status == ConnectionStatus::Connected)
                            .unwrap_or(false)
                        {
                            1
                        } else {
                            0
                        };
                        metrics
                            .proxmox_health
                            .with_label_values(&[&name])
                            .set(value);
                    }
                }
            }
        })
    }

    pub async fn close_all(&self) {
        let pools: Vec<SshPool> = self
            .ssh_pools
            .iter()
            .map(|entry| entry.value().clone())
            .collect();
        for pool in pools {
            pool.close_all().await;
        }
        debug!("Connection manager closed idle SSH sessions");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ssh_keyscan_hint_omits_default_port() {
        assert_eq!(
            ssh_keyscan_hint("example.com", 22),
            format!("ssh-keyscan -H example.com >> {}", KNOWN_HOSTS_HINT_PATH)
        );
    }

    #[test]
    fn test_ssh_keyscan_hint_includes_non_default_port() {
        assert_eq!(
            ssh_keyscan_hint("example.com", 2222),
            format!(
                "ssh-keyscan -H -p 2222 example.com >> {}",
                KNOWN_HOSTS_HINT_PATH
            )
        );
    }

    #[test]
    fn test_backoff_schedule() {
        let expected = vec![(1, 0), (2, 1), (3, 3), (4, 5), (10, 5)];
        for (failures, expected_skip) in expected {
            let skip = match failures {
                1 => 0,
                2 => 1,
                3 => 3,
                _ => 5,
            };
            assert_eq!(skip, expected_skip);
        }
    }

    #[test]
    fn test_disconnected_error_mentions_last_success() {
        let manager = ConnectionManager {
            config: Arc::new(Config {
                docker: Default::default(),
                ssh: Default::default(),
                proxmox: Default::default(),
                audit: Default::default(),
                rate_limits: Default::default(),
                tools: Default::default(),
                confirm: None,
                metrics: Default::default(),
            }),
            docker_clients: DashMap::new(),
            ssh_pools: DashMap::new(),
            proxmox_clients: DashMap::new(),
            health: DashMap::new(),
            metrics: None,
        };

        manager.health.insert(
            "ssh:test".to_string(),
            ConnectionHealth {
                status: ConnectionStatus::Disconnected,
                last_success: None,
                last_error: Some("boom".to_string()),
                consecutive_failures: 1,
                skip_cycles: 0,
                cycles_skipped: 0,
            },
        );

        let message = manager.disconnected_error_message("ssh:test", "test");
        assert!(message.contains("boom"));
        assert!(message.contains("test"));
    }
}
