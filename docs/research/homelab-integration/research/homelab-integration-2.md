You're asking about combining both approaches. Let me lay this out clearly:
The Two Axes
Axis	Option A
Feature Set	Feature flags for tools
Result	One binary, compile-time tool selection
What You're Proposing Now (The Hybrid)
┌─────────────────────────────────────────┐
│        Spacebot (single binary)         │
│                                         │
│  Core Engine (always compiled)          │
│  ├── Channel, Branch, Worker, Cortex    │
│  ├── Memory Graph, LLM Routing         │
│  ├── Messaging, Cron, Skills            │
│  └── Tool Framework (Rig)              │
│                                         │
│  ┌───────────────────────────────────┐  │
│  │ Default Features (always there)  │  │
│  │ ├── shell, git, file tools        │  │
│  │ ├── browser, code editing         │  │
│  │ ├── OpenCode integration          │  │
│  │ └── Dev skills                    │  │
│  └───────────────────────────────────┘  │
│                                         │
│  ┌───────────────────────────────────┐  │
│  │ Homelab Features (opt-in)         │  │
│  │ ├── proxmox, docker, ssh          │  │
│  │ ├── pihole, opnsense, caddy        │  │
│  │ ├── zfs, truenas, borgmatic        │  │
│  │ └── homelab skills                 │  │
│  └───────────────────────────────────┘  │
└─────────────────────────────────────────┘
Compile options:
# Dev only (default)
cargo install spacebot
# Homelab only
cargo install spacebot --features homelab
# Both (your idea)
cargo install spacebot --features "dev homelab"
# or
cargo install spacebot --features full
Comparison: This vs. Path 1 (Core Extraction)
Aspect	Feature Flags (This)
Binary count	1
Installation	Simple (one install)
Tool conflicts	Possible (both have "deploy"? need namespacing)
Upstream sync	Same as today
Feature coupling	All in one crate, harder separation
Maintenance	Simpler initially, messier later
Shared users	Same user has both
Resource usage	All compiled in = larger RAM/disk
My Take
The hybrid (feature flags with everything) is actually the best of both worlds for your specific goal: "homelabbers can be web developers without needing to code."
That user profile exists:
- A homelabber who runs *arr stack, Pi-hole, and Home Assistant
- Who also has a personal blog, a portfolio site, maybe a small side project
- Who wants to manage both their infrastructure AND their code from the same interface
- Maybe they copy-paste code but don't write it from scratch
- Maybe they want "deploy my website" to just work
This is a real user. They don't want two apps. They want one assistant that can do everything they need.
How to Actually Build This
The Path 1 refactor becomes unnecessary if you just structure the features correctly:
# Spacebot's Cargo.toml
[features]
default = ["dev"]
dev = ["spacebot-tools-dev"]
homelab = ["spacebot-tools-homelab"]
full = ["dev", "homelab"]
# Each tool set is its own internal module, behind cfg flags
[dependencies]
# ... core deps ...
[package]
name = "spacebot"
The tools are namespaced so there's no conflict:
// In code:
#[cfg(feature = "homelab")]
pub mod homelab_tools {
    pub mod proxmox;
    pub mod pihole;
    // ...
}
#[cfg(feature = "dev")]
pub mod dev_tools {
    pub mod shell;
    pub mod opencode;
    // ...
}
Users register tools they're allowed to use in their agent config:
# User's config
[[agents]]
id = "main"
[agents.tools]
# These are available
include = [
    "shell",
    "file_read",
    "file_write",
    "proxmox.create_vm",
    "docker.run",
    "pihole.query",
]
# Or by category
include = ["homelab.*", "dev.shell"]
The Winner
For your goal, feature flags with a "full" build is the right call. It's:
- One app to install
- One app to configure
- One chat interface for all tasks
- Feature flag gated so non-homelabbers don't pull in Docker tools they don't use
- Path 1 refactor is unnecessary complexity for what you're trying to achieve
The only real question is whether to make "full" the default or "dev". I'd argue:
- Default = dev (matches current Spacebot user base)
- "I want everything" = --features full
This preserves backward compatibility, attracts new homelab users incrementally, and gives power users the full experience.
Want to start mapping out the actual feature flag structure in the codebase?
