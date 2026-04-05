use anyhow::{Result, anyhow};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use uuid::Uuid;

struct PendingConfirmation {
    tool_name: String,
    params_json: String,
    created_at: Instant,
    used: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ConfirmRule {
    Always(String),
    WhenPattern { when_pattern: Vec<String> },
}

impl ConfirmRule {
    /// Validate that the rule has a recognized value.
    /// The `Always` variant must contain exactly `"always"` — any other string
    /// (e.g. a typo like `"aways"`) is rejected so it doesn't silently fail to
    /// trigger confirmation.
    pub fn validate(&self, rule_name: &str) -> Result<()> {
        match self {
            Self::Always(value) if value != "always" => Err(anyhow!(
                "Invalid confirmation rule for '{}': expected 'always' or {{ when_pattern = [...] }}, got '{}'",
                rule_name,
                value
            )),
            _ => Ok(()),
        }
    }

    pub fn requires_confirmation(&self, command_text: Option<&str>) -> bool {
        match self {
            Self::Always(value) => value == "always",
            Self::WhenPattern { when_pattern } => command_text
                .map(|command| when_pattern.iter().any(|pattern| command.contains(pattern)))
                .unwrap_or(false),
        }
    }
}

pub struct ConfirmationManager {
    pending: Arc<Mutex<HashMap<String, PendingConfirmation>>>,
    rules: HashMap<String, ConfirmRule>,
    token_ttl: Duration,
}

impl ConfirmationManager {
    pub fn new(rules: HashMap<String, ConfirmRule>) -> Self {
        Self {
            pending: Arc::new(Mutex::new(HashMap::new())),
            rules,
            token_ttl: Duration::from_secs(300),
        }
    }

    #[cfg(test)]
    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.token_ttl = ttl;
        self
    }

    pub async fn check_and_maybe_require(
        &self,
        tool_name: &str,
        command_text: Option<&str>,
        description: &str,
        params_json: &str,
    ) -> Result<Option<String>> {
        let rule = match self.rules.get(tool_name) {
            Some(rule) => rule,
            None => return Ok(None),
        };

        if !rule.requires_confirmation(command_text) {
            return Ok(None);
        }

        let token = Uuid::new_v4().to_string();
        let pending = PendingConfirmation {
            tool_name: tool_name.to_string(),
            params_json: params_json.to_string(),
            created_at: Instant::now(),
            used: false,
        };

        let mut map = self.pending.lock().await;
        map.retain(|_, existing| existing.created_at.elapsed() < self.token_ttl && !existing.used);
        map.insert(token.clone(), pending);

        Ok(Some(
            serde_json::json!({
                "status": "confirmation_required",
                "token": token,
                "message": format!(
                    "{}. Call confirm_operation with this token and tool name '{}' to proceed.",
                    description,
                    tool_name
                ),
            })
            .to_string(),
        ))
    }

    pub async fn confirm(&self, token: &str, tool_name: &str) -> Result<String> {
        let mut map = self.pending.lock().await;
        let pending = map.get_mut(token).ok_or_else(|| {
            anyhow!("Invalid or expired confirmation token. Tokens expire after 5 minutes.")
        })?;

        if pending.created_at.elapsed() >= self.token_ttl {
            map.remove(token);
            return Err(anyhow!(
                "Confirmation token has expired (5 minute limit). Please re-initiate the operation."
            ));
        }

        if pending.used {
            return Err(anyhow!(
                "Confirmation token has already been used. Each token is single-use."
            ));
        }

        if pending.tool_name != tool_name {
            return Err(anyhow!(
                "Token was issued for '{}', not '{}'. Tokens are tool-specific.",
                pending.tool_name,
                tool_name
            ));
        }

        let params_json = pending.params_json.clone();
        pending.used = true;
        map.remove(token);
        Ok(params_json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_always_rule_requires_confirmation() {
        let mut rules = HashMap::new();
        rules.insert(
            "docker.container.delete".to_string(),
            ConfirmRule::Always("always".to_string()),
        );
        let manager = ConfirmationManager::new(rules);

        let response = manager
            .check_and_maybe_require(
                "docker.container.delete",
                None,
                "About to delete container webapp-01",
                r#"{"container_id":"webapp-01"}"#,
            )
            .await
            .unwrap()
            .unwrap();

        let parsed: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(parsed["status"], "confirmation_required");
        assert!(parsed["token"].as_str().unwrap().len() > 10);
    }

    #[tokio::test]
    async fn test_when_pattern_matches() {
        let mut rules = HashMap::new();
        rules.insert(
            "ssh.exec".to_string(),
            ConfirmRule::WhenPattern {
                when_pattern: vec!["rm -rf".to_string(), "dd if=".to_string()],
            },
        );
        let manager = ConfirmationManager::new(rules);

        let destructive = manager
            .check_and_maybe_require(
                "ssh.exec",
                Some("sudo rm -rf /tmp/old"),
                "About to run sudo rm -rf /tmp/old",
                r#"{"host":"nas","command":"sudo rm -rf /tmp/old"}"#,
            )
            .await
            .unwrap();
        assert!(destructive.is_some());

        let safe = manager
            .check_and_maybe_require(
                "ssh.exec",
                Some("df -h"),
                "About to run df -h",
                r#"{"host":"nas","command":"df -h"}"#,
            )
            .await
            .unwrap();
        assert!(safe.is_none());
    }

    #[tokio::test]
    async fn test_token_single_use() {
        let mut rules = HashMap::new();
        rules.insert(
            "ssh.exec".to_string(),
            ConfirmRule::Always("always".to_string()),
        );
        let manager = ConfirmationManager::new(rules);

        let response = manager
            .check_and_maybe_require(
                "ssh.exec",
                Some("sudo rm -rf /tmp/old"),
                "About to run sudo rm -rf /tmp/old",
                r#"{"host":"nas","command":"sudo rm -rf /tmp/old"}"#,
            )
            .await
            .unwrap()
            .unwrap();

        let parsed: serde_json::Value = serde_json::from_str(&response).unwrap();
        let token = parsed["token"].as_str().unwrap();

        assert!(manager.confirm(token, "ssh.exec").await.is_ok());
        assert!(manager.confirm(token, "ssh.exec").await.is_err());
    }

    #[tokio::test]
    async fn test_wrong_tool_rejected() {
        let mut rules = HashMap::new();
        rules.insert(
            "ssh.exec".to_string(),
            ConfirmRule::Always("always".to_string()),
        );
        let manager = ConfirmationManager::new(rules);

        let response = manager
            .check_and_maybe_require(
                "ssh.exec",
                Some("sudo rm -rf /tmp/old"),
                "About to run sudo rm -rf /tmp/old",
                r#"{"host":"nas","command":"sudo rm -rf /tmp/old"}"#,
            )
            .await
            .unwrap()
            .unwrap();

        let parsed: serde_json::Value = serde_json::from_str(&response).unwrap();
        let token = parsed["token"].as_str().unwrap();

        assert!(
            manager
                .confirm(token, "docker.container.delete")
                .await
                .is_err()
        );
    }

    #[test]
    fn test_validate_rejects_typo() {
        let rule = ConfirmRule::Always("aways".to_string());
        let result = rule.validate("ssh.exec");
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("ssh.exec"),
            "error should mention the rule name"
        );
        assert!(msg.contains("aways"), "error should mention the bad value");
    }

    #[test]
    fn test_validate_accepts_always() {
        let rule = ConfirmRule::Always("always".to_string());
        assert!(rule.validate("ssh.exec").is_ok());
    }

    #[test]
    fn test_validate_accepts_when_pattern() {
        let rule = ConfirmRule::WhenPattern {
            when_pattern: vec!["rm -rf".to_string()],
        };
        assert!(rule.validate("ssh.exec").is_ok());
    }

    #[tokio::test]
    async fn test_token_expires_after_ttl() {
        let mut rules = HashMap::new();
        rules.insert(
            "ssh.exec".to_string(),
            ConfirmRule::Always("always".to_string()),
        );
        let manager = ConfirmationManager::new(rules).with_ttl(Duration::from_millis(1));

        let response = manager
            .check_and_maybe_require(
                "ssh.exec",
                Some("rm -rf /tmp/old"),
                "About to run rm -rf /tmp/old",
                r#"{"host":"nas","command":"rm -rf /tmp/old"}"#,
            )
            .await
            .unwrap()
            .unwrap();

        let parsed: serde_json::Value = serde_json::from_str(&response).unwrap();
        let token = parsed["token"].as_str().unwrap();

        // Sleep long enough for the 1ms TTL to expire
        tokio::time::sleep(Duration::from_millis(50)).await;

        let result = manager.confirm(token, "ssh.exec").await;
        assert!(result.is_err(), "token should have expired");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("expired") || msg.contains("Invalid"),
            "error should mention expiration, got: {}",
            msg
        );
    }
}
