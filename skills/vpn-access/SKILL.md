---
name: vpn-access
description: Deploy and configure VPN access to a homelab using Tailscale (mesh VPN) or self-hosted WireGuard (wg-easy). Covers subnet routing, exit nodes, split/full tunnel, DNS configuration, and client setup. For Tailscale subnet router troubleshooting, see the tailscale-subnet-router skill.
version: 1.0.0
---

# VPN Access Deployment

## Purpose

Use this skill when a Spacebot agent needs to deploy, configure, or diagnose VPN access to a homelab. This covers two approaches:

- **Tailscale** -- Zero-config mesh VPN built on WireGuard. No port forwarding required. MagicDNS. ACLs. Free for personal use (up to 100 devices). Recommended for most homelab users.
- **WireGuard (self-hosted)** -- Raw WireGuard via wg-easy Docker container. Full control. Requires port forwarding (UDP 51820). No dependency on external coordination server. Better for privacy maximalists.

These can coexist. Tailscale for daily use, self-hosted WireGuard as a backup or fallback.

This skill focuses on **deployment**. For Tailscale subnet router troubleshooting (routes not advertising, approval issues, connectivity debugging), see the `tailscale-subnet-router` skill.

## When to invoke this skill

- User wants remote access to their homelab network.
- User wants to deploy Tailscale on a homelab host or as a Docker container.
- User wants to deploy a self-hosted WireGuard server (wg-easy).
- User needs subnet routing so VPN clients can reach LAN devices.
- User wants an exit node to route all traffic through their homelab.
- User wants to use homelab DNS (Pi-hole/AdGuard) remotely via VPN.
- User needs to configure split tunnel vs full tunnel.
- User wants to generate WireGuard client configs or QR codes.
- VPN is deployed but clients cannot reach homelab resources (deployment-related, not troubleshooting -- for Tailscale troubleshooting, defer to `tailscale-subnet-router` skill).

## Available MCP tools

This skill depends on the following Spacebot MCP tools:

| Tool | Purpose | Confirmation Required |
|------|---------|----------------------|
| `docker.container.list` | List containers (optional name filter) | No |
| `docker.container.start` | Start a stopped container | No |
| `docker.container.stop` | Stop a running container | Yes |
| `docker.container.logs` | Get container logs (tail/since) | No |
| `docker.container.inspect` | Inspect container details (env vars redacted) | No |
| `docker.container.delete` | Delete a container | Yes |
| `docker.container.create` | Create a new container (ports, env, volumes, restart policy) | No |
| `docker.image.list` | List images | No |
| `docker.image.pull` | Pull an image | No |
| `docker.image.inspect` | Inspect image metadata | No |
| `docker.image.delete` | Delete an image | Yes |
| `docker.image.prune` | Remove unused images | Yes |

Additionally, the following tools are used for Compose workflows and host management:

| Tool | Purpose | Confirmation Required |
|------|---------|----------------------|
| `ssh.exec` | Run commands on remote Docker hosts (compose up/down, system commands) | Pattern-based |
| `ssh.upload` | Upload files (compose files, configs) to Docker hosts | No |
| `ssh.download` | Download files from Docker hosts | No |
| `confirm_operation` | Confirm a destructive operation with a token | N/A |
| `audit.log` | Log an operation for audit trail | No |

## Environment variables

| Placeholder | Meaning | Example |
|---|---|---|
| `<DOCKER_HOST>` | Docker host config name in Spacebot | `vpn1` |
| `<COMPOSE_DIR>` | Directory where compose file lives | `/opt/docker/vpn` |
| `<LAN_CIDR>` | Homelab LAN subnet | `10.0.0.0/24` |
| `<WG_HOST>` | Public IP or DDNS hostname for WireGuard | `vpn.example.com` |
| `<WG_PORT>` | WireGuard listen port | `51820` |
| `<WG_PASSWORD>` | wg-easy web UI password (bcrypt hash) | `$2a$12$...` |
| `<TS_AUTHKEY>` | Tailscale auth key for headless setup | `tskey-auth-...` |
| `<DNS_SERVER>` | Homelab DNS server IP (Pi-hole/AdGuard) | `10.0.0.53` |
| `<TZ>` | Timezone | `America/New_York` |

## Decision tree: WireGuard vs Tailscale

### Use Tailscale if:

- You want the simplest possible setup (no port forwarding, no key management).
- You need mesh connectivity between multiple devices (laptop, phone, cloud VMs).
- You want MagicDNS for human-readable names.
- You want ACLs to control which devices can reach what.
- You are comfortable depending on Tailscale's coordination server (traffic is still peer-to-peer and encrypted).
- Free tier (up to 100 devices) is sufficient.

### Use self-hosted WireGuard if:

- You want zero dependency on any external service.
- You want full control over the VPN server and all keys.
- You can forward a UDP port on your router.
- You have a static IP or DDNS hostname.
- Privacy is the top priority -- no third-party coordination server.

### Use both if:

- Tailscale for daily convenience, WireGuard as a fallback.
- Different users prefer different approaches.
- You want redundancy in case Tailscale has an outage.

### Recommendation:

Start with Tailscale. It works in minutes with no networking changes. Add self-hosted WireGuard later if you want a fallback or want to eliminate the Tailscale dependency.

## High-confidence lessons learned

### 1. Tailscale subnet router -- the key to accessing LAN resources

Tailscale by itself only connects Tailscale-installed devices. To reach devices on your homelab LAN that do not have Tailscale installed (NAS, printers, IoT devices, other containers), you need a **subnet router**.

Deploy Tailscale on a host (or container) that can reach your LAN, then advertise the LAN routes:

```
tailscale up --advertise-routes=<LAN_CIDR> --accept-routes
```

After advertising, you must **approve the routes in the Tailscale admin console** (admin.tailscale.com > Machines > your node > Edit route settings). Routes are not active until approved.

Once approved, all Tailscale clients can reach homelab IPs directly.

### 2. Tailscale exit node -- route all traffic through homelab

Enable exit node on the homelab Tailscale host:

```
tailscale up --advertise-routes=<LAN_CIDR> --advertise-exit-node --accept-routes
```

Approve the exit node in the Tailscale admin console. Clients can then toggle "Use exit node" to route ALL traffic through the homelab. This is useful for:

- Using homelab Pi-hole/AdGuard DNS when away from home.
- Appearing to browse from your home IP.
- Encrypting all traffic on untrusted networks.

### 3. WireGuard needs a static port forward

UDP port 51820 (default) must be forwarded from your router to the WireGuard server. There is no way around this for self-hosted WireGuard. If your ISP uses CGNAT, WireGuard will not work without a VPS relay or similar workaround. Tailscale handles NAT traversal automatically.

### 4. wg-easy is the best WireGuard UI

The `ghcr.io/wg-easy/wg-easy` Docker image provides a web UI for managing WireGuard peers. It handles key generation, config creation, and QR code display. Much easier than managing config files manually.

### 5. Key management matters for WireGuard

WireGuard keys are just files. The server private key is the root of trust. Losing it means regenerating ALL client configurations. Back up the entire wg-easy config directory:

```
ssh.exec(host="<DOCKER_HOST>", command="tar czf /opt/docker/vpn/wg-backup-$(date +%Y%m%d).tar.gz /opt/docker/vpn/wg-easy/config")
```

### 6. DNS in VPN -- ad-blocking on the go

Configure VPN clients to use homelab DNS for ad-blocking remotely:

- **WireGuard**: Set `DNS = <DNS_SERVER>` in client config. wg-easy supports setting a default DNS for all clients via `WG_DEFAULT_DNS`.
- **Tailscale**: Configure DNS in admin console > DNS > Add nameserver. Point to your homelab DNS IP (must be reachable via subnet route or Tailscale IP). Enable "Override local DNS" if you want to force it.

### 7. Split tunnel vs full tunnel

**Split tunnel** only routes homelab-destined traffic through the VPN. Better performance for general browsing.

**Full tunnel** routes ALL traffic through the VPN. Better privacy on untrusted networks. Uses homelab DNS for everything.

Configuration:

- **WireGuard split tunnel**: `AllowedIPs = <LAN_CIDR>` in client config.
- **WireGuard full tunnel**: `AllowedIPs = 0.0.0.0/0, ::/0` in client config.
- **Tailscale split tunnel**: Default behavior (only Tailscale network traffic is routed).
- **Tailscale full tunnel**: Enable exit node on server, select it on client.

### 8. Container vs host install for Tailscale

**Host install** (apt, brew, etc.) is simpler and more reliable. The Tailscale daemon runs natively and has full access to networking.

**Container install** works but requires extra capabilities:

```yaml
cap_add:
  - NET_ADMIN
  - NET_RAW
devices:
  - /dev/net/tun:/dev/net/tun
```

Use host install unless you have a specific reason for containerization (e.g., LXC container on Proxmox, isolated environment).

### 9. Tailscale auth keys for automation

For headless/automated deployments, use `TS_AUTHKEY` environment variable instead of interactive login:

```
TS_AUTHKEY=tskey-auth-xxxxx tailscale up --advertise-routes=<LAN_CIDR>
```

Create auth keys in Tailscale admin console > Settings > Keys. Use **reusable** and **ephemeral** keys for automation. Ephemeral keys auto-remove the node when it goes offline, which is useful for disposable containers.

### 10. IP forwarding is required for subnet routing

Both Tailscale subnet routing and WireGuard server functionality require IP forwarding on the host:

```
ssh.exec(host="<DOCKER_HOST>", command="sysctl net.ipv4.ip_forward")
```

If not enabled:

```
ssh.exec(host="<DOCKER_HOST>", command="echo 'net.ipv4.ip_forward=1' >> /etc/sysctl.d/99-vpn.conf && sysctl -p /etc/sysctl.d/99-vpn.conf")
```

Without this, the host will not forward packets between the VPN tunnel and the LAN, and subnet routing will silently fail.

### 11. CGNAT blocks self-hosted WireGuard

If your ISP uses Carrier-Grade NAT (CGNAT), you do not have a publicly routable IP address. Port forwarding is impossible. WireGuard clients cannot reach your server.

To detect CGNAT:
```
ssh.exec(host="<DOCKER_HOST>", command="curl -s https://api.ipify.org && echo '' && ip route get 1.1.1.1 | awk '{print $7}'")
```

If the public IP (from ipify) differs from your router's WAN IP, you are behind CGNAT. Options:
- Use Tailscale instead (handles NAT traversal automatically).
- Rent a cheap VPS and use it as a WireGuard relay (reverse proxy UDP traffic).
- Ask your ISP for a static/public IP (some ISPs offer this for a fee).

### 12. Tailscale on multiple homelab hosts

You can install Tailscale on multiple homelab hosts, but only ONE should be the subnet router for a given CIDR. If two nodes advertise the same subnet, Tailscale will use one as primary and the other as failover (if HA subnet routers are enabled on your plan).

For free plans, pick the most stable/always-on host as the subnet router. Other hosts get their own Tailscale IPs (100.x.y.z) for direct access but do not need to advertise routes.

### 13. Firewall considerations for WireGuard

On hosts running `ufw` or `firewalld`, you must explicitly allow the WireGuard port:

```
# ufw
ssh.exec(host="<DOCKER_HOST>", command="ufw allow 51820/udp")

# firewalld
ssh.exec(host="<DOCKER_HOST>", command="firewall-cmd --permanent --add-port=51820/udp && firewall-cmd --reload")
```

Also allow forwarding between interfaces if using strict firewall rules. Tailscale generally manages its own firewall rules via `iptables` and does not require manual firewall configuration.

### 14. Monitoring VPN connectivity

For WireGuard, check the latest handshake time:
```
ssh.exec(host="<DOCKER_HOST>", command="docker exec wg-easy wg show wg0 latest-handshakes")
```

A handshake older than 2-3 minutes means the peer is likely offline. WireGuard does not maintain a persistent connection -- handshakes occur when traffic flows or keepalives fire.

For Tailscale:
```
ssh.exec(host="<DOCKER_HOST>", command="tailscale status")
```

Shows each node as "active", "idle", or "offline". Use `tailscale ping <node>` to test direct connectivity vs DERP relay.

## Safety rules

1. **Never expose WireGuard private keys in logs or chat.** The server private key and client private keys are secrets. If they appear in logs, redact them. Never paste them in responses.

2. **Use `confirm_operation` before modifying VPN server config.** Changing WireGuard server config or restarting the container disconnects all connected clients.

3. **Back up WireGuard keys and config before changes.** Losing the server private key requires regenerating all client configs.

4. **Use ephemeral + reusable Tailscale auth keys for automation.** Do not use long-lived single-use keys. Ephemeral keys clean up after themselves. Reusable keys can be used across multiple deployments.

5. **Do not run both Tailscale and WireGuard on the same interface without understanding routing.** Both modify the routing table. If both are active on the same host, ensure their routes do not conflict. In practice, they coexist fine if WireGuard uses a dedicated interface (wg0) and Tailscale uses its own (tailscale0).

6. **Use `confirm_operation` before enabling exit node.** This changes how all traffic is routed for clients that select the exit node.

7. **Do not disable IP forwarding on a host running VPN.** This breaks all subnet routing instantly.

## Service configuration details

### wg-easy (WireGuard)

| Setting | Value |
|---------|-------|
| Image | `ghcr.io/wg-easy/wg-easy:latest` |
| WireGuard port | 51820/udp |
| Web UI port | 51821/tcp |
| Config path | `/opt/docker/vpn/wg-easy/config` mapped to `/etc/wireguard` |
| VPN subnet | `10.8.0.0/24` (default) |
| Capabilities | `NET_ADMIN`, `SYS_MODULE` |

After deployment:
1. Access web UI at `http://<DOCKER_HOST>:51821`
2. Log in with configured password
3. Create client configs (each gets a unique key pair)
4. Download config or scan QR code on mobile

### Tailscale (container)

| Setting | Value |
|---------|-------|
| Image | `tailscale/tailscale:latest` |
| Network mode | `host` (required for subnet routing) |
| State path | `/opt/docker/vpn/tailscale/state` mapped to `/var/lib/tailscale` |
| Device | `/dev/net/tun` required |
| Capabilities | `NET_ADMIN`, `NET_RAW` |

After deployment:
1. Check `tailscale status` for node registration
2. Approve subnet routes in admin console
3. Verify clients can reach LAN IPs

### Tailscale (host install)

| Setting | Value |
|---------|-------|
| Service | `tailscaled` (systemd) |
| Config | Managed by `tailscale up` flags |
| State | `/var/lib/tailscale/` |

After install:
1. Run `tailscale up` with desired flags
2. Approve routes in admin console
3. Service persists across reboots via systemd

## Recommended procedural flow for agents

### Tailscale deployment

1. **Gather requirements:**
   - Target Docker host
   - LAN CIDR to advertise (e.g., `10.0.0.0/24`)
   - Want exit node? (yes/no)
   - Homelab DNS server IP (if using Pi-hole/AdGuard)
   - Container or host install?
   - Auth key available? (if not, guide user to create one)

2. **Verify IP forwarding:**
   ```
   ssh.exec(host="<DOCKER_HOST>", command="sysctl net.ipv4.ip_forward")
   ```
   Enable if not set:
   ```
   ssh.exec(host="<DOCKER_HOST>", command="echo 'net.ipv4.ip_forward=1' >> /etc/sysctl.d/99-vpn.conf && sysctl -p /etc/sysctl.d/99-vpn.conf")
   ```

3. **Install Tailscale (host method):**
   ```
   ssh.exec(host="<DOCKER_HOST>", command="curl -fsSL https://tailscale.com/install.sh | sh")
   ```

   Or deploy as container (see `references/tailscale-setup.md` for compose file).

4. **Authenticate and configure:**
   ```
   ssh.exec(host="<DOCKER_HOST>", command="tailscale up --advertise-routes=<LAN_CIDR> --accept-routes --authkey=<TS_AUTHKEY>")
   ```
   Add `--advertise-exit-node` if exit node is desired.

5. **Approve routes in Tailscale admin console:**
   - Instruct user to visit admin.tailscale.com > Machines > find the node > Edit route settings.
   - Approve advertised subnets.
   - Approve exit node if applicable.

6. **Configure DNS (optional):**
   - Instruct user to visit admin.tailscale.com > DNS.
   - Add homelab DNS server as a nameserver.
   - Optionally enable "Override local DNS".

7. **Install Tailscale on client devices:**
   - Desktop: download from tailscale.com
   - iOS/Android: install from app store
   - Connect to the same Tailnet.

8. **Verify connectivity:**
   ```
   ssh.exec(host="<DOCKER_HOST>", command="tailscale status")
   ```
   From a client, ping a homelab LAN IP to confirm subnet routing works.

### WireGuard deployment (wg-easy)

1. **Gather requirements:**
   - Target Docker host
   - Public IP or DDNS hostname (`<WG_HOST>`)
   - LAN CIDR for routing
   - Web UI password
   - DNS server for clients (homelab DNS or public)
   - Can the user forward UDP 51820 on their router?

2. **Verify port forwarding is possible:**
   If the user is behind CGNAT (no public IP), WireGuard will not work. Check:
   ```
   ssh.exec(host="<DOCKER_HOST>", command="curl -s https://api.ipify.org")
   ```
   Compare with the router's WAN IP. If they differ, likely CGNAT -- recommend Tailscale instead.

3. **Verify IP forwarding:**
   ```
   ssh.exec(host="<DOCKER_HOST>", command="sysctl net.ipv4.ip_forward")
   ```
   Enable if needed (same as Tailscale section above).

4. **Create directory structure:**
   ```
   ssh.exec(host="<DOCKER_HOST>", command="mkdir -p /opt/docker/vpn/wg-easy/config")
   ```

5. **Upload compose file:**
   Generate compose file (see `references/wireguard-setup.md` for template). Upload:
   ```
   ssh.upload(host="<DOCKER_HOST>", local_path="./docker-compose.yml", remote_path="/opt/docker/vpn/docker-compose.yml")
   ```

6. **Deploy:**
   ```
   ssh.exec(host="<DOCKER_HOST>", command="docker compose up -d", cwd="/opt/docker/vpn")
   ```

7. **Verify container is running:**
   ```
   ssh.exec(host="<DOCKER_HOST>", command="docker compose ps", cwd="/opt/docker/vpn")
   docker.container.logs(host="<DOCKER_HOST>", name="wg-easy", tail=30)
   ```

8. **Configure router port forwarding:**
   Instruct user to forward UDP 51820 from router WAN to the Docker host IP.

9. **Create client configs via wg-easy web UI:**
   - Access `http://<DOCKER_HOST>:51821`
   - Log in with the configured password.
   - Click "New Client" to generate a config.
   - Scan QR code on mobile or download config file for desktop.

10. **Verify connectivity:**
    - Client connects and gets a WireGuard handshake.
    - Client can ping homelab LAN IPs.
    - Check on server: `ssh.exec(host="<DOCKER_HOST>", command="docker exec wg-easy wg show")`

### Deploying both (Tailscale + WireGuard)

1. Deploy Tailscale first (primary, daily use).
2. Deploy WireGuard second (backup/fallback).
3. Ensure no routing conflicts:
   - Both can run on the same host. Tailscale uses `tailscale0` interface, WireGuard uses `wg0`.
   - If both advertise the same LAN CIDR, clients should only connect via one at a time.
4. Document which VPN to use as primary and which as fallback.

## Fast diagnosis cheatsheet

| Symptom | Most likely cause | Fix |
|---|---|---|
| Cannot connect to WireGuard from outside | Port not forwarded, firewall blocking UDP 51820 | Verify router port forward; check host firewall: `iptables -L -n \| grep 51820` |
| WireGuard handshake succeeds but no traffic | AllowedIPs misconfigured on client or server-side routing issue | Check client AllowedIPs includes target CIDR; verify IP forwarding on server |
| Tailscale subnet routes not working | Routes not approved in admin console | Approve at admin.tailscale.com > Machines > Edit route settings |
| Tailscale subnet routes approved but still no connectivity | IP forwarding not enabled on the subnet router host | Enable: `sysctl -w net.ipv4.ip_forward=1` and persist in `/etc/sysctl.d/` |
| Slow VPN speeds | MTU mismatch causing fragmentation | Lower MTU to 1280 in WireGuard config or Tailscale: `tailscale up --netfilter-mode=off` and test |
| DNS not resolving over VPN | DNS server not configured in VPN client config | Set `DNS = <DNS_SERVER>` in WireGuard; configure nameservers in Tailscale admin DNS |
| Tailscale node shows "offline" | tailscaled service not running on the host | `ssh.exec(host="<DOCKER_HOST>", command="systemctl status tailscaled")` and restart if needed |
| wg-easy web UI not accessible | Container not running or port 51821 not mapped | Check `docker compose ps`; verify port mapping in compose file |
| WireGuard client gets IP but cannot reach LAN | Server not configured to forward traffic to LAN | Verify IP forwarding; check iptables MASQUERADE rule on server |
| Tailscale "needs login" after reboot | Auth key was single-use or non-reusable | Use reusable auth key; or re-authenticate manually |
| Multiple VPN clients cannot reach each other | WireGuard peer-to-peer not configured; or Tailscale ACLs blocking | For WireGuard, add peers to each other's AllowedIPs; for Tailscale, check ACL policy |
| Connection drops intermittently | Dynamic IP changed and WG_HOST is stale | Use DDNS hostname for WG_HOST; or update client configs with new IP |

## References

See supporting reference docs in `references/`:

- `wireguard-setup.md` -- Complete wg-easy Docker Compose deployment, server configuration, client config generation, port forwarding, and DDNS setup.
- `tailscale-setup.md` -- Docker and host install methods, subnet router setup, exit node configuration, MagicDNS, custom nameservers, ACL basics, and auth key management.

For Tailscale subnet router troubleshooting (route not advertising, connectivity issues after approval, debugging steps), see the `tailscale-subnet-router` skill.
