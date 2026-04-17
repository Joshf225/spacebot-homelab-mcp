# Tailscale Setup

## Overview

This reference covers deploying Tailscale for homelab VPN access. Tailscale is a mesh VPN built on WireGuard that handles NAT traversal, key management, and peer discovery automatically via a coordination server. Traffic flows peer-to-peer and is end-to-end encrypted.

## Installation methods

### Host install (recommended)

The simplest and most reliable method. Works on Debian/Ubuntu, RHEL/CentOS, Arch, macOS, and more.

```
ssh.exec(host="<DOCKER_HOST>", command="curl -fsSL https://tailscale.com/install.sh | sh")
```

After install:
```
ssh.exec(host="<DOCKER_HOST>", command="tailscale up --authkey=<TS_AUTHKEY>")
```

Verify:
```
ssh.exec(host="<DOCKER_HOST>", command="tailscale status")
```

### Docker container

Use when host install is not desirable (e.g., Proxmox LXC, isolated environments).

```yaml
services:
  tailscale:
    image: tailscale/tailscale:latest
    container_name: tailscale
    hostname: homelab-ts
    environment:
      - TS_AUTHKEY=<TS_AUTHKEY>
      - TS_EXTRA_ARGS=--advertise-routes=<LAN_CIDR> --accept-routes
      - TS_STATE_DIR=/var/lib/tailscale
    volumes:
      - ./tailscale/state:/var/lib/tailscale
      - /dev/net/tun:/dev/net/tun
    cap_add:
      - NET_ADMIN
      - NET_RAW
    network_mode: host
    restart: unless-stopped
```

Notes:
- `network_mode: host` is required for subnet routing to work from a container. Without it, the container cannot forward packets to the LAN.
- `/dev/net/tun` is required for the WireGuard tunnel interface.
- `TS_STATE_DIR` persists the node identity across container restarts.
- `TS_EXTRA_ARGS` passes arguments to `tailscale up`.

## Subnet router setup

A subnet router allows Tailscale clients to reach devices on your homelab LAN without installing Tailscale on every device.

### Step 1: Enable IP forwarding on the host

```
ssh.exec(host="<DOCKER_HOST>", command="echo 'net.ipv4.ip_forward=1' > /etc/sysctl.d/99-tailscale.conf && sysctl -p /etc/sysctl.d/99-tailscale.conf")
```

Verify:
```
ssh.exec(host="<DOCKER_HOST>", command="sysctl net.ipv4.ip_forward")
```

### Step 2: Advertise routes

Host install:
```
ssh.exec(host="<DOCKER_HOST>", command="tailscale up --advertise-routes=<LAN_CIDR> --accept-routes")
```

Multiple subnets:
```
ssh.exec(host="<DOCKER_HOST>", command="tailscale up --advertise-routes=10.0.0.0/24,192.168.1.0/24 --accept-routes")
```

### Step 3: Approve routes in admin console

1. Go to https://login.tailscale.com/admin/machines
2. Find the node.
3. Click the three-dot menu > Edit route settings.
4. Toggle on each advertised subnet.
5. Save.

Routes are NOT active until approved. This is the most common reason subnet routing does not work after setup.

### Step 4: Verify from a client

From another Tailscale device, try to reach a LAN IP:
```
ping 10.0.0.1
```

If this fails after routes are approved, see the `tailscale-subnet-router` skill for troubleshooting.

## Exit node configuration

An exit node routes ALL client traffic through the homelab, not just homelab-destined traffic.

### Enable on server

```
ssh.exec(host="<DOCKER_HOST>", command="tailscale up --advertise-routes=<LAN_CIDR> --advertise-exit-node --accept-routes")
```

### Approve in admin console

Same process as subnet routes -- find the node, edit route settings, toggle on "Use as exit node".

### Use on client

- **Desktop**: Tailscale tray icon > Exit Nodes > select your homelab node.
- **iOS/Android**: Tailscale app > Exit Node > select your homelab node.
- **CLI**: `tailscale up --exit-node=<NODE_IP_OR_NAME>`

When an exit node is active, all client traffic (web browsing, DNS, everything) goes through the homelab.

## MagicDNS and custom nameservers

### MagicDNS

Tailscale provides MagicDNS, which lets you reach Tailscale devices by hostname (e.g., `homelab-ts` instead of `100.x.y.z`). Enabled by default in newer Tailnets.

### Custom nameservers (homelab DNS)

To use homelab DNS (Pi-hole/AdGuard) for all Tailscale clients:

1. Go to https://login.tailscale.com/admin/dns
2. Under "Nameservers", click "Add nameserver" > Custom.
3. Enter the Tailscale IP (100.x.y.z) of the DNS server if it has Tailscale installed, or the LAN IP if subnet routing is active.
4. Optionally enable "Override local DNS" to force all DNS through homelab.

Restricted nameservers are also supported -- route only specific domains (e.g., `*.home.local`) to your homelab DNS while using public DNS for everything else.

## ACL basics

Tailscale ACLs control which devices can talk to which. By default, all devices in a Tailnet can communicate with each other.

ACLs are configured at https://login.tailscale.com/admin/acls

Example: allow all devices to access homelab subnet, but restrict SSH:

```json
{
  "acls": [
    {
      "action": "accept",
      "src": ["*"],
      "dst": ["10.0.0.0/24:*"]
    },
    {
      "action": "accept",
      "src": ["tag:admin"],
      "dst": ["*:22"]
    }
  ],
  "tagOwners": {
    "tag:admin": ["autogroup:owner"]
  }
}
```

ACLs are optional for personal homelabs but useful if sharing access with others.

## Auth key management

Auth keys authenticate new nodes to your Tailnet without interactive login.

### Creating auth keys

1. Go to https://login.tailscale.com/admin/settings/keys
2. Click "Generate auth key".
3. Options:
   - **Reusable**: Can be used by multiple nodes. Good for automation.
   - **Ephemeral**: Nodes auto-deregister when they go offline. Good for disposable containers.
   - **Pre-approved**: Skips admin approval for subnet routes (if configured in ACLs).
   - **Expiration**: Set a reasonable expiry (default 90 days).

### Recommended combinations

| Use case | Reusable | Ephemeral | Pre-approved |
|---|---|---|---|
| Permanent homelab host | No | No | Yes |
| Automated deployment (Ansible, etc.) | Yes | No | Yes |
| Disposable containers / CI | Yes | Yes | No |
| One-time manual setup | No | No | No |

### Using auth keys

Host install:
```
ssh.exec(host="<DOCKER_HOST>", command="tailscale up --authkey=<TS_AUTHKEY> --advertise-routes=<LAN_CIDR>")
```

Container (via environment variable):
```yaml
environment:
  - TS_AUTHKEY=<TS_AUTHKEY>
```

## Tailscale on Proxmox LXC

A common homelab pattern is running Tailscale inside an LXC container on Proxmox. This requires:

1. Enable TUN device in LXC config (`/etc/pve/lxc/<ID>.conf`):
   ```
   lxc.cdev.allow: c 10:200 rwm
   lxc.mount.entry: /dev/net/tun dev/net/tun none bind,create=file
   ```
2. Enable nesting if needed: `features: nesting=1`
3. Install Tailscale inside the LXC container normally.
4. Enable IP forwarding inside the container.

## Verifying Tailscale status

```
ssh.exec(host="<DOCKER_HOST>", command="tailscale status")
```

Shows all nodes in the Tailnet, their IPs, and online/offline status.

```
ssh.exec(host="<DOCKER_HOST>", command="tailscale netcheck")
```

Shows connectivity info: DERP relay latency, UDP connectivity, nearest relay server. Useful for diagnosing slow connections.

```
ssh.exec(host="<DOCKER_HOST>", command="tailscale ping <TARGET_NODE>")
```

Pings another Tailscale node directly. Shows whether traffic is going peer-to-peer or via DERP relay.

## Troubleshooting

For Tailscale subnet router troubleshooting (routes not advertising, connectivity failures after approval, debugging packet flow), see the **`tailscale-subnet-router` skill**. That skill covers detailed diagnosis workflows that are not duplicated here.

Common quick checks:

| Check | Command |
|---|---|
| Is tailscaled running? | `systemctl status tailscaled` |
| What routes are advertised? | `tailscale status --json \| jq '.Self.AllowedIPs'` |
| Is IP forwarding on? | `sysctl net.ipv4.ip_forward` |
| Is the node connected? | `tailscale status` (look for "idle" or "active") |
| Is traffic relayed or direct? | `tailscale ping <node>` |
