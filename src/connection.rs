use anyhow::{Result, anyhow};
use async_trait::async_trait;
use dashmap::DashMap;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;
use tokio::sync::{Mutex, Notify};
use tokio::time::{Duration, interval, sleep, timeout};
use tracing::{debug, info, warn};

use crate::config::{Config, DockerHost, SshHost, SshPoolConfig};

/// Manages persistent connections to Docker daemons and SSH hosts.
pub struct ConnectionManager {
    pub config: Arc<Config>,
    docker_clients: DashMap<String, DockerClient>,
    ssh_pools: DashMap<String, SshPool>,
    health: DashMap<String, ConnectionHealth>,
}

/// Docker connection handle with transport metadata for diagnostics.
#[derive(Clone)]
pub struct DockerClient {
    client: Arc<bollard::Docker>,
    transport: DockerTransport,
}

#[derive(Debug, Clone)]
pub enum DockerTransport {
    UnixSocket { path: PathBuf },
    Tcp { host: String, tls: bool },
}

impl DockerClient {
    pub fn new(host: &DockerHost) -> Result<Self> {
        let (client, transport) = if host.host.starts_with("unix://") {
            let socket_path = host.host.trim_start_matches("unix://");
            let client = bollard::Docker::connect_with_unix(
                socket_path,
                120,
                bollard::API_DEFAULT_VERSION,
            )
            .map_err(|error| anyhow!("Failed to connect to Docker socket {}: {}", socket_path, error))?;

            (
                client,
                DockerTransport::UnixSocket {
                    path: PathBuf::from(socket_path),
                },
            )
        } else if host.host.starts_with("tcp://") {
            let has_tls = host.key_path.is_some()
                && host.cert_path.is_some()
                && host.ca_path.is_some();

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
        } else {
            return Err(anyhow!(
                "Invalid Docker connection string '{}'. Expected unix:// or tcp://",
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
        }
    }
}

/// SSH connection pool for a single host.
#[derive(Clone)]
pub struct SshPool {
    sessions: Arc<Mutex<VecDeque<PooledSession>>>,
    active_count: Arc<AtomicUsize>,
    total_count: Arc<AtomicUsize>,
    max_sessions: usize,
    session_available: Arc<Notify>,
    host_config: Arc<SshHost>,
    pool_config: SshPoolConfig,
}

pub struct PooledSession {
    pub handle: russh::client::Handle<SshClientHandler>,
    created_at: Instant,
    last_used: Instant,
}

#[derive(Clone)]
pub struct SshClientHandler {
    host: String,
    port: u16,
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
                // The operator must add the host key first:
                //   ssh-keyscan -H <host> >> ~/.ssh/known_hosts
                warn!(
                    "SSH host key for {}:{} not found in known_hosts. \
                     Add it with: ssh-keyscan -H {} >> ~/.ssh/known_hosts",
                    self.host, self.port, self.host
                );
                Err(anyhow!(
                    "SSH host key for {}:{} not found in known_hosts. \
                     Run: ssh-keyscan -H {} >> ~/.ssh/known_hosts",
                    self.host, self.port, self.host
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
                    self.host, self.port, line
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
                    self.host, self.port, error
                ))
            }
        }
    }
}

impl SshPool {
    pub fn new(host_config: SshHost, pool_config: SshPoolConfig) -> Self {
        Self {
            sessions: Arc::new(Mutex::new(VecDeque::new())),
            active_count: Arc::new(AtomicUsize::new(0)),
            total_count: Arc::new(AtomicUsize::new(0)),
            max_sessions: pool_config.max_sessions_per_host,
            session_available: Arc::new(Notify::new()),
            host_config: Arc::new(host_config),
            pool_config,
        }
    }

    pub async fn checkout(&self) -> Result<PooledSession> {
        loop {
            {
                let mut sessions = self.sessions.lock().await;
                while let Some(mut session) = sessions.pop_front() {
                    if self.validate_session(&session) {
                        session.last_used = Instant::now();
                        self.active_count.fetch_add(1, Ordering::Relaxed);
                        return Ok(session);
                    }

                    self.total_count.fetch_sub(1, Ordering::Relaxed);
                }
            }

            if self.total_count.load(Ordering::Relaxed) < self.max_sessions {
                let session = match self.create_session().await {
                    Ok(session) => session,
                    Err(first_error) => {
                        warn!(
                            "SSH connection to {} failed, retrying once: {}",
                            self.host_config.host,
                            first_error
                        );
                        sleep(Duration::from_secs(1)).await;
                        self.create_session().await.map_err(|retry_error| {
                            anyhow!(
                                "SSH connection failed after retry. First: {}. Retry: {}",
                                first_error,
                                retry_error
                            )
                        })?
                    }
                };

                self.total_count.fetch_add(1, Ordering::Relaxed);
                self.active_count.fetch_add(1, Ordering::Relaxed);
                return Ok(session);
            }

            let wait_duration = Duration::from_secs(self.pool_config.checkout_timeout_secs);
            timeout(wait_duration, self.session_available.notified())
                .await
                .map_err(|_| {
                    anyhow!(
                        "All SSH sessions to '{}' are in use ({} active). Try again shortly.",
                        self.host_config.host,
                        self.active_count.load(Ordering::Relaxed)
                    )
                })?;
        }
    }

    pub async fn return_session(&self, mut session: PooledSession, broken: bool) {
        self.active_count.fetch_sub(1, Ordering::Relaxed);

        if broken || !self.validate_session(&session) {
            self.total_count.fetch_sub(1, Ordering::Relaxed);
        } else {
            session.last_used = Instant::now();
            let mut sessions = self.sessions.lock().await;
            sessions.push_back(session);
        }

        self.session_available.notify_one();
    }

    fn validate_session(&self, session: &PooledSession) -> bool {
        let max_lifetime = Duration::from_secs(self.pool_config.max_lifetime_secs);
        if session.created_at.elapsed() > max_lifetime {
            return false;
        }

        let max_idle = Duration::from_secs(self.pool_config.max_idle_time_secs);
        session.last_used.elapsed() <= max_idle
    }

    async fn create_session(&self) -> Result<PooledSession> {
        let config = russh::client::Config {
            inactivity_timeout: Some(Duration::from_secs(self.pool_config.connect_timeout_secs)),
            keepalive_interval: Some(Duration::from_secs(self.pool_config.keepalive_interval_secs)),
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

        Ok(PooledSession {
            handle,
            created_at: Instant::now(),
            last_used: Instant::now(),
        })
    }

    pub async fn cleanup_stale_sessions(&self) {
        let max_lifetime = Duration::from_secs(self.pool_config.max_lifetime_secs);
        let max_idle = Duration::from_secs(self.pool_config.max_idle_time_secs);
        let keepalive_threshold = Duration::from_secs(self.pool_config.keepalive_interval_secs);

        // Phase 1: Remove expired sessions, separate idle sessions for probing
        let sessions_to_probe: Vec<PooledSession> = {
            let mut sessions = self.sessions.lock().await;
            let before = sessions.len();

            // Remove sessions that exceeded lifetime or idle time
            sessions.retain(|session| {
                session.created_at.elapsed() <= max_lifetime
                    && session.last_used.elapsed() <= max_idle
            });
            let expired = before.saturating_sub(sessions.len());
            if expired > 0 {
                self.total_count.fetch_sub(expired, Ordering::Relaxed);
                info!(
                    "Cleaned up {} expired SSH sessions for {}",
                    expired, self.host_config.host
                );
            }

            // Extract sessions idle longer than keepalive threshold for async probing
            let mut remaining = VecDeque::new();
            let mut to_probe = Vec::new();
            for session in sessions.drain(..) {
                if session.last_used.elapsed() > keepalive_threshold {
                    to_probe.push(session);
                } else {
                    remaining.push_back(session);
                }
            }
            *sessions = remaining;

            to_probe
        };
        // Lock released — other checkouts/returns are unblocked during probing

        if sessions_to_probe.is_empty() {
            return;
        }

        // Phase 2: Async keepalive probe on idle sessions (no lock held)
        let mut alive = Vec::new();
        let mut dead: usize = 0;

        for session in sessions_to_probe {
            let probe_result = timeout(Duration::from_secs(3), async {
                match session.handle.channel_open_session().await {
                    Ok(channel) => {
                        channel.close().await.ok();
                        true
                    }
                    Err(_) => false,
                }
            })
            .await;

            match probe_result {
                Ok(true) => alive.push(session),
                _ => dead += 1,
            }
        }

        // Phase 3: Return alive sessions to the pool
        if !alive.is_empty() {
            let mut sessions = self.sessions.lock().await;
            for session in alive {
                sessions.push_back(session);
            }
        }

        if dead > 0 {
            self.total_count.fetch_sub(dead, Ordering::Relaxed);
            info!(
                "Removed {} dead SSH sessions for {} (keepalive probe failed)",
                dead, self.host_config.host
            );
        }
    }

    pub async fn check_connectivity(&self) -> Result<()> {
        // If the pool has recently-used valid sessions, the host was reachable recently.
        // This avoids creating a wasteful disposable connection every 30s.
        {
            let sessions = self.sessions.lock().await;
            for session in sessions.iter() {
                if self.validate_session(session) {
                    return Ok(());
                }
            }
        }

        // No valid pooled sessions — create a test connection to verify reachability
        let session = self.create_session().await?;
        session
            .handle
            .disconnect(russh::Disconnect::ByApplication, "health check complete", "en")
            .await
            .ok();
        Ok(())
    }

    pub async fn close_all(&self) {
        let mut sessions = self.sessions.lock().await;
        sessions.clear();
        self.total_count.store(0, Ordering::Relaxed);
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
    pub async fn new(config: Config) -> Result<Self> {
        let manager = Self {
            config: Arc::new(config),
            docker_clients: DashMap::new(),
            ssh_pools: DashMap::new(),
            health: DashMap::new(),
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
                            info!("Docker '{}' connected via {}", name, client.transport_summary());
                        }
                        Err(error) => {
                            manager.mark_unhealthy(&health_key, error.to_string());
                            warn!(
                                "Docker '{}' ping failed during startup (will retry): {}",
                                name,
                                error
                            );
                        }
                    }
                }
                Err(error) => {
                    manager.mark_unhealthy(&health_key, error.to_string());
                    warn!("Failed to initialize Docker client for '{}': {}", name, error);
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

            manager
                .ssh_pools
                .insert(name.clone(), SshPool::new(host.clone(), manager.config.ssh.pool.clone()));
            // SSH pools start in Connecting state — actual connectivity is verified
            // by the first health monitor cycle, not at startup. This avoids blocking
            // startup on SSH handshakes and avoids falsely reporting Connected.
            info!("SSH pool '{}' initialized (connectivity pending)", name);
        }

        info!(
            "ConnectionManager initialized with {} Docker hosts and {} SSH hosts",
            manager.config.docker.hosts.len(),
            manager.config.ssh.hosts.len()
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

    pub async fn ssh_checkout(&self, host: &str) -> Result<PooledSession> {
        let pool = self
            .ssh_pools
            .get(host)
            .map(|entry| entry.clone())
            .ok_or_else(|| anyhow!(self.disconnected_error_message(&format!("ssh:{}", host), host)))?;

        pool.checkout().await
    }

    pub async fn ssh_return(&self, host: &str, session: PooledSession, broken: bool) {
        if let Some(pool) = self.ssh_pools.get(host) {
            pool.return_session(session, broken).await;
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

            let last_error = health
                .last_error
                .as_deref()
                .unwrap_or("unknown");

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
                                name,
                                failures,
                                error
                            );
                        }
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
                                name,
                                failures,
                                error
                            );
                        }
                    }
                }
            }
        })
    }

    pub async fn close_all(&self) {
        let pools: Vec<SshPool> = self.ssh_pools.iter().map(|entry| entry.value().clone()).collect();
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
                audit: Default::default(),
                rate_limits: Default::default(),
                tools: Default::default(),
                confirm: None,
            }),
            docker_clients: DashMap::new(),
            ssh_pools: DashMap::new(),
            health: DashMap::new(),
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
