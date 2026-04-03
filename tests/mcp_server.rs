use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use rmcp::model::ClientInfo;
use serde_json::{Value, json};
use tempfile::NamedTempFile;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdout, Command};
use tokio::time::timeout;

fn binary_path() -> &'static str {
    env!("CARGO_BIN_EXE_spacebot-homelab-mcp")
}

fn write_config(contents: &str) -> NamedTempFile {
    let file = NamedTempFile::new().expect("create temp config");
    std::fs::write(file.path(), contents).expect("write temp config");
    file
}

fn config_path(path: &Path) -> String {
    path.to_str().expect("utf-8 config path").to_string()
}

async fn spawn_server(config: &Path) -> Child {
    let mut command = Command::new(binary_path());
    command
        .args(["server", "--config", &config_path(config)])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    command.spawn().expect("spawn MCP server")
}

async fn read_json_line(lines: &mut Lines<BufReader<ChildStdout>>) -> Value {
    let next_line = timeout(Duration::from_secs(5), lines.next_line())
        .await
        .expect("timed out waiting for server output")
        .expect("read line from server")
        .expect("server produced a JSON-RPC line");

    serde_json::from_str(&next_line).expect("parse server JSON-RPC message")
}

#[tokio::test]
async fn test_mcp_server_starts_and_lists_tools() {
    let config = write_config("");
    let mut child = spawn_server(config.path()).await;

    let mut stdin = child.stdin.take().expect("server stdin");
    let stdout = child.stdout.take().expect("server stdout");
    let mut lines = BufReader::new(stdout).lines();

    let initialize_request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": ClientInfo::default(),
    });
    stdin
        .write_all(format!("{}\n", initialize_request).as_bytes())
        .await
        .expect("write initialize request");
    stdin.flush().await.expect("flush initialize request");

    let initialize_response = read_json_line(&mut lines).await;
    assert_eq!(initialize_response["id"], json!(1));
    assert_eq!(initialize_response["result"]["serverInfo"]["name"], json!("spacebot-homelab-mcp"));
    assert!(initialize_response["result"]["capabilities"]["tools"].is_object());

    let initialized_notification = json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized",
    });
    stdin
        .write_all(format!("{}\n", initialized_notification).as_bytes())
        .await
        .expect("write initialized notification");

    let list_tools_request = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list",
    });
    stdin
        .write_all(format!("{}\n", list_tools_request).as_bytes())
        .await
        .expect("write tools/list request");
    stdin.flush().await.expect("flush tools/list request");

    let tools_response = read_json_line(&mut lines).await;
    assert_eq!(tools_response["id"], json!(2));
    let tool_names = tools_response["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect::<Vec<_>>();

    for expected_tool in [
        "docker.container.list",
        "docker.container.start",
        "docker.container.stop",
        "docker.container.logs",
        "docker.container.inspect",
        "ssh.exec",
        "ssh.upload",
        "ssh.download",
        "confirm_operation",
    ] {
        assert!(
            tool_names.contains(&expected_tool),
            "missing tool '{}', got {:?}",
            expected_tool,
            tool_names
        );
    }

    drop(stdin);

    let status = timeout(Duration::from_secs(5), child.wait())
        .await
        .expect("timed out waiting for server exit")
        .expect("wait for server exit");
    assert!(status.success(), "server exited unsuccessfully: {status}");
}

#[tokio::test]
async fn test_ssh_exec_confirmation_flow() {
    let config = write_config(
        r#"
[confirm]
"ssh.exec" = { when_pattern = ["systemctl restart"] }
"#,
    );
    let mut child = spawn_server(config.path()).await;

    let mut stdin = child.stdin.take().expect("server stdin");
    let stdout = child.stdout.take().expect("server stdout");
    let mut lines = BufReader::new(stdout).lines();

    let initialize_request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": ClientInfo::default(),
    });
    stdin
        .write_all(format!("{}\n", initialize_request).as_bytes())
        .await
        .expect("write initialize request");
    stdin.flush().await.expect("flush initialize request");
    let _ = read_json_line(&mut lines).await;

    let initialized_notification = json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized",
    });
    stdin
        .write_all(format!("{}\n", initialized_notification).as_bytes())
        .await
        .expect("write initialized notification");

    let tool_call = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "ssh.exec",
            "arguments": {
                "host": "nas",
                "command": "sudo systemctl restart nginx"
            }
        }
    });
    stdin
        .write_all(format!("{}\n", tool_call).as_bytes())
        .await
        .expect("write tools/call request");
    stdin.flush().await.expect("flush tools/call request");

    let call_response = read_json_line(&mut lines).await;
    assert_eq!(call_response["id"], json!(2));
    let content_text = call_response["result"]["content"]
        .as_array()
        .and_then(|items| items.first())
        .and_then(|item| item["text"].as_str())
        .expect("tool response text");
    // Confirmation response is returned as raw JSON (not wrapped in output envelope)
    let confirmation_json: Value = serde_json::from_str(content_text).expect("confirmation json");
    assert_eq!(confirmation_json["status"], json!("confirmation_required"));
    assert!(confirmation_json["token"].as_str().unwrap().len() > 10);

    drop(stdin);
    let status = timeout(Duration::from_secs(5), child.wait())
        .await
        .expect("timed out waiting for server exit")
        .expect("wait for server exit");
    assert!(status.success(), "server exited unsuccessfully: {status}");
}

#[tokio::test]
async fn test_doctor_runs() {
    let config = write_config("");

    let output = Command::new(binary_path())
        .args(["doctor", "--config", &config_path(config.path())])
        .output()
        .await
        .expect("run doctor command");

    assert!(output.status.success(), "doctor failed: {}", String::from_utf8_lossy(&output.stderr));

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Ready to start."), "unexpected doctor output: {stdout}");
}
