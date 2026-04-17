# Network Configuration

Bridge setup, VLAN tagging, and common network patterns for Proxmox VE.

## Checking current network

```text
proxmox.network.list (host=<PVE_HOST>)
```

Output shows: interface name, type, IP address, active status, autostart, and bridge ports.

## Network architecture overview

Proxmox networking has three layers:

1. **Physical NICs** (`eno1`, `enp3s0`, etc.) -- the actual hardware interfaces.
2. **Linux bridges** (`vmbr0`, `vmbr1`, etc.) -- virtual switches that connect VMs to the network.
3. **VLAN interfaces** (`vmbr0.50`, `eno1.10`, etc.) -- 802.1Q tagged sub-interfaces.

VMs connect to bridges. Bridges connect to physical NICs (or VLAN-tagged sub-interfaces). This is the fundamental model.

## Default network setup

A fresh Proxmox installation creates:

| Interface | Type | Purpose |
|-----------|------|---------|
| `eno1` (or similar) | Physical NIC | Uplink to the physical network |
| `vmbr0` | Bridge | Main VM bridge, bridged to `eno1` |

The host's management IP is typically assigned to `vmbr0`, not the physical NIC.

## Common homelab patterns

### Single flat network (no VLANs)

All VMs share one bridge, one subnet.

```text
Physical NIC (eno1) <-> vmbr0 (10.0.1.0/24) <-> VMs
```

VM NIC config: `virtio,bridge=vmbr0`

This is the simplest setup. All VMs are on the same L2 segment.

### VLAN-aware bridge

One bridge handles multiple VLANs. The physical switch port must be configured as a trunk.

```text
Physical NIC (eno1) <-> vmbr0 (VLAN-aware) <-> VMs with VLAN tags
```

Configuration (in `/etc/network/interfaces`):
```ini
auto vmbr0
iface vmbr0 inet static
    address 10.0.1.1/24
    gateway 10.0.1.254
    bridge-ports eno1
    bridge-stp off
    bridge-fd 0
    bridge-vlan-aware yes
    bridge-vids 2-4094
```

VM NIC configs:
- Management VLAN (untagged): `virtio,bridge=vmbr0`
- IoT VLAN 10: `virtio,bridge=vmbr0,tag=10`
- Media VLAN 20: `virtio,bridge=vmbr0,tag=20`
- Homelab VLAN 50: `virtio,bridge=vmbr0,tag=50`

### Separate bridges per VLAN

Each VLAN gets its own bridge. More explicit, easier to reason about, but more config.

```text
eno1.10 <-> vmbr1 (IoT VLAN 10)
eno1.20 <-> vmbr2 (Media VLAN 20)
eno1.50 <-> vmbr3 (Homelab VLAN 50)
```

VM NIC config: `virtio,bridge=vmbr1` (no tag needed, the bridge is already on the VLAN).

### Internal-only network

A bridge with no physical NIC attached. VMs on this bridge can only talk to each other.

```text
vmbr99 (no bridge-ports) <-> VMs (isolated network)
```

Useful for: database networks where VMs should only be accessible from app VMs, not from the LAN.

## VLAN tag assignment for VMs

When using a VLAN-aware bridge, set the VLAN tag on the VM's NIC:

| Tool | How to set VLAN |
|------|----------------|
| `proxmox.vm.create` | `net="virtio,bridge=vmbr0,tag=50"` |
| `proxmox.vm.clone` + SSH | After clone: `qm set <VMID> --net0 virtio,bridge=vmbr0,tag=50` |
| Proxmox Web UI | Hardware > Network Device > VLAN Tag |

## Checking VM network config

Via SSH to the Proxmox host:
```bash
# QEMU VM:
qm config <VMID> | grep net

# LXC container:
pct config <VMID> | grep net
```

Example output:
```text
net0: virtio=AA:BB:CC:DD:EE:FF,bridge=vmbr0,tag=50
```

## Network troubleshooting

### VM has no network connectivity

1. **Check VM status:** `proxmox.vm.status` -- is it running?
2. **Check bridge:** `proxmox.network.list` -- is the bridge active?
3. **Check VM NIC config:** SSH `qm config <VMID> | grep net` -- correct bridge and VLAN tag?
4. **Check guest network:** SSH `qm guest exec <VMID> -- ip addr` (needs qemu-guest-agent).
5. **Check physical switch:** if using VLANs, is the switch port configured as a trunk allowing the required VLAN?

### VMs on different VLANs can't communicate

This is expected behavior. VLANs isolate traffic. Communication between VLANs requires a router (firewall/gateway) that routes between the subnets. Typically this is OPNsense, pfSense, or the physical router.

### Bridge shows inactive

Check via SSH:
```bash
ip link show vmbr0
brctl show vmbr0      # or: bridge link show
```

If the bridge's member port (physical NIC) is down, the bridge will be inactive. Check cable, switch port, and NIC status.

## Bonding (link aggregation)

For redundancy or throughput, multiple NICs can be bonded:

```text
bond0 (eno1 + eno2, LACP) <-> vmbr0 <-> VMs
```

Configuration:
```ini
auto bond0
iface bond0 inet manual
    bond-slaves eno1 eno2
    bond-miimon 100
    bond-mode 802.3ad
    bond-xmit-hash-policy layer3+4

auto vmbr0
iface vmbr0 inet static
    address 10.0.1.1/24
    gateway 10.0.1.254
    bridge-ports bond0
    bridge-stp off
    bridge-fd 0
```

The physical switch must support LACP and have the ports configured for it.

## Applying network changes

Network changes made via the Proxmox API (`proxmox.network.create`, `proxmox.network.update`, `proxmox.network.delete`) are **staged** — they do not take effect immediately. Use `proxmox.network.apply` to make them live (equivalent to the "Apply Configuration" button in the Proxmox web UI).

For changes made directly to `/etc/network/interfaces` via SSH, apply with:
- `ifreload -a` -- applies changes without reboot (Proxmox uses ifupdown2).
- `systemctl restart networking` -- restarts networking (brief outage).
- Reboot -- safest but most disruptive.

**Warning:** incorrect network configuration can make the host unreachable. Always have IPMI/iLO/iDRAC access or physical console access as a fallback.

## Creating network interfaces via MCP tools

### Create a bridge

```text
proxmox.network.create(
  iface="vmbr1",
  type="bridge",
  address="10.0.50.1",
  netmask="255.255.255.0",
  bridge_ports="eno2",
  autostart=true,
  comments="Homelab VLAN 50 bridge"
)
proxmox.network.apply()  # Make the change live
```

For an internal-only bridge (no physical uplink):
```text
proxmox.network.create(
  iface="vmbr99",
  type="bridge",
  autostart=true,
  comments="Internal-only bridge for isolated VMs"
)
proxmox.network.apply()
```

### Create a VLAN interface

```text
proxmox.network.create(
  iface="eno1.50",
  type="vlan",
  vlan_id=50,
  vlan_raw_device="eno1",
  autostart=true,
  comments="VLAN 50 on eno1"
)
proxmox.network.apply()
```

### Create a bond

```text
proxmox.network.create(
  iface="bond0",
  type="bond",
  bridge_ports="eno1 eno2",
  bond_mode="802.3ad",
  autostart=true,
  comments="LACP bond for vmbr0"
)
proxmox.network.apply()
```

### Modify an existing interface

```text
proxmox.network.update(
  iface="vmbr1",
  gateway="10.0.50.254",
  comments="Updated gateway"
)
proxmox.network.apply()
```

### Delete an interface

```text
proxmox.network.delete(iface="vmbr1")
proxmox.network.apply()
```

### Workflow: always verify before applying

1. Make changes with `proxmox.network.create` / `proxmox.network.update` / `proxmox.network.delete`.
2. Review staged config: `proxmox.network.list` — verify everything looks correct.
3. Apply: `proxmox.network.apply` — makes changes live.
4. If something goes wrong, use IPMI/console access to revert `/etc/network/interfaces` manually.
