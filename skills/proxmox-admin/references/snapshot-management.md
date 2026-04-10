# Snapshot Management

Snapshot creation, rollback, and cleanup patterns for Proxmox VMs and LXC containers.

## When to snapshot

- **Before upgrades:** OS updates, kernel upgrades, major package changes.
- **Before configuration changes:** network reconfig, storage changes, service migrations.
- **Before risky operations:** anything that could leave the VM in a broken state.
- **After reaching a known-good state:** baseline snapshot after initial setup.

## Creating a snapshot

```text
proxmox.vm.snapshot.create (
  host=<PVE_HOST>,
  vmid=<VMID>,
  snapname="pre-upgrade-2024-01-15",
  description="Before apt dist-upgrade",
  vmstate=false
)
```

### Naming conventions

Use descriptive, dated names:
- `pre-upgrade-YYYY-MM-DD` -- before OS or package upgrades.
- `before-network-change` -- before network reconfiguration.
- `baseline-v1` -- after initial setup is complete and verified.
- `pre-migration` -- before moving a service to a different VM.

Avoid: `snap1`, `test`, `backup`, or any name that doesn't explain what it protects against.

### With or without RAM state

| vmstate | Behavior | Use case |
|---------|----------|----------|
| `false` | Disk-only snapshot. On rollback, VM boots fresh from this disk state. | Default. Good for most cases. |
| `true` | Includes RAM contents. On rollback, VM resumes from the exact point. | When you need to preserve in-memory state (e.g., active database transactions). |

RAM-state snapshots are larger and only available for QEMU VMs (not LXC).

## Listing snapshots

```text
proxmox.vm.snapshot.list (host=<PVE_HOST>, vmid=<VMID>)
```

Output shows: name, description, creation time, whether RAM state is included, and parent snapshot.

Snapshots form a tree. Each snapshot has a parent (except the first). The `current` entry represents the live state.

## Rolling back to a snapshot

**This is destructive.** All changes since the snapshot will be lost.

1. **List snapshots to verify the target:**
   ```text
   proxmox.vm.snapshot.list (host=<PVE_HOST>, vmid=<VMID>)
   ```

2. **Preview the rollback:**
   ```text
   proxmox.vm.snapshot.rollback (
     host=<PVE_HOST>,
     vmid=<VMID>,
     snapname="pre-upgrade-2024-01-15",
     dry_run=true
   )
   ```

3. **Stop the VM first** (recommended for reliable rollback):
   ```text
   proxmox.vm.stop (host=<PVE_HOST>, vmid=<VMID>)
   confirm_operation (token=<returned_token>, tool_name="proxmox.vm.stop")
   ```

4. **Execute the rollback** (requires confirmation):
   ```text
   proxmox.vm.snapshot.rollback (
     host=<PVE_HOST>,
     vmid=<VMID>,
     snapname="pre-upgrade-2024-01-15"
   )
   confirm_operation (token=<returned_token>, tool_name="proxmox.vm.snapshot.rollback")
   ```

5. **Start the VM:**
   ```text
   proxmox.vm.start (host=<PVE_HOST>, vmid=<VMID>)
   ```

## Cleaning up old snapshots

Snapshots consume storage space. On LVM thin and ZFS, each snapshot holds the delta from the next snapshot in the chain. Over time, many snapshots can consume significant space.

**Guidelines:**
- Keep at most 2-3 snapshots per VM.
- Remove snapshots older than 30 days unless they serve as a critical baseline.
- After a successful upgrade or change, remove the "pre-" snapshot once stability is confirmed.

**To remove a snapshot** (via SSH -- not yet available as an MCP tool):
```bash
qm delsnapshot <VMID> <SNAPNAME>       # QEMU VM
pct delsnapshot <VMID> <SNAPNAME>       # LXC container
```

Snapshot deletion merges the snapshot data into its parent, so it may take time on large snapshots.

## Snapshot vs backup

| Aspect | Snapshot | Backup (vzdump) |
|--------|----------|-----------------|
| Speed | Instant (CoW) | Minutes to hours depending on size |
| Location | Same storage as the VM | Can be remote (NFS, PBS) |
| Purpose | Quick rollback point | Disaster recovery, off-host protection |
| Retention | Short-term (days) | Long-term (weeks/months) |
| Risk | If storage fails, snapshot is lost | Independent copy survives storage failure |

Snapshots are **not backups.** They exist on the same storage as the VM. Use vzdump/PBS for real backups.

## Common issues

### "Can't rollback: VM is running"

Stop the VM first. Some snapshot configurations (especially those without RAM state) require the VM to be stopped.

### "Snapshot chain too long"

Proxmox warns when the snapshot chain exceeds a certain depth. This causes performance degradation on LVM thin. Consolidate by removing intermediate snapshots.

### "Not enough space for snapshot"

Check `proxmox.storage.list`. Snapshots need space for future writes (CoW). If the pool is nearly full, clean up old snapshots or unused VMs first.
