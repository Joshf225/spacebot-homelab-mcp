# Backup Management (vzdump)

## Overview

Proxmox vzdump creates full backups of VMs and LXC containers that can be stored on a separate storage pool for disaster recovery. Unlike snapshots (which live on the same storage as the VM), backups are portable and survive storage pool failures.

## Backup modes

| Mode | Downtime | Consistency | Best for |
|------|----------|-------------|----------|
| **snapshot** | None | Good (crash-consistent via LVM/ZFS snapshot) | Production VMs that can't tolerate downtime |
| **suspend** | Brief pause | Better (RAM is paused during backup) | VMs where brief pause is acceptable |
| **stop** | Full stop | Best (VM is fully stopped during backup) | Maintenance windows, templates, non-critical VMs |

**Recommendation:** Use `snapshot` mode for most workloads. Use `stop` only during maintenance windows or for VMs that are already stopped.

## Compression

| Algorithm | Speed | Ratio | CPU usage | Recommendation |
|-----------|-------|-------|-----------|----------------|
| **zstd** | Fast | Best | Moderate | Default choice — best balance |
| **lzo** | Fastest | Lowest | Low | Use when CPU is limited |
| **gzip** | Slow | Good | High | Legacy compatibility only |
| **0** (none) | Instant | None | None | Use for fast local-to-local copies |

**Recommendation:** Use `zstd` unless you have a specific reason not to.

## Storage requirements

- Backup size ≈ VM's **used** disk space (not allocated), minus compression savings.
- With zstd compression, expect 30-60% of used disk size depending on content.
- Always check free space on the backup storage before starting: `proxmox.storage.list`.
- The backup storage pool must have `backup` in its content types.

## Backup workflow

```
1. proxmox.storage.list          → identify backup-capable storage with enough space
2. proxmox.vm.backup.create      → create the backup (mode=snapshot, compress=zstd)
3. proxmox.vm.backup.list        → verify the backup was created
```

## Restore workflow

```
1. proxmox.vm.backup.list        → find the archive volume ID
2. proxmox.vm.list               → verify target VMID is not in use
3. proxmox.vm.backup.restore     → restore (requires confirmation)
4. proxmox.vm.status             → verify the restored VM
5. proxmox.vm.start              → start the restored VM
```

## Common issues

### Backup timeout

Large VMs on slow storage (HDD, NFS over slow network) can exceed the default task wait timeout. Solutions:
- Increase `task_wait_timeout_secs` in the Proxmox host config.
- Use `zstd` compression to reduce I/O.
- Schedule backups during low-activity periods.

### Restore fails: VMID already exists

The restore API creates a new VM with the specified VMID. If that VMID is already in use, the operation fails. Solutions:
- Choose an unused VMID.
- Delete the existing VM first (if it's the one being restored over).

### Storage full during backup

If the backup storage runs out of space mid-backup, the task fails and may leave a partial backup file. Solutions:
- Check free space before starting.
- Remove old backups: `proxmox.vm.backup.list` to identify, then delete via SSH or the Proxmox UI.
- Consider retention policies to auto-prune old backups.

## Retention

Proxmox supports retention policies for automated backup jobs (configured via the Proxmox UI or CLI, not via the API tools). Common retention strategies:

- **keep-last:** Keep the N most recent backups.
- **keep-daily/weekly/monthly:** Keep one backup per time period.
- **Example:** `keep-last=3, keep-weekly=2, keep-monthly=1` — keeps 3 most recent, plus the last 2 weekly and 1 monthly.

For manual backups created via `proxmox.vm.backup.create`, retention must be managed manually by listing and deleting old backups.
