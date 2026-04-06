# Homelab Integration Walkthrough: SAAS Developer Who Also Homelabs

## The User Profile

**Alex** builds a small SAAS (task management app for freelancers). He runs Spacebot locally to:
- Write and deploy code via OpenCode
- Run `git` operations, tests, database migrations
- Manage his production Docker stack on a VPS
- Monitor his app's health

His homelab runs at home:
- Proxmox (VM for his personal services)
- Docker (media stack, home automation)
- Pi-hole (local DNS)
- TrueNAS (backups)

---

## Step 1: The Problem

Alex builds his SAAS like this:

```
┌─────────────────────────────────────────────┐
│  Spacebot (dev mode)                        │
│                                             │
│  ┌─────────────────────────────────────────┐│
│  │  Dev Agent (worker tools: shell, file)  ││
│  │  • Writes code via OpenCode             ││
│  │  • Deploys to VPS via shell             ││
│  └─────────────────────────────────────────┘│
│                                             │
│  His homelab is completely separate:        │
│  • CLI commands on his laptop               │
│  • Portainer UI for Docker                  │
│  • Proxmox web UI                           │
└─────────────────────────────────────────────┘
```

He wants **one assistant** that knows both his code AND his home infrastructure.

---

## Step 2: Installation

Alex installs the homelab MCP server alongside Spacebot:

```bash
# Standalone binary — no Spacebot rebuild required
cargo install spacebot-homelab-mcp
```

Create its config file:

```toml
# ~/.spacebot-homelab/config.toml

[docker]
host = "unix:///var/run/docker.sock"

[ssh.hosts.home]
host = "192.168.1.1"
user = "alex"
private_key_path = "~/.ssh/homelab"

[ssh.hosts.proxmox]
host = "192.168.1.10"
user = "homelab-agent"  # Restricted user, not root — see security-approach.md
private_key_path = "~/.ssh/proxmox"
```

Then add it to his Spacebot agent config:

```toml
# ~/.spacebot/config.toml

# === His existing SAAS agent (unchanged) ===
[[agents]]
id = "dev"

# === NEW: Homelab agent ===
[[agents]]
id = "homelab"

[[agents.mcp]]
name = "homelab"
command = "spacebot-homelab-mcp"
args = ["--config", "~/.spacebot-homelab/config.toml"]
```

Restart Spacebot. The MCP server starts as a child process — Spacebot manages its lifecycle automatically. The homelab tools are available to workers spawned by the `homelab` agent.

> **Note:** This is a separate binary, not a Spacebot plugin. Spacebot's `[[agents.mcp]]` config is a standard feature for connecting to external MCP tool servers. No Spacebot source changes are needed. See `architecture-decision.md` for the rationale.

Verify the setup:

```bash
spacebot-homelab-mcp doctor --config ~/.spacebot-homelab/config.toml
```

```
✓ Docker socket /var/run/docker.sock: accessible
✓ SSH to home (192.168.1.1): OK
✓ SSH to proxmox (192.168.1.10): OK
✓ 8 tools available: docker.container.list, docker.container.start,
  docker.container.stop, docker.container.logs, docker.container.inspect,
  ssh.exec, ssh.upload, ssh.download
```

---

## Step 3: How It Works (Spacebot Process Model)

When Alex sends a message to the homelab agent, Spacebot's process model handles it:

```
Alex (Discord/Telegram)
  │
  ▼
Channel (homelab agent) — receives message, decides what to do
  │
  ├──→ Branch — thinks about what tools/steps are needed
  │
  └──→ spawn_worker — delegates the actual work
        │
        Worker — has MCP tools from spacebot-homelab-mcp
        │        (docker.container.*, ssh.*)
        │
        └──→ MCP protocol ──→ spacebot-homelab-mcp process
                               │
                               ├── Docker API client
                               └── SSH connection pool
```

Workers are the correct process type for homelab operations. They do task work, have no channel context, and report status. The channel stays responsive — it never blocks waiting for an SSH command to finish.

---

## Step 4: The Workflow

Alex talks to both agents from the same Discord server:

### Scenario A: Building his SAAS

> **Alex** (in #dev channel): Deploy the new login flow to production

His `dev` agent spawns a worker that:
- Opens OpenCode, makes changes
- Runs tests
- Deploys via shell commands (`docker build && docker push && ssh deploy@vps ...`)
- Confirms: "Deployed! Health check passed."

### Scenario B: SAAS server trouble

> **Alex** (in #dev channel): Users are reporting 500 errors

His `dev` agent spawns a worker that:
- Uses `shell` tool to run `ssh deploy@vps.example.com 'docker logs webapp --tail 100'`
- Finds: "database connection timeout"
- Runs `ssh deploy@vps.example.com 'docker restart webapp'`
- Verifies: "Restarted. Health check passing now."

> **Note:** The dev agent uses its existing `shell` tool for VPS operations — it doesn't need homelab MCP tools for this. The VPS is part of his SAAS deployment workflow, not his homelab.

### Scenario C: Homelab maintenance

> **Alex** (in #homelab channel): Is my media server still running?

His `homelab` agent spawns a worker that:
- Calls `docker.container.list` (MCP tool)
- Replies: "Jellyfin is stopped. Want me to restart it?"

> **Alex**: yep

The homelab agent spawns another worker:
- Calls `docker.container.start` with container ID
- Replies: "Jellyfin started. Status: running."

### Scenario D: Infrastructure question

> **Alex** (in #homelab channel): How much storage do I have left?

His `homelab` agent spawns a worker that:
- Calls `ssh.exec` with host=`home`, command=`zpool list`
- Replies: "12TB used of 18TB (67%). Backup pool has 8TB free."

---

## Step 5: Scheduled Monitoring

Alex adds a cron job to the **homelab agent** for infrastructure monitoring:

```toml
# Under the homelab agent's config
[[agents]]
id = "homelab"

[[agents.mcp]]
name = "homelab"
command = "spacebot-homelab-mcp"
args = ["--config", "~/.spacebot-homelab/config.toml"]

[agents.bindings.telegram]
chat_ids = [123456]

[[agents.cron]]
id = "homelab-health"
prompt = """
Check my homelab services:
1. Run docker.container.list and report any stopped containers that should be running
2. Run ssh.exec on 'home' with 'zpool status' to check for disk errors
If anything is critical, DM me on Telegram.
"""
interval = "30m"
```

This cron job is scoped to the homelab agent, so it has access to the homelab MCP tools. It cannot access the dev agent's tools or VPS — each agent's workers only get their own agent's tool set.

> **Note about cross-domain monitoring:** If Alex wants a single cron job that checks both his SAAS and his homelab, he would need to add the homelab MCP server to the dev agent as well, or create a dedicated monitoring agent with both tool sets. Cron jobs cannot cross agent boundaries.

---

## What Changed

| Before | After |
|--------|-------|
| Two separate workflows | One Spacebot instance, two specialized agents |
| Homelab = Portainer UI | Homelab = chat interface via MCP tools |
| "Check my server" = manually SSH | "Check my server" = message the agent |
| Dev and ops are isolated | Each agent handles its own domain cleanly |
| No monitoring | Cron jobs within each agent's scope |

**Alex**: "I already use Spacebot to build my SAAS. Now it also manages my homelab. I added a binary and a config block — no Spacebot rebuild, no source changes, no fork."
