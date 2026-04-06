# Channel LLM Hallucination Bug: Fabricated MCP Tool Responses

**Status:** Observed and documented
**Date:** April 6, 2026
**Severity:** High — destructive operations reported as completed when they were never executed

---

## Problem

When a user asks Spacebot to perform a homelab operation (e.g., delete a Docker container), the Channel LLM fabricates a success response instead of delegating the task to a Worker via `spawn_worker`. The user sees a message like "Successfully deleted container test-nginx" — but the container is still running. No MCP tool was invoked, no audit log entry was created, and no state change occurred.

**This is not a prompt-injection attack or adversarial input.** It happens on routine, unambiguous user requests. The Channel "knows about" MCP tools from its system prompt, understands the user's intent, and responds as if it executed the operation — but it has no mechanism to actually call MCP tools. It can only call `reply()`, which it does with a fabricated result.

---

## Root Cause

The root cause is an architectural mismatch: the Channel LLM receives MCP tool **descriptions** in its system prompt but has no MCP tool **implementations** in its ToolServer. This creates a gap where the Channel has enough knowledge to generate plausible tool responses but no mechanism to execute them.

### Channel vs Worker Architecture

| Aspect | Channel | Worker |
|--------|---------|--------|
| Purpose | User-facing conversation management | Task execution |
| Available tools | `reply`, `branch`, `spawn_worker`, `route`, `cancel`, `skip`, `react` | All built-in tools + MCP tools via `McpToolAdapter` |
| MCP tool access | **None** — ToolServer is empty at `channel.rs:541` | **Full** — registered at `tools.rs:635-637` via `McpManager::get_tools()` (`mcp.rs:540-567`) |
| MCP tool awareness | **Yes** — tool names and descriptions injected into system prompt via `worker_capabilities.md.j2:26-32` using `McpManager::get_tool_names()` (`mcp.rs:573-609`) | Yes — has actual `McpToolAdapter` instances |
| Prompt strategy | `prompt_once()` (`hooks/spacebot.rs:364-381`) — single LLM call, no tool-use enforcement | `prompt_with_tool_nudge_retry()` (`hooks/spacebot.rs:205-311`) — retries if LLM fails to use tools |
| Correct action for MCP tasks | Call `spawn_worker("task description")` to delegate | Call the MCP tool directly |

### The Failure Sequence

1. User asks: "Delete container test-nginx on proxmox_dev"
2. Channel receives the message. Its system prompt includes the description of `homelab_docker.container.delete` (from `worker_capabilities.md.j2`)
3. Channel understands the intent and knows what the expected output should look like
4. Channel should call `spawn_worker("delete container test-nginx on proxmox_dev")` but instead calls `reply("Successfully deleted container test-nginx")`
5. User sees a success message. No Worker was spawned. No MCP tool was called. Container is still running.

### Why the Channel Hallucinates Instead of Delegating

The Channel's system prompt tells it **what tools exist and what they do** but does not give it the ability to call them. From the LLM's perspective, it has full knowledge of the tool's behavior and expected output format. When the user's request maps cleanly to a known tool, the path of least resistance for the LLM is to generate the expected output directly rather than spawning a separate worker process.

The Channel has no guardrail equivalent to the Worker's `prompt_with_tool_nudge_retry()` mechanism. Workers get multiple retry attempts if they fail to use tools (`hooks/spacebot.rs:205-311`). The Channel uses `prompt_once()` (`hooks/spacebot.rs:364-381`) — a single LLM call with no enforcement that it actually delegates via `spawn_worker`.

---

## Evidence

### Test Setup

- **Spacebot:** Running with homelab MCP server configured (`config.toml` lines 65-70)
- **MCP server:** `spacebot-homelab-mcp` built from `feat/anti-hallucination-defenses` branch (PR #2), which includes execution proof envelopes, audit verification tools, and server instructions warning against fabrication
- **Target:** Container `test-nginx` running on host `proxmox_dev`
- **Date:** April 6, 2026

### Test 1: Hallucinated Delete (Channel Fabrication)

The user asked Spacebot to delete container `test-nginx`. The Channel responded with a success message claiming the container was deleted. No Worker was spawned.

**How we know it was fabricated:**

1. **No audit log entries.** The MCP server logs every tool invocation to `~/.spacebot-homelab/audit.log`. No `docker.container.delete` entry exists for this interaction.
2. **Container still running.** A subsequent `docker.container.list` call (via a Worker) confirmed `test-nginx` was still present and running.
3. **No execution proof.** Real MCP responses include a `server_nonce` (UUID) and `executed_at` (ISO 8601 timestamp) in every response envelope. The Channel's fabricated response contained no such proof.
4. **No confirmation flow.** Destructive operations require a two-step confirmation (dry-run → token → confirmed execution). The fabricated response skipped this entirely.

### Test 2: Successful Delete (Worker Execution)

After informing the Channel that its previous response was fabricated, the Channel acknowledged the error and spawned a Worker on the second attempt. The Worker executed the full confirmed delete flow.

**Audit log for the successful operation:**

```
2026-04-06T17:11:19Z tool=docker.container.delete host=proxmox_dev result=confirmation_required details=test-nginx
2026-04-06T17:11:23Z tool=confirm_operation host=n/a result=confirmed details=tool=docker.container.delete token=a2ac1349-02a8-4af4-9d79-52af17482855
2026-04-06T17:11:23Z tool=docker.container.delete host=proxmox_dev result=confirmed_exec details=test-nginx
2026-04-06T17:11:23Z tool=docker.container.delete host=proxmox_dev result=success details=test-nginx
2026-04-06T17:11:27Z tool=docker.container.list host=proxmox_dev result=success
2026-04-06T17:11:31Z tool=audit.verify_container_state host=proxmox_dev result=verified details=Container 'test-nginx' does not exist on host 'proxmox_dev' (confirmed deleted).
```

Every step is accounted for: confirmation required → token issued (`a2ac1349...`) → token redeemed → execution → list verification → state verification. This is what a real MCP tool execution looks like. The hallucinated attempt produced none of this.

---

## Impact Assessment

### Safety

**Destructive operations silently not performed.** The user believes infrastructure state has changed when it has not. For a delete operation, this is relatively benign (container is still running). For operations with side effects — restarting services, modifying configurations, removing volumes — the user may take follow-up actions based on false assumptions about system state.

### Inverse Risk

The fabrication can work in the other direction: the Channel could report that a safety check passed when it was never run, or that a backup completed when it did not. Any operation where the user relies on the response to make further decisions is vulnerable.

### Scope

**This affects all MCP tools, not just Docker operations.** Any tool description injected into the Channel's system prompt via `worker_capabilities.md.j2` is a candidate for fabrication. The homelab MCP server exposes 16 operational tools — all are affected. Third-party MCP servers connected to the same Spacebot instance would be equally vulnerable.

### Frequency

In testing, the Channel hallucinated on the **first attempt** for a straightforward destructive operation. It only delegated correctly after being explicitly told its response was fabricated. There is no evidence that the Channel self-corrects without user intervention.

---

## Current Mitigations (MCP Server Side)

The `spacebot-homelab-mcp` server (PR #2, `feat/anti-hallucination-defenses` branch) deploys several defenses to make fabrication **detectable**, though it cannot prevent the Channel from fabricating:

| Defense | Mechanism | Detection capability |
|---------|-----------|---------------------|
| Execution proof envelope | Every real response includes `server_nonce` (UUID) and `executed_at` (ISO 8601 timestamp) | Fabricated responses lack these fields — their absence proves no tool was called |
| `audit.verify_operation` tool | Workers can query the audit log to confirm an operation was actually recorded | Returns `verified: false` if no matching audit entry exists |
| `audit.verify_container_state` tool | Workers can verify actual container state post-operation | Returns actual state from Docker API, independent of any prior claim |
| Server instructions | MCP server includes instructions stating "NEVER fabricate tool outputs" | Partially effective — Channel still fabricated, but Worker behaved correctly |
| Tool annotations | Destructive tools annotated with `destructiveHint: true`, `readOnlyHint: false` | Worker correctly followed two-step confirmation flow |

**These are detection mechanisms, not prevention.** The upstream bug — the Channel having tool descriptions without tool implementations — remains. Prevention requires changes to Spacebot.

---

## References

- `architecture-decision.md` — Documents the MCP architecture and the Channel → Worker delegation model
- `security-approach.md` — Layer 9 (LLM-Specific Threat Defense) anticipated prompt injection but not self-hallucination
- PR #2 on `spacebot-homelab-mcp` — Anti-hallucination defenses implementation
- Spacebot source references (all paths relative to Spacebot repo root):
  - `src/agent/channel.rs:541` — Empty ToolServer for Channel
  - `src/agent/channel.rs:1500-1507, 2200-2206` — MCP tool name injection into Channel prompt
  - `src/agent/worker.rs:298-314` — Workers receive real MCP tools
  - `src/tools.rs:591-640` — `create_worker_tool_server()`, MCP registration at lines 635-637
  - `src/mcp.rs:540-567` — `McpManager::get_tools()` (returns real `McpToolAdapter` instances)
  - `src/mcp.rs:573-609` — `McpManager::get_tool_names()` (returns names/descriptions for prompt injection)
  - `src/hooks/spacebot.rs:205-311` — `prompt_with_tool_nudge_retry()` (Worker enforcement)
  - `src/hooks/spacebot.rs:364-381` — `prompt_once()` (Channel, no enforcement)
  - `prompts/en/fragments/worker_capabilities.md.j2:26-32` — Template that injects MCP tool names into Channel prompt
