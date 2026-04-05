use prometheus::{
    Encoder, HistogramOpts, HistogramVec, IntCounterVec, IntGaugeVec, Opts, Registry, TextEncoder,
};
use std::sync::Arc;

/// Central metrics registry for the MCP server.
/// All fields are registered with the Prometheus registry and appear in `/metrics`
/// output even if not yet actively updated from Rust code.
#[derive(Clone)]
#[allow(dead_code)]
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
        let registry =
            Registry::new_custom(Some("homelab".to_string()), None).expect("metrics registry");

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
            Opts::new(
                "ssh_pool_active_sessions",
                "Active (checked out) SSH sessions",
            ),
            &["host"],
        )
        .expect("ssh_pool_active metric");

        let ssh_pool_idle = IntGaugeVec::new(
            Opts::new("ssh_pool_idle_sessions", "Idle (available) SSH sessions"),
            &["host"],
        )
        .expect("ssh_pool_idle metric");

        let ssh_pool_total = IntGaugeVec::new(
            Opts::new(
                "ssh_pool_total_sessions",
                "Total SSH sessions (active + idle)",
            ),
            &["host"],
        )
        .expect("ssh_pool_total metric");

        let docker_health = IntGaugeVec::new(
            Opts::new(
                "docker_connection_healthy",
                "Docker connection health (1=up, 0=down)",
            ),
            &["host"],
        )
        .expect("docker_health metric");

        let ssh_health = IntGaugeVec::new(
            Opts::new(
                "ssh_connection_healthy",
                "SSH connection health (1=up, 0=down)",
            ),
            &["host"],
        )
        .expect("ssh_health metric");

        let confirmation_tokens_issued = IntCounterVec::new(
            Opts::new(
                "confirmation_tokens_issued_total",
                "Confirmation tokens issued",
            ),
            &["tool"],
        )
        .expect("confirmation_tokens_issued metric");

        let confirmation_tokens_resolved = IntCounterVec::new(
            Opts::new(
                "confirmation_tokens_resolved_total",
                "Confirmation tokens resolved",
            ),
            &["outcome"],
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
#[cfg(feature = "metrics")]
pub fn spawn_metrics_server(listen: &str, metrics: Arc<Metrics>) -> tokio::task::JoinHandle<()> {
    use axum::{Router, extract::State, routing::get};

    let app = Router::new()
        .route(
            "/metrics",
            get(|State(metrics): State<Arc<Metrics>>| async move { metrics.encode() }),
        )
        .with_state(metrics);

    let listener_addr: std::net::SocketAddr =
        listen.parse().expect("invalid metrics listen address");

    tokio::spawn(async move {
        let listener = tokio::net::TcpListener::bind(listener_addr)
            .await
            .expect("bind metrics server");
        tracing::info!(
            "Metrics server listening on http://{}/metrics",
            listener_addr
        );
        axum::serve(listener, app).await.ok();
    })
}
