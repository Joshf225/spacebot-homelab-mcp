# Peer Review: Spacebot Homelab Integration Ideas

**Date:** March 31, 2026

---

## User's Original Request

Please peer review the ideas in @spacebot/homelab-integration/ and give me the reality check of downsides to any of the approaches, what can be improved on and any suggestions.

---

## Context Documents Reviewed

The following documents were reviewed as part of this peer review:

### 1. homelab-integration-2.md

This document discusses combining both the "dev" and "homelab" feature sets using Rust's Cargo feature flags. Key points:

- Proposes a hybrid approach where the core engine is always compiled, with default features (dev tools) and opt-in homelab features
- Installation options: `cargo install spacebot`, `cargo install spacebot --features homelab`, or `cargo install spacebot --features "dev homelab"`
- Tools are namespaced to avoid conflicts (e.g., `homelab.docker.run` vs `dev.shell`)
- User config would specify which tools are available via `include` patterns
- Argument: The hybrid approach serves the "homelabber who is also a web developer" user profile

### 2. homelab-integration-solution.md

This document addresses whether to fork Spacebot vs. integrate homelab tools into the main project:

- **Why forking is wrong**: Maintenance burden, upstream drift, becomes a separate project within months
- **Why feature flags solve the "bloat" concern**: Rust's compile-time optimization means tools not compiled don't exist in the binary
- **Three options proposed:**
  - **Option A (Recommended):** Upstream contribution with feature flags
  - **Option B:** External plugin crate (if upstream says no)
  - **Option C:** "Homelab Edition" distribution (wraps Spacebot with homelab defaults)

### 3. session-ses_2d35.md

Background research session about Spacebot documentation and homelab use cases:

- Spacebot is an agentic AI system built in Rust with specialized processes (Channel, Branch, Worker, Compactor, Cortex)
- Single binary with embedded databases (SQLite, LanceDB, redb)
- Process model: delegation is the only way work gets done
- User's goal: integrate homelab tools to serve DevOps/Networkers/homelabbers
- Research was conducted into virtualization, networking, containerization, storage, self-hosted apps, monitoring, and automation tools

---

# Reality Check: Downsides to the Approaches

## 1. Compile-Time Feature Gating Doesn't Solve the Real Problem

The feature flag approach gates code at compile time, but the docs assume this means "non-homelabbers don't pull in Docker tools they don't use." This is partially true, but:

- **Dependencies still accumulate in `Cargo.toml`**: You cannot conditionally add dependencies based on features in Rust. If `homelab-compute` feature needs `bollard` (Docker API crate) and `russh` (SSH crate), these must be declared as dependencies. The only optimization is whether they get compiled into the binary.
- **Separate dependency features needed**: You need features like `homelab-compute = ["dep:russh", "dep:bollard"]` to properly gate dependencies.
- **Compile time impact**: Even if not compiled into the binary, the dependency graph still affects resolution and build caching.

**Better approach**: Document explicitly that `cargo install spacebot --features homelab` will pull in extra dependencies (~15-20 additional crates) and increase compile times.

---

## 2. The "Include" Pattern Has a Critical Flaw

The proposed config:
```toml
[agents.tools]
include = ["homelab.*", "dev.shell"]
```

This is a solid concept but has two significant problems:

### Problem A: Compile-Time vs Runtime Mismatch

If the `homelab` feature isn't compiled into the binary, but a user's config includes `homelab.docker.run`, the agent will attempt to call a tool that doesn't exist. The agent needs to know *at runtime* (or ideally, at config validation time) which tools are actually available.

**Fix needed**: Implement config validation that checks whether requested tools exist based on compiled features, with clear error messages.

### Problem B: Tool Routing Complexity

With 30+ tools across categories (shell, file, docker, proxmox, pihole, etc.), the LLM needs to learn which tools are available. This creates:

- **Prompt bloat**: System prompts become longer as they enumerate all available tools with descriptions
- **Token cost**: More tools in the prompt = more tokens per turn
- **Decision complexity**: The LLM may get confused about which tool to use for a given task

**Fix needed**: Document how the agent discovers available tools. Consider generating a tool manifest based on compiled features and injecting it into the system prompt. Group tools logically (compute, network, storage) to help the LLM reason about them.

---

## 3. Homelab Tools Are Fundamentally Different from Dev Tools

The docs compare them as if they're interchangeable:

| Dev Tools | Homelab Tools |
|-----------|---------------|
| shell, file | proxmox, docker, ssh |

But the operational model is completely different:

### Dev Tools (Current Spacebot)
- Stateless
- File-based operations
- Workspace-constrained
- Short-lived invocations

### Homelab Tools (New Domain)
- **Stateful API clients**: Need to maintain connections to Proxmox, Docker daemon, etc.
- **SSH session management**: Connection pooling, keep-alive, reconnection
- **Credential handling**: SSH keys, API tokens, TLS certificates
- **Async retry logic**: Network operations fail constantly; need backoff, retries
- **State machines**: Operations have lifecycle states (e.g., "VM starting" → "VM running" → "VM stopping")
- **Idempotency concerns**: "create container" should be idempotent; "delete container" should check existence first

**This isn't a feature flag addition — it's a new subsystem.** Consider whether this complexity fits Spacebot's architecture. The tool execution model may need enhancement to handle:
- Connection pooling across tool invocations
- Persistent SSH sessions
- Async operation status polling

---

## 4. The "Homelabber Who Is Also a Web Developer" Profile May Be Rare

The docs assume a specific intersection:
> A homelabber who runs *arr stack, Pi-hole, and Home Assistant who also has a personal blog, a portfolio site, maybe a small side project

This user exists, but the intersection may be smaller than expected. Consider that many homelabbers:

- **Don't write code at all** — they want infrastructure management, not development tools
- **Already have other tools** — GitHub Codespaces, VS Code Server, local IDEs for code work
- **Want Spacebot to handle infrastructure** — they don't want it to also be their coding assistant
- **Have different tools for different contexts** — Docker Compose for homelab, VS Code for code

**The "full" build might serve fewer users than expected.** Consider that most homelabbers want infrastructure tools, not necessarily dev tools. The value proposition should be tested with the target audience before building.

---

## 5. Upstream Acceptance is Not Guaranteed

The docs assume:
> The Spacebot team would likely welcome this. It's a major new use case that validates their architecture.

This is optimistic but risky. Real considerations:

- **Roadmap conflicts**: They may have a roadmap that doesn't include homelab integrations
- **Scope creep concerns**: They may want to keep the project focused on "developer-focused AI assistant"
- **Review timeline**: The PR review process could take months; you may need to maintain a fork in the meantime
- **Maintenance burden**: They may accept the code but not commit to maintaining it long-term
- **Architectural objections**: They may disagree with how tools are integrated (feature flags vs. plugin system vs. separate binaries)

**Risk**: You spend months building something that upstream doesn't want, or that gets rejected after significant effort.

---

## 6. The Distribution Model Creates Ongoing Work

Option C (Homelab Edition distribution):
```bash
curl -sSL https://homelabbot.sh/install | bash
```

This sounds clean but creates:

- **Ongoing maintenance**: Every Spacebot release means testing your distribution, updating your documentation, potentially fixing compatibility issues
- **Support burden**: Users report issues to you, not upstream; you become the first responder
- **Branding effort**: Documentation, website, community, issue tracking
- **Security responsibility**: If upstream has a security vulnerability, you need to quickly rebuild and release

This is essentially a full-time project if the distribution gains traction. It may be more work than contributing to upstream.

---

## 7. Tool Naming Conflicts Still Exist

Even with namespacing (`homelab.docker.run` vs `dev.shell`), you risk:

- **`deploy` appearing in both tool sets**: homelab might have "deploy docker stack" and dev might have "deploy to production"
- **`restart` ambiguity**: `homelab.vm.restart` vs `homelab.container.restart` vs `dev.service.restart`
- **Wrong tool selection**: The LLM might pick `homelab.vm.create` when the user wants `homelab.docker.create` for a container

**Mitigation needed**: Clear documentation on tool naming conventions. Consider structured naming: `{category}.{resource}.{action}` (e.g., `proxmox.vm.start`, `docker.container.stop`, `pihole.dns.query`).

---

## 8. Resource Usage on Low-End Hardware

Even with feature flags:
- Spacebot runs multiple concurrent processes (channel, branches, workers, cortex)
- Memory graph + 3 databases are always running (SQLite, LanceDB, redb)
- LLM inference requires significant RAM

This is not lightweight. A Raspberry Pi or low-end homelab server may:

- **Struggle with memory**: 1-2GB RAM is tight for the full stack
- **Have slow storage**: SD cards are too slow for database operations
- **Lack CPU for inference**: Local LLM inference is unrealistic on ARM entry-level hardware

**Document minimum requirements clearly.** Consider whether a "light" mode exists (fewer concurrent processes, no cortex, etc.).

---

# What Can Be Improved

## 1. Add a Plugin System Instead of Just Feature Flags

Rather than compiling tools into the binary, design a plugin interface:

```rust
pub trait HomelabTool {
    fn execute(&self, params: Params) -> Result<Output>;
    fn name(&self) -> &str;
    fn description(&self) -> &str;
}

pub trait ToolRegistry {
    fn register(&mut self, tool: Box<dyn HomelabTool>);
    fn list_tools(&self) -> Vec<&str>;
    fn get_tool(&self, name: &str) -> Option<&dyn HomelabTool>;
}
```

Benefits:
- Allows versioning independent of Spacebot
- Lets users pick which tools they want (no rebuild required)
- Creates a clear extension API
- External crates can provide tools without modifying Spacebot

**Even if upstream rejects homelab in-tree, they'd likely accept a plugin interface.** This is a lower-commitment proposal that creates the foundation for many extensions beyond homelab.

---

## 2. Start with a Minimal V1

Don't try to integrate everything at once. Prioritize the most universal tools:

- **V1 (Foundation):** Docker API + SSH (most homelabbers use Docker; SSH is universal)
- **V2 (Virtualization):** Proxmox (if user has it; many don't)
- **V3 (Storage):** TrueNAS API, ZFS commands (more niche)
- **V4 (Network):** Pi-hole, OPNsense (advanced users only)

This gives you a working product faster and validates demand before investing in niche tools.

---

## 3. Separate "Knowledge" from "Capability"

The docs conflate these concepts:
- **Skills** = what the agent knows (knowledge base, domain expertise)
- **Tools** = what the agent can do (actions, API calls)

A homelab agent needs both, but consider allowing independent gating:

```toml
[agents]
skills = ["homelab.infrastructure", "homelab.storage"]

[agents.tools]
include = ["docker.*"]  # Can do Docker, but doesn't know Proxmox
```

This way:
- A user can have homelab knowledge without tools (for testing/simulation)
- A user can have tools without full knowledge (for ad-hoc operations)
- Skills can be loaded from files; tools are compiled-in

---

## 4. Add Safety Features for Destructive Operations

Dangerous operations (destroy VM, delete container, format disk) should have:

```rust
#[tool]
async fn proxmox_vm_destroy(vm_id: String, dry_run: bool) -> Result<String> {
    if dry_run {
        return Ok(format!("Would destroy VM {} - use dry_run=false to execute", vm_id));
    }
    // Actual destruction
}
```

Required safety features:
- **`--dry-run` flag** on all destructive tools
- **Confirmation step** for high-impact operations
- **Audit logging** of all destructive actions
- **Config option** to disable certain tool categories entirely
- **Rate limiting** to prevent accidental repeated operations

This is critical for trust. Homelabbers won't use a tool that could destroy their infrastructure without safeguards.

---

## 5. Consider the Offline/Local-First Angle

The homelab philosophy emphasizes self-hosted, local control. Spacebot already fits this, but homelab tools have varying offline capabilities:

| Tool | Offline Capable? |
|------|------------------|
| SSH to local server | Yes |
| Docker API (local) | Yes |
| Proxmox (local) | Yes |
| Pi-hole (local DNS) | Yes |
| Cloud APIs (AWS, Azure) | No |
| Remote monitoring | Depends |

Consider:
- **Can homelab tools work without internet?** (Yes for local, no for cloud)
- **Can agents run entirely offline?** (Yes, if models are local)
- **Is there a local-only mode?** (Flag to disable cloud-only features)

This could be a key differentiator from cloud-based AI assistants. Document which tools require network access.

---

## 6. Improve Config Validation and Error Messages

The current proposal lacks validation:

- What happens if user requests a tool that doesn't exist?
- What happens if tool API credentials are missing?
- What happens if SSH connection fails?

**Improvements needed:**
- Startup validation: Check that all configured tools exist based on compiled features
- Clear error messages: "Tool 'homelab.proxmox.create_vm' not available. Compile with --features homelab-compute"
- Connection validation: Provide a `spacebot doctor` or `spacebot validate` command
- Credential check: Verify API keys/SSH keys exist before registering tools

---

# Suggestions

## 1. Validate the User Profile First

Before building significant code, survey the homelab community:

- **r/homelab**: "What would you want an AI assistant to do in your homelab?"
- **r/selfhosted**: "Would you want one tool for both coding and infrastructure, or separate tools?"
- **Twitter/Mastodon**: Test concepts with homelab influencers

This validates whether the "hybrid user" actually exists and what features they'd prioritize. Building without validation risks creating something nobody wants.

---

## 2. Propose a Plugin Interface First, Not the Full Implementation

Instead of "here's 30 homelab tools," propose:

> "We'd like to add a plugin system so Spacebot can be extended with domain-specific tools. Here's a draft trait. Would you accept a PR for this?"

This is lower-risk for upstream and creates the foundation for homelab tools without needing immediate buy-in on the full scope. Even if they reject the plugin system, you've learned something about their priorities.

---

## 3. Build a Proof of Concept with 2-3 Tools

Pick the most universal and demonstrate value:

1. **Docker API** (highest value, most universal)
   - List containers, start/stop/create/remove
   - List images, pull images
   - View logs, stats

2. **SSH** (universal access pattern)
   - Execute commands on remote hosts
   - File transfer capability

3. **Optional: Pi-hole** (common but not universal)
   - Query DNS logs
   - Add/remove DNS entries

Show it works, then expand. Don't try to map the entire homelab ecosystem in V1.

---

## 4. Document the Trade-offs Explicitly

The existing docs are persuasive but one-sided. Add a section like:

> **Trade-offs**
>
> - Binary size increases ~20MB with full homelab features
> - Compile time increases ~2-3 minutes with full feature set
> - More dependencies = more potential security vulnerabilities to track
> - Some features may not work on ARM (Raspberry Pi)
> - Requires network access for most tools (SSH, Docker API, etc.)
> - More complex tool routing = more LLM prompt tokens
> - Maintenance burden for upstream-compatible tool implementations

This builds trust with users and shows you've considered the downsides.

---

## 5. Consider a Staged Rollout

| Phase | Focus | Scope |
|-------|-------|-------|
| Phase 1 | External crate | `spacebot-homelab` crate with Docker + SSH tools |
| Phase 2 | Plugin interface | Propose to upstream; if accepted, integrate |
| Phase 3 | Full features | Based on Phase 1 learnings |

This gives you:
- An escape route if upstream says no
- Working code you can use immediately
- Feedback from actual users before committing to the full scope

---

## 6. Add Migration Path for Existing Users

If adding homelab features changes configuration format:

- Provide migration scripts
- Support old config format for at least one release
- Document breaking changes clearly

Don't break existing users' setups.

---

# Summary

| Approach | Pros | Cons |
|----------|------|------|
| Feature flags | Single binary, compile-time optimization | Dependency bloat in Cargo.toml, testing matrix grows, tool routing complexity |
| Fork | Complete control, no upstream constraints | Maintenance nightmare, upstream drift, effectively a new project within months |
| Distribution | Branded experience, no fork maintenance | Ongoing maintenance, dependency on upstream, support burden |
| Plugin system | Extensible, independent versioning, clean separation | Requires upstream buy-in for interface design |

---

## Recommendation

**Best path**: Propose a plugin interface to upstream first (low commitment, tests the waters), build a minimal proof-of-concept with Docker+SSH (high value, low scope, immediate utility), then expand based on feedback.

The ideas are solid but could benefit from being more incremental and less all-or-nothing. Start small, validate with users, grow based on evidence rather than assumptions.

---

## Appendix: Key Questions to Answer Before Proceeding

1. **Who is the target user?** (Survey first)
2. **What features do they actually want?** (Not what we assume)
3. **Is upstream receptive to a plugin interface?** (Test before building)
4. **What's the minimum viable tool set?** (Docker + SSH is likely enough for V1)
5. **How will you handle credential management?** (SSH keys, API tokens, TLS)
6. **What's the failure mode for network operations?** (Retry, timeout, clear errors)
7. **How do users verify tools work before using them?** (Validation command)
8. **What's the security model for destructive operations?** (Dry-run, confirmation, logging)