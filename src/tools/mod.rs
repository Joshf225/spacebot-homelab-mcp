/// Docker container tools
pub mod docker;
/// SSH tools
pub mod ssh;

pub fn wrap_output_envelope(tool_name: &str, content: &str) -> String {
    serde_json::json!({
        "type": "tool_result",
        "source": tool_name,
        "data_classification": "untrusted_external",
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
