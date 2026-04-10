# VM/CT Lifecycle

Step-by-step procedures for creating, cloning, starting, stopping, and deleting Proxmox VMs and LXC containers using Spacebot MCP tools.

## Creating a QEMU VM from scratch

Use this when no suitable template exists.

1. **Check capacity:**
   ```
   proxmox.node.status (host=<PVE_HOST>)
   ```
   Verify sufficient RAM and CPU.

2. **Check storage:**
   ```
   proxmox.storage.list (host=<PVE_HOST>)
   ```
   Identify a pool with `images` in its content field and enough free space.

3. **Check existing VMIDs:**
   ```
   proxmox.vm.list (host=<PVE_HOST>)
   ```
   Choose a VMID that fits the user's numbering convention.

4. **Preview the creation:**
   ```
   proxmox.vm.create (
     host=<PVE_HOST>,
     vmid=<VMID>,
     vm_type="qemu",
     name="my-new-vm",
     cores=2,
     memory=2048,
     ostype="l26",
     iso="local:iso/ubuntu-24.04.iso",
     storage="local-lvm",
     disk_size="32G",
     net="virtio,bridge=vmbr0",
     dry_run=true
   )
   ```

5. **Execute the creation** (will require confirmation):
   ```
   proxmox.vm.create (...same params, dry_run=false)
   confirm_operation (token=<returned_token>, tool_name="proxmox.vm.create")
   ```

6. **Start the VM:**
   ```
   proxmox.vm.start (host=<PVE_HOST>, vmid=<VMID>)
   ```

7. **Verify:**
   ```
   proxmox.vm.status (host=<PVE_HOST>, vmid=<VMID>)
   ```

## Creating an LXC container from scratch

1. Follow steps 1-3 above.

2. **Create the container:**
   ```
   proxmox.vm.create (
     host=<PVE_HOST>,
     vmid=<VMID>,
     vm_type="lxc",
     name="my-container",
     cores=1,
     memory=512,
     ostype="local:vztmpl/ubuntu-24.04-standard_24.04-1_amd64.tar.zst",
     storage="local-lvm",
     disk_size="8",
     net="name=eth0,bridge=vmbr0,ip=dhcp",
     dry_run=true
   )
   ```

   Note: for LXC, `ostype` is the OS template path (not an ISO), `disk_size` is in GB (no suffix), and `net` uses LXC-specific format.

3. Execute, start, and verify as above.

## Cloning from a template

This is the preferred provisioning path.

1. **Identify the template:**
   ```
   proxmox.vm.list (host=<PVE_HOST>)
   ```
   Look for template VMIDs (commonly in the 9000+ range).

2. **Preview the clone:**
   ```
   proxmox.vm.clone (
     host=<PVE_HOST>,
     vmid=<TEMPLATE_VMID>,
     name="new-service-vm",
     full=true,
     dry_run=true
   )
   ```

3. **Execute the clone:**
   ```
   proxmox.vm.clone (...same params, dry_run=false)
   ```
   The API will auto-assign a VMID. To specify one, add `newid=<VMID>`.

4. **Start and verify:**
   ```
   proxmox.vm.start (host=<PVE_HOST>, vmid=<NEW_VMID>)
   proxmox.vm.status (host=<PVE_HOST>, vmid=<NEW_VMID>)
   ```

## Full clone vs linked clone

| Aspect | Full Clone | Linked Clone |
|--------|-----------|--------------|
| Speed | Slow (copies all disk data) | Fast (CoW snapshot) |
| Storage | Independent, uses full disk space | Minimal initial, grows with writes |
| Template dependency | None -- safe to delete template | Depends on template disk |
| Use case | Production, long-lived VMs | Dev/test, ephemeral workloads |

Set `full=true` for full clone (default), `full=false` for linked clone.

## Stopping a VM/CT

```
proxmox.vm.stop (host=<PVE_HOST>, vmid=<VMID>, dry_run=true)
proxmox.vm.stop (host=<PVE_HOST>, vmid=<VMID>)
confirm_operation (token=<returned_token>, tool_name="proxmox.vm.stop")
```

This performs a graceful shutdown (ACPI shutdown signal for QEMU, `shutdown` for LXC). The VM's guest OS should handle the signal and shut down cleanly.

If the VM doesn't respond to graceful shutdown (hung guest), use SSH to force-stop:
```bash
qm stop <VMID>   # Force-stop QEMU VM
pct stop <VMID>   # Force-stop LXC container
```

## Deleting a VM/CT

**Prerequisites:**
- VM must be stopped.
- Verify no linked clones depend on this VM (if it was used as a template source).

```
proxmox.vm.status (host=<PVE_HOST>, vmid=<VMID>)   # Confirm stopped
proxmox.vm.delete (host=<PVE_HOST>, vmid=<VMID>, purge=true, dry_run=true)
proxmox.vm.delete (host=<PVE_HOST>, vmid=<VMID>, purge=true)
confirm_operation (token=<returned_token>, tool_name="proxmox.vm.delete")
```

The `purge=true` option also removes unreferenced disk images and purges the VM from backup jobs and replication configs.

## Converting a VM to a template

Templates cannot be created through the Proxmox API. Use SSH:

```bash
# Prepare the VM (run inside the guest):
sudo cloud-init clean
sudo truncate -s 0 /etc/machine-id
sudo rm -f /etc/ssh/ssh_host_*
sudo history -c

# On the Proxmox host:
qm shutdown <VMID>
qm template <VMID>
```

After conversion, the VM icon changes and it can only be cloned, not started.
