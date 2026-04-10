---
name: proxmox-admin
description: Manage Proxmox VE VM/CT lifecycle, resource planning, templates, snapshots, storage, VLAN configuration, and troubleshooting. Covers QEMU VMs and LXC containers, clone-based provisioning, VMID numbering, storage pool selection, network bridge setup, and common failure modes.
version: 1.0.0
---

# Proxmox VE Administration

## Purpose

Use this skill when a Spacebot agent needs to create, manage, troubleshoot, or plan Proxmox VE virtual machines and LXC containers. This includes day-to-day lifecycle operations, template-based provisioning, snapshot management, resource capacity planning, storage selection, and network/VLAN configuration.

This playbook separates five commonly conflated areas:

- **QEMU VMs vs LXC containers** -- different resource models, different use cases, different API paths.
- **Template cloning vs fresh creation** -- cloning is the standard provisioning path; direct creation is for bootstrapping templates.
- **Linked clones vs full clones** -- different storage and performance tradeoffs.
- **Storage types and content restrictions** -- not every storage pool can hold every content type.
- **Network bridges vs VLANs vs bonds** -- distinct layers of network configuration on a Proxmox host.

## When to invoke this skill

- A VM or CT needs to be created, started, stopped, cloned, or deleted.
- A template needs to be created from an existing VM for repeatable provisioning.
- Snapshots need to be taken before risky changes, or rolled back after failures.
- Storage capacity is running low and the agent needs to assess pool usage.
- A new service needs a VM/CT and the agent must choose the right type, size, storage pool, and VLAN.
- Network bridges or VLAN-tagged interfaces need to be verified or planned.
- A Proxmox API call fails and the agent needs to diagnose the cause.
- The agent needs to understand which VMID range to use for a new workload.

## Available MCP tools

This skill depends on the following Spacebot MCP tools:

| Tool | Purpose | Confirmation Required |
|------|---------|----------------------|
| `proxmox.node.list` | List cluster nodes with CPU, memory, uptime | No |
| `proxmox.node.status` | Detailed node status | No |
| `proxmox.vm.list` | List VMs/CTs on a node (filter by qemu/lxc) | No |
| `proxmox.vm.status` | Detailed VM/CT status (CPU, memory, disk, network I/O) | No |
| `proxmox.vm.start` | Start a VM/CT | No |
| `proxmox.vm.stop` | Graceful shutdown of a VM/CT | Yes |
| `proxmox.vm.create` | Create a new VM/CT | Yes |
| `proxmox.vm.clone` | Clone a VM from a template or existing VM | No |
| `proxmox.vm.delete` | Permanently delete a VM/CT | Yes |
| `proxmox.vm.snapshot.list` | List snapshots for a VM/CT | No |
| `proxmox.vm.snapshot.create` | Create a snapshot | No |
| `proxmox.vm.snapshot.rollback` | Rollback to a previous snapshot (current state lost) | Yes |
| `proxmox.storage.list` | List storage pools with usage | No |
| `proxmox.network.list` | List network interfaces (bridges, VLANs, bonds) | No |

Additionally, `ssh.exec` may be used for operations not covered by the Proxmox API (e.g., editing `/etc/network/interfaces`, running `qm` or `pct` CLI commands directly, checking kernel modules).

## Environment variables

Throughout this skill, the following placeholders are used. The agent must resolve these to actual values from the user's environment before executing operations.

| Placeholder | Meaning | Example |
|---|---|---|
| `<PVE_HOST>` | Proxmox host config name in Spacebot | `pve1` |
| `<NODE>` | Proxmox node name within the cluster | `pve1` |
| `<VMID>` | Numeric VM/CT identifier | `100` |
| `<TEMPLATE_VMID>` | VMID of a template to clone from | `9000` |
| `<STORAGE>` | Storage pool name | `local-lvm` |
| `<BRIDGE>` | Network bridge name | `vmbr0` |
| `<VLAN_TAG>` | 802.1Q VLAN tag number | `50` |
| `<SUBNET_CIDR>` | Network range for the VLAN/bridge | `10.0.50.0/24` |
| `<ISO_PATH>` | ISO image path in Proxmox storage | `local:iso/ubuntu-24.04.iso` |

## High-confidence lessons learned

These patterns recur across Proxmox homelab environments and represent the most common mistakes and best practices.

1. **Always clone from templates; never build from scratch repeatedly.**
   - Create a golden template VM (e.g., VMID 9000-9099), install the OS, run cloud-init or your setup script, convert to template.
   - All future VMs of that type are cloned from the template. This is faster, more consistent, and reduces manual error.
   - Templates cannot be started. They must be cloned first.

2. **Use full clones for production, linked clones for ephemeral/test.**
   - **Full clone:** independent copy of all disks. No dependency on the template. Safe to delete the template later. Higher storage cost.
   - **Linked clone:** CoW snapshot-based. Very fast to create, uses minimal initial storage. Depends on the template -- if the template is deleted or corrupted, the clone breaks. Good for dev/test VMs that will be destroyed soon.

3. **VMID numbering conventions prevent collisions and confusion.**
   - Common pattern: `100-199` infrastructure VMs, `200-299` application VMs, `300-399` LXC containers, `9000-9099` templates.
   - The exact ranges don't matter -- what matters is having a convention and following it.
   - Use `proxmox.vm.list` to see existing VMIDs before creating new ones. The API's `/cluster/nextid` endpoint returns the next available VMID, but it doesn't respect numbering conventions.

4. **Stop a VM before deleting it.**
   - The Proxmox API will reject a delete request for a running VM. Always stop first, then delete.
   - Use `proxmox.vm.status` to confirm it's stopped before calling `proxmox.vm.delete`.

5. **Snapshot before risky changes, not after.**
   - Before modifying a VM's configuration, upgrading software, or changing network settings: create a snapshot.
   - Name snapshots descriptively: `pre-upgrade-2024-01-15`, `before-network-change`, not `snap1`.
   - Snapshots with RAM state (`vmstate=true`) allow resuming from the exact point. Without RAM state, the VM will boot fresh from the snapshot's disk state.

6. **Storage pool content types matter.**
   - `local` (directory storage) typically holds ISOs and container templates.
   - `local-lvm` (LVM thin) typically holds VM disk images and CT rootfs.
   - Not all storage types support all content. Check `proxmox.storage.list` to see the `content` field for each pool.
   - ZFS pools can hold both images and rootfs but have different performance characteristics than LVM thin.

7. **LXC containers are lighter than QEMU VMs but less isolated.**
   - Use LXC for trusted, single-purpose services (Pi-hole, nginx reverse proxy, monitoring agents).
   - Use QEMU for anything needing full kernel isolation, custom kernels, or running non-Linux OSes.
   - LXC containers share the host kernel. This means no custom kernel modules, no different kernel versions, and a smaller security boundary.

8. **The Proxmox API uses async tasks for mutating operations.**
   - POST requests for start, stop, create, clone, delete, and snapshot operations return a UPID (Unique Process ID) string.
   - The Spacebot tools automatically poll the task status and wait for completion (up to 120 seconds).
   - If a tool returns a timeout message, the task may still be running. Check the Proxmox web UI or use SSH to run `qm status <VMID>`.

## Symptoms classification

Use the first matching category.

### 1) VM/CT won't start

Typical signs:
- `proxmox.vm.start` returns an error or the task fails.
- The VM appears as "stopped" after a start attempt.

Most likely causes:
- **Insufficient resources:** not enough free RAM or CPU on the node.
- **Storage not available:** the storage pool holding the VM's disk is inactive or full.
- **Lock file:** a previous operation left a lock (`.lock` file). Common after a failed migration or snapshot.
- **Missing ISO:** the VM config references an ISO that's been moved or deleted.

First actions:
1. `proxmox.node.status` -- check available RAM and CPU.
2. `proxmox.storage.list` -- check storage pool health and free space.
3. `proxmox.vm.status` -- look for lock indicators in the output.
4. Via SSH: `qm config <VMID>` to inspect the full VM configuration.

### 2) VM/CT is unreachable on the network after creation

Typical signs:
- VM is running (status shows "running") but cannot be pinged or accessed on the expected IP.
- Other VMs on the same bridge work fine.

Most likely causes:
- **Wrong bridge:** the VM's network interface is attached to the wrong `vmbr` bridge.
- **Missing VLAN tag:** if VLANs are in use, the VM's NIC may need a VLAN tag that wasn't set.
- **No IP configured inside the VM:** the VM booted but cloud-init or DHCP didn't run, or the network config inside the guest is wrong.
- **Firewall:** Proxmox has a built-in firewall that can block traffic if enabled but not configured.

First actions:
1. `proxmox.vm.status` -- confirm it's actually running.
2. `proxmox.network.list` -- verify the bridge exists and is active.
3. Via SSH to Proxmox host: `qm config <VMID>` to check the `net0` line.
4. Via SSH: `qm guest exec <VMID> -- ip addr` (requires qemu-guest-agent) to check the guest's network config.

### 3) Clone operation fails or is extremely slow

Typical signs:
- `proxmox.vm.clone` returns a task timeout.
- The clone appears to start but never completes.

Most likely causes:
- **Full clone on slow storage:** full clones of large disks on HDD-backed storage can take a very long time.
- **Source VM is running:** cloning a running VM requires more I/O and may be slower. Prefer cloning stopped VMs or templates.
- **Storage pool full:** not enough free space on the target storage for the cloned disk.

First actions:
1. `proxmox.storage.list` -- check free space on the target storage.
2. Consider using a linked clone instead if the workload is ephemeral.
3. If possible, stop the source VM before cloning.

### 4) Snapshot rollback fails

Typical signs:
- `proxmox.vm.snapshot.rollback` returns an error.
- The VM state does not change after the rollback.

Most likely causes:
- **VM is running:** some snapshot configurations require the VM to be stopped before rollback.
- **Snapshot chain corruption:** if the underlying storage has issues, snapshot metadata can become inconsistent.
- **Wrong snapshot name:** the snapshot name is case-sensitive.

First actions:
1. `proxmox.vm.snapshot.list` -- confirm the snapshot name exists.
2. Stop the VM first, then retry the rollback.
3. Via SSH: `qm snapshot <VMID> list` for additional detail.

### 5) Storage pool shows high usage or errors

Typical signs:
- `proxmox.storage.list` shows a pool at >90% usage.
- VM creation or clone fails with "no space left" errors.

Most likely causes:
- **Orphaned disk images:** deleted VMs may have left disk images behind.
- **Snapshot accumulation:** many snapshots over time consume significant space on LVM thin and ZFS.
- **Thin provisioning overcommit:** LVM thin pools can overcommit, making the apparent usage lower than reality.

First actions:
1. `proxmox.storage.list` -- get current usage numbers.
2. `proxmox.vm.list` -- identify VMs that can be cleaned up.
3. `proxmox.vm.snapshot.list` on suspect VMs -- identify old snapshots to remove.
4. Via SSH: `lvs` or `zfs list -t snapshot` to see actual disk-level usage.

## Decision tree

Follow in order when provisioning a new workload.

### Step 1: QEMU VM or LXC container?

**Use LXC if:**
- The workload is a single Linux service (DNS, reverse proxy, monitoring agent).
- You want minimal resource overhead.
- You don't need custom kernel modules or a non-Linux OS.
- The workload is trusted (same trust boundary as the host kernel).

**Use QEMU if:**
- You need full OS isolation (different kernel, Windows, BSD).
- The workload is untrusted or multi-tenant.
- You need GPU passthrough, PCI passthrough, or custom hardware access.
- You need to run Docker-in-VM (Docker inside LXC is fragile).

### Step 2: Clone from template or create fresh?

**Clone from template if:**
- A template exists for the desired OS/base image.
- This is a standard deployment (not a one-off experiment with a new OS).

**Create fresh if:**
- You're bootstrapping a new template (first VM of its kind).
- You need to install from an ISO that hasn't been templated yet.

To clone:
1. Identify the template VMID: `proxmox.vm.list` (look for template entries or known template VMIDs in the user's convention).
2. `proxmox.vm.clone` with `vmid=<TEMPLATE_VMID>`, `name=<new-name>`, `full=true` (or `false` for linked).
3. After clone completes, `proxmox.vm.start` to boot the new VM.

To create fresh:
1. `proxmox.vm.create` with cores, memory, storage, ISO, and network parameters.
2. `proxmox.vm.start` to boot from the ISO.
3. After OS installation, consider converting to template for future use.

### Step 3: Choose resource sizing

**Typical homelab sizing (starting points, adjust based on workload):**

| Workload | Type | Cores | RAM | Disk |
|----------|------|-------|-----|------|
| Pi-hole / DNS | LXC | 1 | 256 MB | 4 GB |
| Nginx reverse proxy | LXC | 1 | 512 MB | 8 GB |
| Docker host (light) | QEMU | 2 | 2 GB | 32 GB |
| Docker host (medium) | QEMU | 4 | 8 GB | 64 GB |
| Plex/Jellyfin | QEMU | 4 | 4 GB | 32 GB + media mount |
| Database (Postgres/MySQL) | QEMU | 2 | 4 GB | 32 GB |
| Home Assistant | QEMU | 2 | 2 GB | 32 GB |
| Windows desktop | QEMU | 4 | 8 GB | 64 GB |

Always check available resources first: `proxmox.node.status` for node-level capacity.

### Step 4: Choose storage pool

1. `proxmox.storage.list` -- see available pools and their free space.
2. Match content type: VM disk images need a pool with `images` in its content field. CT rootfs needs `rootdir`. ISOs need `iso`.
3. Prefer local fast storage (SSD/NVMe-backed LVM thin or ZFS) for OS disks.
4. Use NFS/CIFS for bulk media storage or backups.

### Step 5: Choose network configuration

1. `proxmox.network.list` -- see available bridges and VLANs.
2. Default: `virtio,bridge=vmbr0` (standard paravirtualized NIC on the main bridge).
3. If VLANs are in use: `virtio,bridge=vmbr0,tag=<VLAN_TAG>`.
4. For isolated networks: use a separate bridge (e.g., `vmbr1` for an internal-only network).

## VMID numbering conventions

The agent should follow the user's established convention. If no convention exists, suggest:

| Range | Purpose |
|-------|---------|
| 100-199 | Infrastructure VMs (DNS, DHCP, reverse proxy, monitoring) |
| 200-299 | Application VMs (Docker hosts, databases, media servers) |
| 300-399 | LXC containers |
| 400-499 | Test/ephemeral VMs |
| 9000-9099 | Templates (QEMU) |
| 9100-9199 | Templates (LXC) |

Check existing VMIDs with `proxmox.vm.list` before choosing a new one.

## Template management

### Creating a template from an existing VM

Templates are created via the Proxmox CLI (not the REST API):

```bash
# Via SSH to the Proxmox host:
qm template <VMID>
```

Before converting:
1. Ensure the VM is stopped.
2. Install and configure cloud-init (for automated hostname, SSH key, network setup on clone).
3. Clean up: remove SSH host keys, machine-id, bash history.
4. The VM becomes permanently read-only after conversion. It cannot be started, only cloned.

### Cloud-init integration

For QEMU templates with cloud-init:
1. Add a cloud-init drive: `qm set <VMID> --ide2 <STORAGE>:cloudinit`
2. Set defaults: `qm set <VMID> --ciuser admin --sshkeys ~/.ssh/authorized_keys --ipconfig0 ip=dhcp`
3. Clones inherit these settings but can override them before first boot.

## Safety rules

1. **Snapshot before destructive operations.** Before stopping, deleting, or rolling back a VM, create a snapshot first if the VM has unsaved state.

2. **Use dry_run on mutating tools.** All mutating Proxmox tools support `dry_run=true`. Use it to preview the operation before executing.

3. **Confirm destructive tools require two steps.** Tools marked with "Confirmation Required" return a confirmation token on first call. The agent must call `confirm_operation` with that token to execute. This prevents accidental destruction.

4. **Never delete a template that has linked clones.** Linked clones depend on the template's disk. Deleting the template will corrupt all linked clones. Use `proxmox.vm.list` to check for VMs that might be linked clones before deleting a template.

5. **Stop before delete.** A running VM cannot be deleted. Always stop first, verify it's stopped, then delete.

6. **Check node capacity before creating VMs.** Don't overcommit RAM beyond ~80% of physical unless the workloads are known to be light. CPU overcommit is generally fine up to 4:1 for typical homelab loads.

## Troubleshooting: API connectivity issues

### Authentication failures

Symptoms: HTTP 401 from any Proxmox tool.

Check:
- Token ID format: must be `USER@REALM!TOKENID` (e.g., `root@pam!spacebot`).
- Token secret: must be the UUID returned when the token was created.
- `--privsep 0` on the token: if privilege separation is enabled (default), the token needs its own ACL entries.
- Token hasn't expired (tokens can have expiration dates).

### TLS/certificate errors

Symptoms: connection refused or TLS handshake failure.

Check:
- `verify_tls = false` in config (default for homelab self-signed certs).
- The Proxmox host is reachable on port 8006.
- No firewall blocking the connection between Spacebot's host and the Proxmox host.

### Permission denied on specific operations

Symptoms: HTTP 403 on a tool that previously worked.

Check:
- The API token user has the required role. `PVEVMAdmin` covers most VM operations. `PVEAdmin` covers everything.
- ACLs may be scoped to specific paths (e.g., `/vms/100` instead of `/`). Ensure the token has permissions on the target VMID's path.

## Recommended procedural flow for agents

When managing Proxmox VMs/CTs:

1. **Assess current state:** `proxmox.node.list`, `proxmox.vm.list`, `proxmox.storage.list`.
2. **Check capacity:** `proxmox.node.status` for CPU/RAM availability.
3. **Determine VM type:** QEMU or LXC based on the workload (see decision tree).
4. **Choose provisioning method:** clone from template (preferred) or create fresh.
5. **Select VMID:** follow the user's numbering convention or suggest one.
6. **Select storage:** match content type to pool, prefer fast local storage for OS disks.
7. **Select network:** bridge + VLAN tag based on the network segment for this workload.
8. **Preview with dry_run:** use `dry_run=true` on create/clone/delete before executing.
9. **Execute and verify:** run the operation, then verify with `proxmox.vm.status`.
10. **Snapshot after stable state:** once the VM is configured and working, take a snapshot as a restore point.

## Fast diagnosis cheatsheet

| Symptom | Most likely fix |
|---|---|
| VM won't start: "not enough memory" | Check `proxmox.node.status`; reduce VM memory or stop unused VMs |
| VM won't start: "storage not active" | Check `proxmox.storage.list`; remount or repair the storage pool |
| VM starts but no network | Check `net0` config via SSH `qm config <VMID>`; verify bridge and VLAN tag |
| Clone fails with timeout | Check storage free space; use linked clone for speed; stop source VM first |
| Delete returns error | Stop the VM first, then retry delete |
| API returns 401 | Verify token_id format (`USER@REALM!TOKENID`) and token_secret |
| API returns 403 on specific VM | Check ACL scope -- token may lack permissions on that VMID path |
| Snapshot rollback fails | Stop the VM first, verify snapshot name with `proxmox.vm.snapshot.list` |
| Storage pool >90% full | Remove old snapshots, delete unused VMs, check for orphaned disks via SSH |

## References

See supporting reference docs in `references/`:

- `vm-lifecycle.md` -- step-by-step VM/CT creation, cloning, deletion procedures.
- `snapshot-management.md` -- snapshot creation, rollback, and cleanup patterns.
- `storage-planning.md` -- storage pool types, content restrictions, and capacity planning.
- `network-configuration.md` -- bridge setup, VLAN tagging, and common network patterns.
- `api-troubleshooting.md` -- Proxmox API authentication, permissions, and error diagnosis.
