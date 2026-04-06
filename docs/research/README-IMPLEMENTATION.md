# Implementation Plan Index

This folder contains the complete, detailed implementation plan for the `spacebot-homelab-mcp` project across all five milestones (M1-M5).

## Quick Links

### 📘 For First-Time Readers
Start here: **[IMPLEMENTATION-GUIDE.md](IMPLEMENTATION-GUIDE.md)**
- High-level overview of all 5 milestones
- Project structure and dependencies
- Execution order and verification steps
- Troubleshooting quick reference

### 🔧 For M1 (Binary Bootstrap)
**[M1-Implementation.md](M1-Implementation.md)**
- 11 detailed step-by-step instructions
- MCP server setup with stdio transport
- Config loading and validation
- Tool registration framework
- Spacebot integration verification

### 🐳 For M2-M5 (Tools, Safety, Validation)
**[M2-M5-Implementation.md](M2-M5-Implementation.md)**
- **M2:** Docker tools (5 tools, 2-3 days)
- **M3:** SSH tools (3 tools, 2-3 days)
- **M4:** Safety & observability (rate limiting, audit logging, 1-2 days)
- **M5:** End-to-end validation with Spacebot (1 day)

### 📋 Reference Documents
**In `spacebot/homelab-integration/`:**
- `poc-specification.md` — Tool schemas, test plans, requirements
- `architecture-decision.md` — Why MCP was chosen
- `security-approach.md` — 10-layer security model
- `connection-manager.md` — SSH pool and Docker client design

## Implementation Status

| Milestone | Status | Days | Details |
|-----------|--------|------|---------|
| M1 | ✅ Planned | 1-2 | Binary boots, MCP server ready |
| M2 | 📋 Planned | 2-3 | Docker tools (5 tools) |
| M3 | 📋 Planned | 2-3 | SSH tools (3 tools) |
| M4 | 📋 Planned | 1-2 | Safety & observability |
| M5 | 📋 Planned | 1 | End-to-end validation |
| **Total** | | **7-11 days** | Complete PoC |

## How to Use These Plans

### Option 1: Follow Step-by-Step (Recommended)
1. Read `IMPLEMENTATION-GUIDE.md` for overview
2. Execute M1 using `M1-Implementation.md`
3. Execute M2-M5 using `M2-M5-Implementation.md`
4. Refer back to `IMPLEMENTATION-GUIDE.md` for verification at each stage

### Option 2: Reference Specific Milestone
- **Need to implement Docker tools?** → Jump to M2 section in `M2-M5-Implementation.md`
- **Implementing SSH?** → Jump to M3 section
- **Adding rate limiting?** → Jump to M4 section
- **Verifying with Spacebot?** → Jump to M5 section

### Option 3: Quick Lookup
Use `IMPLEMENTATION-GUIDE.md` for:
- Project structure overview
- Dependency list by milestone
- Key patterns and conventions
- Verification commands
- Troubleshooting quick reference

## Key Features of This Plan

✅ **Extremely Detailed**
- Step-by-step instructions for every change
- Current code vs. replacement code shown
- Line numbers for file locations
- Rationale for each decision

✅ **Executable by Another Model**
- No ambiguous instructions
- All dependencies documented
- Error handling guidance provided
- Complete code examples
- Verification steps for each change

✅ **Production-Quality**
- Security gates (allowlist/blocklist)
- Audit logging for compliance
- Rate limiting to prevent abuse
- Error handling and recovery
- Health monitoring and diagnostics

✅ **Well-Organized**
- Clear milestone boundaries
- Phased implementation approach
- Cross-references between documents
- Future enhancements documented

## Document Statistics

| Document | Size | Lines | Content |
|----------|------|-------|---------|
| M1-Implementation.md | 17 KB | 583 | Binary bootstrap, MCP setup |
| M2-M5-Implementation.md | 57 KB | 2,047 | All tools, safety, validation |
| IMPLEMENTATION-GUIDE.md | 12 KB | 322 | Quick reference & overview |
| README-IMPLEMENTATION.md | 4 KB | 180 | This file |
| **Total** | **90 KB** | **3,132** | **Complete specification** |

## Code Examples Included

- **M1:** MCP server handler, stdio transport, CLI setup (~100 lines)
- **M2:** DockerClient init, 5 tool handlers, output formatting (~1,200 lines)
- **M3:** SSH pool/session types, 3 tool handlers, validation (~800 lines)
- **M4:** RateLimiter implementation, integration (~300 lines)
- **M5:** Configuration examples, test procedures (documentation)

## Execution Timeline

```
Day 1:    M1 - Binary boots and serves MCP
         ├─ 11 implementation steps
         ├─ Cargo.toml updates
         ├─ MCP server creation
         └─ Verification tests

Days 2-4: M2 - Docker tools work
         ├─ DockerClient initialization
         ├─ 5 Docker tool handlers
         ├─ Tool registration
         └─ Integration testing

Days 5-6: M3 - SSH tools work
         ├─ SSH pool and session types
         ├─ 3 SSH tool handlers
         ├─ Command validation
         └─ Integration testing

Day 7:    M4 - Safety and observability
         ├─ Rate limiting implementation
         ├─ Audit logging enhancement
         ├─ Health monitoring
         └─ Unit tests

Day 8:    M5 - End-to-end validation
         ├─ Spacebot configuration
         ├─ Tool discovery testing
         ├─ Docker/SSH tool testing
         └─ Error handling verification
```

## Key Milestones and Verification

### M1 Completion Criteria
- Binary compiles: `cargo build --release` ✅
- Server starts: `./spacebot-homelab-mcp server` ✅
- Doctor works: `./spacebot-homelab-mcp doctor` ✅
- Spacebot can connect and discover server ✅

### M2 Completion Criteria
- 5 Docker tools registered ✅
- docker.container.list returns formatted output ✅
- Output truncated at 10,000 chars ✅
- Audit logging records invocations ✅
- Integration tests pass ✅

### M3 Completion Criteria
- 3 SSH tools registered ✅
- Command validation tests pass ✅
- Allowlist blocks unauthorized commands ✅
- Blocklist blocks dangerous patterns ✅
- Dry-run mode works ✅

### M4 Completion Criteria
- Rate limiter blocks excess calls ✅
- Audit logging shows all operations ✅
- Doctor shows accurate health status ✅
- Security configuration is validated ✅

### M5 Completion Criteria
- Spacebot discovers all 8 tools ✅
- Docker tools work end-to-end ✅
- SSH tools work end-to-end ✅
- Rate limiting prevents abuse ✅
- Error handling is graceful ✅

## Troubleshooting

### Can't find where to implement something?
→ Use the table of contents in each document to navigate

### Need code examples?
→ M2-M5-Implementation.md has complete code for all implementations

### Getting compilation errors?
→ See the "Troubleshooting Guide" section in M2-M5-Implementation.md

### Want to verify progress?
→ Each milestone has a verification checklist in the respective document

## Future Work

After M5 is complete:
1. SSH connection pooling (true pool reuse)
2. SFTP file transfer implementation
3. Per-user rate limiting
4. Syslog support
5. Background health monitor
6. Prometheus metrics
7. Graceful degradation
8. Performance optimization

See IMPLEMENTATION-GUIDE.md → "Future Enhancements" for details.

## Getting Help

1. **Can't understand a step?** → Read the "Why this works" section
2. **Getting an error?** → Check "Verification" section and try again
3. **Need context?** → Jump to "Architecture" section in the milestone
4. **Want to see working code?** → Look at the code examples in each step

## Quick Commands

```bash
# Build the project
cargo build --release

# Run the server
./target/release/spacebot-homelab-mcp server --config example.config.toml

# Run diagnostics
./target/release/spacebot-homelab-mcp doctor --config example.config.toml

# Run tests
cargo test --lib

# Check Docker integration (if Docker is running)
cargo test --test docker_integration -- --ignored --nocapture
```

## Ready to Start?

👉 **Begin here:** [IMPLEMENTATION-GUIDE.md](IMPLEMENTATION-GUIDE.md)

Or jump directly to:
- **M1:** [M1-Implementation.md](M1-Implementation.md)
- **M2-M5:** [M2-M5-Implementation.md](M2-M5-Implementation.md)

Good luck! 🚀
