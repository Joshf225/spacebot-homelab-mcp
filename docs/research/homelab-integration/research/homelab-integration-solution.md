# Question:

Before we move on, I had a thought: We - Consider making an alternative to spacebot. Or a versioned spacebot thats tailored for homelabbers or even just forking spacebot and making it just for homelabbers - Problem: There are lots of homelabbing tools and softwares, simply adding all of the support etc to spacebot may make it too large and hard to maintain. - Solution: Fork spacebot and make a homelabber - spacebot alternative. Give the app a more tailored feel for the specific user, keeps things clean, removes the noise and extra things that aren’t needed for homelabbers.
11:01 PM

# Answer:

I understand the instinct, but I'd push back on the fork. Here's why, and what I think the actual right move is.
Why Forking is the Wrong Call
The Maintenance Problem Inverts
You're worried that adding homelab tools to Spacebot makes it too large. But forking creates a worse version of that problem:

- Spacebot's core engine (process model, memory graph, LLM routing, messaging, compaction, cortex) is actively developed. Every bug fix, performance improvement, and new LLM provider support happens upstream.
- A fork means you're now maintaining the entire engine plus your homelab layer. Every upstream change requires a manual rebase/merge.
- The Spacebot team ships a new compaction algorithm? You have to port it. They fix a memory leak in the cortex? You have to port it. They add support for a new LLM provider? Port it.
- Within 6 months, the fork drifts so far that merging upstream becomes a full-time job. Within a year, it's effectively a separate project that happens to share ancestry.
  This is the story of almost every ambitious fork. It sounds clean at the start and becomes a maintenance tar pit.
  The "Bloat" Concern is Solvable Without Forking
  The concern is real -- 30 homelab tools compiled into a binary that a non-homelabber doesn't need is wasteful. But Spacebot is written in Rust, which gives us the exact mechanism to solve this:
  Cargo feature flags. This is what they exist for.

# In Spacebot's Cargo.toml

[features]
default = ["core"]
core = []

# Homelab tool sets - opt-in at compile time

homelab = ["homelab-compute", "homelab-network", "homelab-storage",
"homelab-media", "homelab-monitoring", "homelab-automation"]
homelab-compute = ["dep:russh"] # Proxmox, Docker, LXC, SSH
homelab-network = [] # OPNsense, Pi-hole, Caddy, WireGuard
homelab-storage = [] # TrueNAS, ZFS, Borg, Restic, rclone
homelab-media = [] # \*arr stack, Jellyfin, Plex
homelab-monitoring = [] # Grafana, Prometheus, Uptime Kuma
homelab-automation = [] # Ansible runner, Terraform, cloud-init
Build for a homelabber:
cargo install spacebot --features homelab
Build vanilla Spacebot:
cargo install spacebot
Zero bloat for non-homelabbers. Full toolset for homelabbers. One codebase.
Tools that aren't compiled in simply don't exist in the binary. No runtime overhead, no configuration noise, no extra code. This is a core strength of Rust that dynamic languages can't match.
The Tailored UX Doesn't Require a Fork Either
Spacebot already has the mechanisms for a tailored homelabber experience:
Agent identity files define personality and role:

<!-- ~/.spacebot/agents/homelab/SOUL.md -->

You are a homelab infrastructure assistant. You think in terms
of VLANs, Docker stacks, ZFS pools, and backup strategies.
You speak the language of r/homelab and r/selfhosted.

<!-- ~/.spacebot/agents/homelab/ROLE.md -->

You manage a homelab environment. Your responsibilities:

- Infrastructure provisioning (Proxmox VMs/CTs, Docker stacks)
- Network configuration (firewall, DNS, reverse proxy, VPN)
- Storage management (ZFS, backups, replication)
- Service deployment and lifecycle
- Monitoring and alerting
- Security hardening
  Skills define what the agent knows and can do. A homelabber loads homelab skills. A developer loads dev skills. Same engine, different knowledge.
  Tool availability is already per-agent in Spacebot's architecture. A homelab agent gets homelab tools. A coding agent gets coding tools. They don't interfere.

---

What I'd Actually Propose Instead
Option A: Upstream Contribution (Recommended)
Contribute the homelab tools and skills directly to Spacebot behind feature flags. This is the cleanest path:
spacebot/
├── src/
│ ├── tools/
│ │ ├── reply.rs # Core (always compiled)
│ │ ├── branch.rs # Core
│ │ ├── spawn_worker.rs # Core
│ │ └── homelab/ # Behind #[cfg(feature = "homelab-*")]
│ │ ├── mod.rs
│ │ ├── proxmox.rs # #[cfg(feature = "homelab-compute")]
│ │ ├── docker.rs # #[cfg(feature = "homelab-compute")]
│ │ ├── ssh.rs # #[cfg(feature = "homelab-compute")]
│ │ ├── pihole.rs # #[cfg(feature = "homelab-network")]
│ │ ├── opnsense.rs # #[cfg(feature = "homelab-network")]
│ │ ├── caddy.rs # #[cfg(feature = "homelab-network")]
│ │ ├── truenas.rs # #[cfg(feature = "homelab-storage")]
│ │ ├── borgmatic.rs # #[cfg(feature = "homelab-storage")]
│ │ └── ...
│ └── ...
├── skills/
│ └── homelab/ # Skill files, always available
│ ├── proxmox-admin/
│ ├── docker-deploy/
│ ├── network-architect/
│ └── ...
└── Cargo.toml # Feature flags
Why this is best:

- You benefit from all upstream engine improvements automatically
- The Spacebot team benefits from your tools (SSH, Docker API, etc. are useful beyond homelab)
- The community grows rather than fragments
- Homelabbers get a first-class experience with --features homelab
- The Docker image could ship a spacebot:homelab tag pre-compiled with all homelab features
  The Spacebot team would likely welcome this. It's a major new use case that validates their architecture. The project is by Spacedrive (a storage/file management app) -- they're already adjacent to the homelab world.
  Option B: External Plugin/Extension (If Upstream Says No)
  If the Spacebot team doesn't want homelab tools in-tree, the next best option is an external crate that provides tools and skills:

# User's Cargo.toml overlay or build config

[dependencies]
spacebot = "0.x"
spacebot-homelab = "0.1" # Your crate, provides Tool impls
This depends on Spacebot having a stable plugin interface. Looking at their architecture, the tool system uses Rig's Tool trait, which is public. Whether they support external tool registration at runtime is the question -- if they don't today, that's a smaller contribution to propose (a plugin/extension loading mechanism) than forking the entire project.
Option C: The "Homelab Edition" Distribution (Middle Ground)
If you want the branded, tailored experience without a fork, you create a distribution -- a thin wrapper that packages Spacebot with homelab defaults:
homelab-bot/
├── Cargo.toml # Depends on spacebot with features = ["homelab"]
├── src/
│ └── main.rs # Just calls spacebot::run() with homelab defaults
├── config/
│ └── default-config.toml # Pre-configured for homelab use
├── agents/
│ └── homelab/
│ ├── SOUL.md # Homelab personality
│ ├── IDENTITY.md # "I am your homelab assistant"
│ ├── ROLE.md # Homelab responsibilities
│ └── skills/ # Pre-loaded homelab skills
├── Dockerfile # Pre-built with homelab features
└── install.sh # One-liner installer

# User experience:

curl -sSL https://homelabbot.sh/install | bash

# or

docker run -d ghcr.io/you/homelabbot:latest
This gives you:

- Custom branding and identity
- Pre-configured for homelab out of the box
- No fork maintenance burden -- you depend on upstream Spacebot as a library/binary
- Opinionated defaults (homelab skills pre-loaded, connections config ready to fill in)
- The "tailored feel" you want
  This is essentially what Linux distributions do with the Linux kernel. Ubuntu doesn't fork the kernel. It packages it with specific configs, defaults, and tooling. You'd do the same with Spacebot.

---

My Recommendation
Option A (upstream contribution) first, Option C (distribution) second, fork never.

1. Open an issue/discussion on the Spacebot repo proposing the homelab tool integration with feature flags. Share the research we've done -- it's compelling.
2. Start building the tools in a way that's upstreamable (clean Rust, follows Spacebot's patterns, behind feature flags).
3. If you want the branded homelabber experience, create a distribution that wraps Spacebot with homelab defaults.
4. Only fork if the upstream team explicitly rejects the contribution and refuses to support external tool loading. Even then, I'd try Option B (external crate) first.
   The fork path feels clean today but becomes a full-time maintenance job within months. The contribution path feels harder today but scales correctly.
