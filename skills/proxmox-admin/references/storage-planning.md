# Storage Planning

Storage pool types, content restrictions, and capacity planning for Proxmox VE.

## Checking current storage

```
proxmox.storage.list (host=<PVE_HOST>)
```

Output shows: storage name, type, status, used/total space, usage percentage, and content types.

## Storage types

| Type | Description | Best for | Performance |
|------|-------------|----------|-------------|
| `dir` | Directory on a filesystem | ISOs, templates, VZDump backups | Depends on underlying FS |
| `lvm` | LVM logical volumes | VM disk images | Good; no thin provisioning |
| `lvmthin` | LVM thin provisioning | VM disk images, CT rootfs | Good; supports snapshots, CoW |
| `zfspool` | ZFS dataset | VM disk images, CT rootfs | Excellent; snapshots, compression, dedup |
| `nfs` | NFS remote mount | Backups, ISOs, shared storage | Network-dependent |
| `cifs` | SMB/CIFS remote mount | Backups, ISOs | Network-dependent |
| `cephfs` / `rbd` | Ceph distributed storage | Multi-node shared VM storage | Good; requires Ceph cluster |
| `pbs` | Proxmox Backup Server | Backups (deduplicated, incremental) | Excellent for backup workloads |

## Content types

Each storage pool declares which content types it can hold:

| Content | Meaning | Typical pools |
|---------|---------|---------------|
| `images` | VM disk images (QEMU) | lvmthin, zfspool, nfs, rbd |
| `rootdir` | CT rootfs (LXC) | lvmthin, zfspool, dir, nfs |
| `iso` | ISO installation images | dir, nfs, cifs |
| `vztmpl` | LXC container templates | dir, nfs, cifs |
| `backup` | VZDump backup files | dir, nfs, cifs, pbs |
| `snippets` | Snippets (cloud-init, hookscripts) | dir, nfs |

**Key rule:** you cannot store a VM disk image on a pool that only supports `iso,vztmpl,backup`. Check the `content` field in `proxmox.storage.list` output.

## Default storage layout

A fresh Proxmox installation typically has:

| Pool | Type | Content | Notes |
|------|------|---------|-------|
| `local` | dir | iso, vztmpl, backup, snippets | `/var/lib/vz` -- for ISOs and templates |
| `local-lvm` | lvmthin | images, rootdir | Thin-provisioned LV for VM/CT disks |

## Capacity planning guidelines

### Warning thresholds

| Usage | Action |
|-------|--------|
| < 70% | Healthy. No action needed. |
| 70-85% | Monitor. Plan cleanup or expansion. |
| 85-95% | Warning. Clean up old snapshots, unused VMs, orphaned disks. |
| > 95% | Critical. Proxmox may refuse to create new VMs. Immediate cleanup required. |

### LVM thin overcommit

LVM thin pools report "virtual" size vs "actual" usage. A 100 GB thin pool can hold VMs with 200 GB of allocated disk space if they haven't written that much data. But once actual writes exceed pool capacity, the pool enters a degraded state.

Check actual thin pool usage via SSH:
```bash
lvs -o lv_name,lv_size,data_percent,pool_lv
```

### ZFS-specific

ZFS reports usage differently. Check with:
```bash
zfs list -o name,used,avail,refer,mountpoint
zfs list -t snapshot -o name,used,refer    # Snapshot space usage
```

ZFS snapshots can consume significant space when the original data is modified frequently.

## Storage selection for new VMs

**Decision matrix:**

| Requirement | Recommended pool type |
|-------------|----------------------|
| Fast OS disk | lvmthin (SSD) or zfspool (SSD/NVMe) |
| VM with snapshots needed | lvmthin or zfspool (both support CoW snapshots) |
| Bulk data / media | NFS or CIFS mount (e.g., from TrueNAS) |
| Shared across cluster nodes | rbd (Ceph) or NFS |
| Backup target | dir on separate disk, NFS, or PBS |

## Adding a new storage pool

Storage pool management is not available through the MCP tools. Use the Proxmox web UI or SSH:

```bash
# Add an NFS storage:
pvesm add nfs backup-nfs \
  --server 10.0.1.50 \
  --export /mnt/backups \
  --content backup

# Add a ZFS pool:
pvesm add zfspool fast-ssd \
  --pool rpool/data \
  --content images,rootdir

# Add a directory:
pvesm add dir iso-store \
  --path /mnt/iso \
  --content iso,vztmpl
```

## Cleaning up storage

### Find orphaned disk images (via SSH)

```bash
# List all disk images in a storage pool:
pvesm list local-lvm

# Compare against running VM configs:
qm list
pct list

# Remove an unused disk:
pvesm free local-lvm:vm-999-disk-0
```

### Remove old backups

```bash
# List backups:
ls /var/lib/vz/dump/

# Remove specific backup:
rm /var/lib/vz/dump/vzdump-qemu-100-*.vma.zst
```
