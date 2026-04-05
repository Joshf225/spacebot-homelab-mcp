//! Desktop notification support for the homelab MCP server.
//!
//! Fires on two events during startup:
//! - **Connected** — MCP server is up and tools are ready.
//! - **Failed**    — A fatal error occurred before the server could start.
//!
//! Cross-process deduplication: a Unix timestamp written to
//! `/tmp/spacebot-homelab-mcp-notify` prevents notification spam when the
//! process is restarted rapidly (within a 60-second window).
//!
//! Compile with `--no-default-features` to strip `notify-rust` entirely; all
//! public functions become silent no-ops with no runtime overhead.

// ── Feature-gated implementation ─────────────────────────────────────────────

#[cfg(feature = "notifications")]
mod desktop {
    use std::fs;
    use std::path::Path;
    use std::time::{SystemTime, UNIX_EPOCH};

    const THROTTLE_FILE: &str = "/tmp/spacebot-homelab-mcp-notify";
    const THROTTLE_SECS: u64 = 60;

    // ── Types ─────────────────────────────────────────────────────────────────

    pub enum Severity {
        Success,
        Error,
    }

    pub struct Notification {
        pub title: String,
        pub body: String,
        pub severity: Severity,
        pub sound: bool,
    }

    pub trait NotificationDispatcher: Send + Sync {
        fn dispatch(&self, notification: &Notification) -> anyhow::Result<()>;
    }

    // ── DesktopDispatcher ─────────────────────────────────────────────────────

    /// Sends OS-native desktop notifications via `notify-rust`.
    pub struct DesktopDispatcher;

    impl NotificationDispatcher for DesktopDispatcher {
        fn dispatch(&self, n: &Notification) -> anyhow::Result<()> {
            let mut notif = notify_rust::Notification::new();
            notif.summary(&n.title);
            notif.body(&n.body);

            if n.sound {
                // macOS: use NSSound names ("Glass" = success, "Basso" = error)
                #[cfg(target_os = "macos")]
                {
                    let sound_name = match n.severity {
                        Severity::Success => "Glass",
                        Severity::Error => "Basso",
                    };
                    notif.sound_name(sound_name);
                }

                // Linux: use freedesktop.org sound names + urgency on error
                #[cfg(target_os = "linux")]
                {
                    let sound_name = match n.severity {
                        Severity::Success => "message-new-instant",
                        Severity::Error => "dialog-warning",
                    };
                    notif.sound_name(sound_name);
                    if matches!(n.severity, Severity::Error) {
                        notif.urgency(notify_rust::Urgency::Critical);
                    }
                }
            }

            notif.show().map_err(|e| anyhow::anyhow!("{}", e))?;
            Ok(())
        }
    }

    // ── ThrottleGuard ─────────────────────────────────────────────────────────

    /// Manages the cross-process deduplication temp file.
    ///
    /// All methods are best-effort: if the temp file can't be read or written,
    /// the notification fires anyway.
    pub struct ThrottleGuard;

    impl ThrottleGuard {
        /// Returns `true` if a failure notification should be sent now.
        /// Returns `false` if one was already sent within the last 60 seconds.
        pub fn should_notify_failure() -> bool {
            let path = Path::new(THROTTLE_FILE);
            if !path.exists() {
                return true;
            }
            match fs::read_to_string(path) {
                Ok(content) => {
                    if let Ok(ts) = content.trim().parse::<u64>() {
                        let now = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .map(|d| d.as_secs())
                            .unwrap_or(0);
                        return now.saturating_sub(ts) >= THROTTLE_SECS;
                    }
                    true
                }
                Err(_) => true,
            }
        }

        /// Writes the current timestamp to the throttle file.
        pub fn record_failure() {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let _ = fs::write(THROTTLE_FILE, now.to_string());
        }

        /// Removes the throttle file so the next failure always notifies.
        /// Called on success to reset the window.
        pub fn clear() {
            let _ = fs::remove_file(THROTTLE_FILE);
        }
    }

    // ── Public helpers ────────────────────────────────────────────────────────

    /// Returns `true` if notifications are suppressed via the
    /// `SPACEBOT_HOMELAB_NO_NOTIFY` environment variable (set to any
    /// non-empty value). Used to silence popups during `cargo test`.
    fn is_suppressed() -> bool {
        std::env::var("SPACEBOT_HOMELAB_NO_NOTIFY")
            .map(|v| !v.is_empty())
            .unwrap_or(false)
    }

    /// Send a "Connected" desktop notification.
    ///
    /// Always fires regardless of throttle state; clears the failure throttle
    /// file so a subsequent failure in the same session will always notify.
    ///
    /// Suppressed when the `SPACEBOT_HOMELAB_NO_NOTIFY` env var is set.
    pub fn notify_connected(tool_count: usize, tool_summary: &str) {
        if is_suppressed() {
            tracing::debug!("Desktop notifications suppressed (SPACEBOT_HOMELAB_NO_NOTIFY)");
            return;
        }
        ThrottleGuard::clear();
        let n = Notification {
            title: "Homelab MCP".to_string(),
            body: format!("Connected — {tool_count} tools available\n{tool_summary}"),
            severity: Severity::Success,
            sound: true,
        };
        if let Err(e) = DesktopDispatcher.dispatch(&n) {
            tracing::warn!("Failed to send desktop notification: {}", e);
        }
    }

    /// Send a "Connection failed" desktop notification.
    ///
    /// Throttled to at most one notification per 60-second window to prevent
    /// spam when Spacebot's retry logic re-spawns the process rapidly.
    ///
    /// Suppressed when the `SPACEBOT_HOMELAB_NO_NOTIFY` env var is set.
    pub fn notify_failed(error: &str) {
        if is_suppressed() {
            tracing::debug!("Desktop notifications suppressed (SPACEBOT_HOMELAB_NO_NOTIFY)");
            return;
        }
        if !ThrottleGuard::should_notify_failure() {
            tracing::debug!("Failure notification suppressed (within 60 s throttle window)");
            return;
        }
        ThrottleGuard::record_failure();
        let n = Notification {
            title: "Homelab MCP".to_string(),
            body: format!("Connection failed: {error}"),
            severity: Severity::Error,
            sound: true,
        };
        if let Err(e) = DesktopDispatcher.dispatch(&n) {
            tracing::warn!("Failed to send desktop notification: {}", e);
        }
    }
}

// Re-export everything from the inner module when the feature is on.
// The trait / struct types are public API for future dispatcher implementations
// (e.g. McpProtocolDispatcher) even though the binary itself only calls the
// two helper functions directly.
#[cfg(feature = "notifications")]
#[allow(unused_imports)]
pub use desktop::{
    DesktopDispatcher, Notification, NotificationDispatcher, Severity, ThrottleGuard,
    notify_connected, notify_failed,
};

// ── No-op stubs when `notifications` feature is disabled ─────────────────────

/// No-op: feature `notifications` is disabled.
#[cfg(not(feature = "notifications"))]
#[allow(unused_variables)]
pub fn notify_connected(_tool_count: usize, _tool_summary: &str) {}

/// No-op: feature `notifications` is disabled.
#[cfg(not(feature = "notifications"))]
#[allow(unused_variables)]
pub fn notify_failed(_error: &str) {}
