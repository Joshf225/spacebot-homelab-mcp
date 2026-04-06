# Architecture Decision Record: Homelab Integration Approach

**Status:** Accepted
**Date:** March 31, 2026
**Context:** How should homelab tools be integrated into Spacebot?

---

## Decision

**Use MCP (Model Context Protocol) as the primary integration mechanism.** The homelab tools will run as a standalone MCP server (`spacebot-homelab-mcp`) that Spacebot connects to via its existing MCP integration. No changes to Spacebot core are required for V1.

Feature flags remain a viable long-term path if upstream wants homelab tools in-tree, but MCP is the immediate, zero-dependency approach.

---

## Context

Three approaches were proposed across the research documents:

1. **Compile-time feature flags** — Add homelab tools behind `#[cfg(feature = "homelab")]` in Spacebot's source
2. **Runtime plugin system** — Create an external crate with runtime tool loading via `[plugins]` config
3. **Fork** — Rejected universally; maintenance burden is unjustifiable

The peer review identified that approach #2 (runtime plugins) assumes infrastructure that doesn't exist in Spacebot. The use case document (`use_case.md`) silently adopted approach #2 without validating feasibility. This ADR resolves the ambiguity.

---

## Investigation Findings

### Rig's Tool System (v0.33)

| Capability | Supported | Mechanism |
|-----------|-----------|-----------|
| Runtime tool registration | Yes | `ToolServerHandle::add_tool(&self, impl ToolDyn + 'static)` |
| Runtime tool removal | Yes | `ToolServerHandle::remove_tool(&self, &str)` |
| Cross-crate tools | Yes | `ToolDyn` trait is object-safe; blanket impl from `Tool` |
| Handle sharing | Yes | `ToolServerHandle` is `Clone + Send + Sync` |
| MCP tool integration | Yes | `rmcp` feature, `McpToolAdapter` wraps external tools |
| Dynamic library loading | No | No stable Rust ABI; no `dlopen` support |

### Spacebot's Actual Implementation

| Aspect | Finding |
|--------|---------|
| Tool registration | Hardcoded in factory functions in `src/tools.rs` (lines 350-782) |
| Plugin system | **Does not exist.** No `[plugins]` config, no plugin loading, no extension API. |
| MCP support | **Already exists.** `McpToolAdapter` in `src/tools/mcp.rs`. Tools are namespaced as `{server_name}_{tool_name}`. |
| MCP scope | Workers only. MCP tools are passed to `create_worker_tool_server()` and registered at worker spawn. |
| Dynamic tool management | Channel ToolServer already uses `add_tool()`/`remove_tool()` per conversation turn. |

### Critical Finding

**Spacebot already connects to external MCP servers and makes their tools available to workers.** This is configured per-agent in `config.toml`. The mechanism is production-ready — it's how Spacebot integrates with external tool providers today.

Workers are the correct process type for homelab operations: they do task work (shell, file, Docker, SSH), have no channel context, and report status. The channel delegates to workers for all tool execution. This matches the homelab use case perfectly.

---

## Options Evaluated

### Option A: Feature Flags (Compile-Time)

```toml
# Cargo.toml
[features]
homelab = ["homelab-compute", "homelab-network", "homelab-storage"]
homelab-compute = ["dep:bollard", "dep:russh"]
```

| Pro | Con |
|-----|-----|
| Single binary | Requires upstream acceptance |
| Zero runtime overhead for unused tools | Increases compile time for all contributors |
| No separate process to manage | Tightly coupled to Spacebot release cycle |
| | Must follow Spacebot's code style, review process, CI |
| | Cannot iterate independently |

**Verdict:** Good long-term goal. Not actionable today without upstream buy-in.

### Option B: Runtime Plugin (`[plugins]` Config)

```toml
[plugins]
spacebot-homelab = { }
```

| Pro | Con |
|-----|-----|
| Clean separation | **Spacebot has no plugin system** — this must be built from scratch |
| Could be loaded dynamically | Rust has no stable ABI; "load at runtime" means compile-time linking or FFI |
| | Requires changes to Spacebot's config parser, startup sequence, and tool factory |
| | use_case.md's "no rebuild" claim is false for compiled plugins |

**Verdict:** Requires significant upstream engineering to create the plugin infrastructure. The `[plugins]` config section shown in `use_case.md` does not exist. This is a future aspiration, not a current capability.

### Option C: MCP Server (Out-of-Process) -- CHOSEN

```toml
# Spacebot's config.toml — already supported syntax
[[agents]]
id = "homelab"

[[agents.mcp]]
name = "homelab"
command = "spacebot-homelab-mcp"
args = ["--config", "~/.spacebot-homelab/config.toml"]
```

| Pro | Con |
|-----|-----|
| **Works today** — no Spacebot changes needed | Separate process to manage |
| Independent release cycle | Inter-process communication overhead (minimal for tool calls) |
| Can be written in any language | MCP tools are worker-only in Spacebot |
| Own error handling, connection management | Tool descriptions must fit MCP schema |
| Testable in isolation | Extra binary to install |
| Can run on a different machine | |
| Aligns with industry direction (MCP standard) | |

**Verdict:** The only approach that works immediately, requires zero upstream changes, and lets us iterate independently. The "workers-only" constraint is a feature, not a bug — homelab operations belong on workers.

---

## Chosen Architecture

```
┌──────────────────────────────────────────────────────────────┐
│  Spacebot (unchanged)                                         │
│                                                               │
│  Channel ──→ spawn_worker ──→ Worker                          │
│                                    │                          │
│                                    │ (MCP protocol)           │
│                                    ▼                          │
│                          ┌─────────────────────┐              │
│                          │ spacebot-homelab-mcp │              │
│                          │                     │              │
│                          │ ┌─────────────────┐ │              │
│                          │ │ Connection Mgr  │ │              │
│                          │ │ ┌─────┐ ┌─────┐ │ │              │
│                          │ │ │ SSH │ │Docker│ │ │              │
│                          │ │ │Pool │ │Client│ │ │              │
│                          │ │ └─────┘ └─────┘ │ │              │
│                          │ └─────────────────┘ │              │
│                          │                     │              │
│                          │ Tools:              │              │
│                          │  docker.container.* │              │
│                          │  docker.image.*     │              │
│                          │  ssh.exec           │              │
│                          │  ssh.upload         │              │
│                          │  ssh.download       │              │
│                          └─────────────────────┘              │
└──────────────────────────────────────────────────────────────┘
```

### How It Works

1. User installs `spacebot-homelab-mcp` (a standalone Rust binary)
2. User adds an MCP server entry to their Spacebot agent config
3. On agent startup, Spacebot spawns the MCP server as a child process
4. When a worker needs homelab tools, the MCP server provides them via the MCP protocol
5. The MCP server manages its own connections (SSH pool, Docker client, etc.)
6. Credentials are configured in the MCP server's own config file (or via Spacebot's secret store reference)

### What This Means for the Documents

| Document | Required Change |
|----------|----------------|
| `use_case.md` | Replace `[plugins]` with `[[agents.mcp]]`. Remove "no rebuild" claim. Show `cargo install spacebot-homelab-mcp`. |
| `security-approach.md` | Credential management now lives in the MCP server, not Spacebot's secret store. Update all layers accordingly. |
| Connection manager design | Lives inside the MCP server process. Simpler than injecting into Spacebot's internals. |

---

## Future Path

If the homelab MCP server proves valuable and the Spacebot team is receptive:

1. **Phase 1 (now):** Ship as standalone MCP server. Zero upstream dependency.
2. **Phase 2:** Propose to Spacebot team: expand MCP tool availability beyond workers (branches, cortex). This is a small change — just wiring MCP tools into other factory functions.
3. **Phase 3:** If upstream wants homelab tools in-tree, port the MCP tools to native Rig `Tool` implementations behind feature flags. The tool logic is identical; only the transport changes.

Each phase is independently valuable. No phase requires the next to succeed.

---

## Risks

| Risk | Likelihood | Mitigation |
|------|-----------|------------|
| MCP protocol overhead adds latency | Low | MCP over stdio is fast; homelab ops (SSH, Docker API) are network-bound anyway |
| Spacebot changes MCP integration | Low | MCP is a stable protocol; Spacebot's `rmcp` integration follows the spec |
| Users don't want a separate binary | Medium | Ship as a Docker image alongside Spacebot; one `docker-compose.yml` runs both |
| MCP worker-only limitation blocks use cases | Low | Homelab ops are worker tasks; channels delegate, they don't execute |

---

## Decision Log

| Date | Decision | Rationale |
|------|----------|-----------|
| Research phase | Feature flags proposed | Assumed upstream would accept; simplest compile-time model |
| Peer review | Plugin system recommended | Identified feature flag limitations; didn't verify Spacebot's extension points |
| use_case.md | Plugin system assumed | Silently adopted without feasibility check |
| **This ADR** | **MCP server chosen** | Only approach that works today, requires zero upstream changes, proven integration point |
