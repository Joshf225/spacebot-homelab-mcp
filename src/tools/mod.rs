/// Docker container tools
pub mod docker;
/// Docker image tools
pub mod docker_image;
/// SSH tools
pub mod ssh;
/// Audit verification tool
pub mod verify;

/// Build a tamper-evident output envelope.
///
/// Every real tool result includes a `server_nonce` (8 random hex bytes) and
/// `executed_at` ISO-8601 timestamp that an LLM cannot predict ahead of time.
/// A downstream verification step (or human reviewer) can check these fields
/// to distinguish genuine server responses from hallucinated ones.
pub fn wrap_output_envelope(tool_name: &str, content: &str) -> String {
    use chrono::Utc;
    use uuid::Uuid;

    // 8-byte random hex nonce — compact but unguessable
    let nonce = format!("{:016x}", Uuid::new_v4().as_u128() & 0xffff_ffff_ffff_ffff);

    serde_json::json!({
        "type": "tool_result",
        "source": tool_name,
        "data_classification": "untrusted_external",
        "server_nonce": nonce,
        "executed_at": Utc::now().to_rfc3339(),
        "server_version": env!("CARGO_PKG_VERSION"),
        "content": content
    })
    .to_string()
}

pub fn truncate_output(output: &str, max_chars: usize) -> String {
    let char_count = output.chars().count();
    if char_count <= max_chars {
        return output.to_string();
    }

    let truncated: String = output.chars().take(max_chars).collect();
    format!(
        "[Output truncated. Showing {} chars of {} total chars.]\n{}",
        max_chars, char_count, truncated
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wrap_output_envelope_json_format() {
        let result = wrap_output_envelope("ssh.exec", "hello world");
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["type"], "tool_result");
        assert_eq!(parsed["source"], "ssh.exec");
        assert_eq!(parsed["data_classification"], "untrusted_external");
        assert_eq!(parsed["content"], "hello world");

        // Execution proof fields must be present
        let nonce = parsed["server_nonce"].as_str().unwrap();
        assert_eq!(nonce.len(), 16, "nonce should be 16 hex chars");
        assert!(
            nonce.chars().all(|c| c.is_ascii_hexdigit()),
            "nonce should be hex"
        );

        let executed_at = parsed["executed_at"].as_str().unwrap();
        assert!(
            executed_at.contains('T'),
            "executed_at should be ISO-8601"
        );

        let version = parsed["server_version"].as_str().unwrap();
        assert!(
            !version.is_empty(),
            "server_version should be non-empty"
        );
    }

    #[test]
    fn test_wrap_output_envelope_unique_nonces() {
        let r1 = wrap_output_envelope("test", "a");
        let r2 = wrap_output_envelope("test", "b");
        let p1: serde_json::Value = serde_json::from_str(&r1).unwrap();
        let p2: serde_json::Value = serde_json::from_str(&r2).unwrap();
        assert_ne!(
            p1["server_nonce"], p2["server_nonce"],
            "each envelope should have a unique nonce"
        );
    }

    #[test]
    fn test_truncate_output_short() {
        let short = "hello";
        let result = truncate_output(short, 100);
        assert_eq!(result, "hello");
    }

    #[test]
    fn test_truncate_output_long() {
        let long = "a".repeat(200);
        let result = truncate_output(&long, 50);
        assert!(result.contains("[Output truncated. Showing 50 chars of 200 total chars.]"));
        // The truncated content after the prefix message should be 50 'a's
        assert!(result.ends_with(&"a".repeat(50)));
    }
}
