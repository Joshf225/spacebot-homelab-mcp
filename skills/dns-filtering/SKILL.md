---
name: dns-filtering
description: Deploy and configure DNS-based ad/tracker blocking with Pi-hole or AdGuard Home. Covers decision criteria, Docker deployment with macvlan networking, upstream DNS, blocklist management, local DNS records, DHCP integration, conditional forwarding, VPN DNS, redundancy, and common failure diagnosis.
version: 1.0.0
---

# DNS Filtering Deployment

## Purpose

Use this skill when a Spacebot agent needs to deploy, configure, or troubleshoot a network-wide DNS filtering solution using either:

- **Pi-hole** -- the classic DNS sinkhole. Mature ecosystem, extensive community blocklists, FTL DNS engine, web UI with detailed query stats, API for automation, optional DHCP server.
- **AdGuard Home** -- modern alternative. Built-in DNS-over-HTTPS/TLS, YAML-based configuration, per-client filtering rules, built-in DHCP server, DNS rewrites for local records, cleaner UI.

This playbook separates commonly conflated areas:

- **Pi-hole vs AdGuard Home** -- different tools with different strengths; the choice depends on user needs and experience.
- **Container networking vs DNS networking** -- how the container connects to Docker is separate from how LAN clients reach the DNS server.
- **DNS server deployment vs network integration** -- getting the container running is step one; making all clients use it is step two.
- **Blocking vs local resolution** -- ad-blocking and local DNS records (e.g., `jellyfin.home.lan`) are separate functions that both tools handle.

## When to invoke this skill

- User wants network-wide ad/tracker blocking.
- User wants to deploy Pi-hole or AdGuard Home.
- User needs local DNS records for homelab services.
- User wants DNS-over-HTTPS or DNS-over-TLS for upstream queries.
- User has port 53 conflicts (systemd-resolved or another DNS service).
- User wants DNS filtering accessible over VPN (Tailscale, WireGuard).
- User needs redundant DNS (primary + secondary on different hosts).
- Existing Pi-hole or AdGuard Home has configuration or resolution issues.
- User wants conditional forwarding or split DNS for a local domain.

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
| `<DOCKER_HOST>` | Docker host config name in Spacebot | `dns1` |
| `<COMPOSE_DIR>` | Directory where compose file lives | `/opt/docker/dns-filtering` |
| `<DNS_IP>` | Dedicated IP for the DNS container (macvlan) | `10.0.0.2` |
| `<LAN_SUBNET>` | LAN subnet in CIDR notation | `10.0.0.0/24` |
| `<LAN_GATEWAY>` | LAN gateway (router IP) | `10.0.0.1` |
| `<HOST_INTERFACE>` | Host network interface name | `eth0` |
| `<TZ>` | Timezone | `America/New_York` |
| `<WEBPASSWORD>` | Pi-hole admin password | (user-provided) |
| `<LOCAL_DOMAIN>` | Local DNS domain suffix | `home.lan` |

## Decision tree: Pi-hole vs AdGuard Home

### Choose Pi-hole if:

- User already has Pi-hole experience or an existing deployment.
- User wants the largest community blocklist ecosystem.
- User wants the FTL DNS engine with detailed long-term query statistics.
- User prefers `pihole` CLI tools for scripting and automation.
- User needs tight integration with third-party tools that expect the Pi-hole API (e.g., Home Assistant, Grafana dashboards).

### Choose AdGuard Home if:

- This is a fresh deployment with no existing preference.
- User wants built-in DNS-over-HTTPS / DNS-over-TLS without extra components.
- User wants per-client filtering rules (different blocklists for different devices).
- User prefers YAML-based configuration that can be version-controlled.
- User wants built-in DNS rewrites for local records (no dnsmasq config files).
- User wants a simpler, more modern setup experience.

### General recommendation:

Both are excellent. For new setups, **AdGuard Home** is the default recommendation due to simpler configuration, built-in encrypted DNS, and per-client rules. Recommend **Pi-hole** when the user specifically asks for it or has an existing investment in the Pi-hole ecosystem.

## High-confidence lessons learned

### 1. Port 53 conflict -- the most common deployment blocker

On Linux hosts, `systemd-resolved` listens on `127.0.0.53:53` by default and often binds to `0.0.0.0:53`, preventing any other process from using port 53.

**Detection:**
```
ssh.exec(host="<DOCKER_HOST>", command="ss -tlnp | grep ':53 '")
ssh.exec(host="<DOCKER_HOST>", command="systemctl is-active systemd-resolved")
```

**Resolution options (pick one):**
- **Best: Use macvlan networking** -- the DNS container gets its own IP, so port 53 on that IP is unused. No need to touch systemd-resolved at all. See lesson #2.
- **Disable the stub listener only** -- preserves systemd-resolved for the host but frees port 53:
  ```
  ssh.exec(host="<DOCKER_HOST>", command="sed -i 's/#DNSStubListener=yes/DNSStubListener=no/' /etc/systemd/resolved.conf && systemctl restart systemd-resolved")
  ```
- **Disable systemd-resolved entirely** -- simplest but the host loses its default DNS. Only do this if the host will use the Pi-hole/AdGuard container for its own DNS:
  ```
  ssh.exec(host="<DOCKER_HOST>", command="systemctl disable --now systemd-resolved && rm /etc/resolv.conf && echo 'nameserver 1.1.1.1' > /etc/resolv.conf")
  ```

**Warning:** Do not disable systemd-resolved on a remote-only machine without first verifying you have an alternative DNS path. You could lose DNS resolution and lock yourself out of SSH.

### 2. Macvlan networking is the recommended approach

Giving the DNS container its own LAN IP via a macvlan Docker network is the cleanest deployment model:

- No port 53 conflicts with the host.
- Clients point directly to the container's dedicated IP.
- The container appears as a standalone device on the LAN.
- Works identically for Pi-hole and AdGuard Home.

**Tradeoff:** The Docker host cannot directly reach the macvlan container's IP without a bridge shim interface. This is a Docker limitation, not a macvlan limitation. See `references/network-integration.md` for the shim setup.

Create the macvlan network:
```
ssh.exec(host="<DOCKER_HOST>", command="docker network create -d macvlan --subnet=<LAN_SUBNET> --gateway=<LAN_GATEWAY> -o parent=<HOST_INTERFACE> dns_macvlan")
```

### 3. Two instances for redundancy -- DNS is critical infrastructure

DNS is the single most important network service. If it goes down, nothing resolves and users perceive "the internet is down."

Deploy a primary and secondary instance on different Docker hosts. Configure both IPs in DHCP so clients have automatic failover. The two instances do not need to sync -- they operate independently with the same blocklists and upstream config.

If only one Docker host is available, run two containers on the same host with different macvlan IPs. This protects against container crashes but not host failure.

### 4. DHCP integration -- how clients discover the DNS server

Three approaches, ordered by recommendation:

**(a) Set DNS in router DHCP settings (recommended):**
- Log in to router admin, find DHCP settings, set primary DNS to `<DNS_IP>`.
- Optionally set secondary DNS to a second Pi-hole/AdGuard instance or a fallback like `1.1.1.1`.
- All DHCP clients automatically receive the new DNS on lease renewal.
- Safest because reverting is a single router setting change.

**(b) Let Pi-hole/AdGuard be the DHCP server:**
- Disable DHCP on the router, enable DHCP in Pi-hole/AdGuard.
- Gives more control (per-device static leases, hostnames in DNS).
- Riskier: if the DNS container goes down, no new DHCP leases are issued.
- Only recommended if the user has a specific need for it.

**(c) Per-device DNS configuration:**
- Set DNS manually on individual devices.
- Useful for testing or for devices that should bypass filtering.
- Does not scale.

After changing DHCP settings, force clients to renew their lease or wait for expiry.

### 5. Upstream DNS selection

Configure at least two upstream DNS providers for reliability:

| Provider | Primary | Secondary | Notes |
|----------|---------|-----------|-------|
| Cloudflare | `1.1.1.1` | `1.0.0.1` | Fast, privacy-focused |
| Quad9 | `9.9.9.9` | `149.112.112.112` | Malware blocking built-in |
| Google | `8.8.8.8` | `8.8.4.4` | Reliable, some privacy concerns |

For maximum privacy, run **Unbound** as a local recursive resolver. This eliminates third-party upstream DNS entirely -- queries go directly to authoritative nameservers. See `references/network-integration.md` for a compose snippet.

AdGuard Home supports DNS-over-HTTPS and DNS-over-TLS upstream natively:
- `https://dns.cloudflare.com/dns-query`
- `tls://1dot1dot1dot1.cloudflare-dns.com`

Pi-hole can use DoH/DoT via `cloudflared` or `unbound` as a sidecar.

### 6. Blocklists -- start conservative, expand as needed

**Default lists are good enough for most users.** Both Pi-hole and AdGuard Home ship with sensible defaults.

Recommended additions:
- StevenBlack's unified hosts list (ads + malware): `https://raw.githubusercontent.com/StevenBlack/hosts/master/hosts`
- OISD blocklist (curated, low false positives): `https://big.oisd.nl/`

Do not add every blocklist found online. More lists means:
- More false positives (legitimate sites blocked)
- More maintenance (whitelisting requests)
- Marginal improvement in actual blocking

When a user reports a broken site, check the query log to identify which list blocked which domain, then whitelist that domain.

### 7. Local DNS records for homelab services

Both tools support custom DNS records, which is one of their most valuable homelab features beyond ad-blocking.

**AdGuard Home:** Use DNS Rewrites (Filters > DNS Rewrites). Add entries like:
- `jellyfin.home.lan` -> `10.0.0.50`
- `sonarr.home.lan` -> `10.0.0.50`

**Pi-hole:** Add entries to a custom dnsmasq config file:
```
ssh.exec(host="<DOCKER_HOST>", command="echo 'address=/jellyfin.home.lan/10.0.0.50' >> /opt/docker/dns-filtering/pihole/dnsmasq.d/05-custom.conf")
```
Then restart Pi-hole DNS: `docker exec pihole pihole restartdns`

This eliminates the need to edit `/etc/hosts` on every device and works automatically for all LAN clients.

### 8. Conditional forwarding for local domain resolution

If a local domain (e.g., devices registered via router DHCP as `device.home.lan`) should resolve via the router's built-in DNS, configure conditional forwarding:

**Pi-hole:** Settings > DNS > Conditional Forwarding. Set the local network CIDR and router IP.

**AdGuard Home:** DNS Settings > Upstream DNS > add a domain-specific upstream:
```
[/home.lan/]10.0.0.1
```
This sends queries for `*.home.lan` to the router at `10.0.0.1` while all other queries go through normal upstream + filtering.

### 9. VPN + DNS for ad-blocking on the go

If the homelab runs Tailscale or WireGuard (see vpn-access skill), point the VPN's DNS settings to the Pi-hole/AdGuard IP for ad-blocking outside the home network.

**Tailscale:** In the Tailscale admin console, set DNS to the Pi-hole/AdGuard Tailscale IP or LAN IP (if subnet routing is enabled).

**WireGuard:** In the client config, set `DNS = <DNS_IP>`.

Ensure the VPN can reach the DNS container's IP. If using macvlan, the DNS IP must be routable from the VPN subnet.

### 10. Gravity / filter updates

**Pi-hole:** Gravity (the blocklist database) must be updated periodically:
```
ssh.exec(host="<DOCKER_HOST>", command="docker exec pihole pihole -g")
```
Pi-hole includes a cron job for this by default (weekly). Verify:
```
ssh.exec(host="<DOCKER_HOST>", command="docker exec pihole crontab -l")
```

**AdGuard Home:** Filters auto-update on a configurable interval (default: 24 hours). Check Settings > Filters > Filter update interval.

## Service configuration details

### Pi-hole

| Setting | Value |
|---------|-------|
| Image | `pihole/pihole:latest` |
| Default port | 53 (DNS), 80 (Web UI) |
| Config volume | `/opt/docker/dns-filtering/pihole/etc-pihole` mapped to `/etc/pihole` |
| Dnsmasq volume | `/opt/docker/dns-filtering/pihole/dnsmasq.d` mapped to `/etc/dnsmasq.d` |
| Environment | `TZ`, `WEBPASSWORD`, `FTLCONF_LOCAL_IPV4` (set to container IP) |

After deployment:
1. Access web UI at `http://<DNS_IP>/admin`
2. Log in with `WEBPASSWORD`
3. Go to Settings > DNS, configure upstream DNS servers
4. Go to Adlists, add additional blocklists if desired
5. Go to Local DNS > DNS Records to add homelab entries
6. Verify blocking: `ssh.exec(host="<DOCKER_HOST>", command="docker exec pihole nslookup ads.google.com localhost")`

### AdGuard Home

| Setting | Value |
|---------|-------|
| Image | `adguard/adguardhome:latest` |
| Default port | 53 (DNS), 3000 (initial setup), 80 (Web UI after setup) |
| Work volume | `/opt/docker/dns-filtering/adguard/work` mapped to `/opt/adguardhome/work` |
| Config volume | `/opt/docker/dns-filtering/adguard/conf` mapped to `/opt/adguardhome/conf` |

After deployment:
1. Access setup wizard at `http://<DNS_IP>:3000`
2. Complete initial setup (set admin credentials, listening interfaces)
3. Go to Settings > DNS Settings, configure upstream DNS
4. Go to Filters > DNS Blocklists, add lists if desired
5. Go to Filters > DNS Rewrites for local DNS records
6. Verify blocking: `ssh.exec(host="<DOCKER_HOST>", command="docker exec adguardhome nslookup ads.google.com 127.0.0.1")`

## Safety rules

1. **Always have a fallback DNS before switching all clients.** Keep the router's original DNS settings noted. If the DNS container fails, clients need a way to resolve.

2. **Test with ONE device first.** Manually set DNS on a single device to `<DNS_IP>` and verify resolution and blocking before changing DHCP for the entire network.

3. **Use `confirm_operation` before stopping DNS containers.** Stopping a DNS server that the entire network depends on causes an immediate network-wide outage from the user's perspective.

4. **Back up before making changes.**
   - Pi-hole: `ssh.exec(host="<DOCKER_HOST>", command="docker exec pihole pihole -a -t")`
   - AdGuard Home: `ssh.exec(host="<DOCKER_HOST>", command="cp /opt/docker/dns-filtering/adguard/conf/AdGuardHome.yaml /opt/docker/dns-filtering/adguard/conf/AdGuardHome.yaml.bak")`

5. **Do not disable systemd-resolved on a remote-only machine without testing.** If DNS breaks on the host, you lose SSH access if hostname-based SSH config is used. Always verify you can reach the host by IP.

6. **Never run `docker compose down -v` on the DNS stack.** This deletes volumes containing blocklist databases, query logs, and configuration.

7. **Do not add excessive blocklists.** Start with defaults + one curated list. Aggressive blocking causes user frustration (broken websites, apps) that erodes trust in the setup.

8. **Pin image versions for stability or accept the tradeoff with `:latest`.** DNS is critical infrastructure -- unexpected breaking changes from a `:latest` pull can take down the network.

## Recommended procedural flow for agents

### Full deployment

1. **Gather requirements:**
   - Pi-hole or AdGuard Home? (default: AdGuard Home for new setups)
   - Target Docker host
   - Networking: macvlan (recommended) or bridge?
   - If macvlan: desired DNS container IP, LAN subnet, gateway, host interface
   - Timezone
   - Upstream DNS preference (Cloudflare, Quad9, Google, Unbound)
   - Any local DNS records needed?

2. **Check for port 53 conflicts (bridge mode only):**
   ```
   ssh.exec(host="<DOCKER_HOST>", command="ss -tlnp | grep ':53 '")
   ```
   If systemd-resolved is listening, resolve per lesson #1. Skip this step for macvlan.

3. **Create directory structure:**
   For Pi-hole:
   ```
   ssh.exec(host="<DOCKER_HOST>", command="mkdir -p /opt/docker/dns-filtering/pihole/{etc-pihole,dnsmasq.d}")
   ```
   For AdGuard Home:
   ```
   ssh.exec(host="<DOCKER_HOST>", command="mkdir -p /opt/docker/dns-filtering/adguard/{work,conf}")
   ```

4. **Create macvlan network (if using macvlan):**
   ```
   ssh.exec(host="<DOCKER_HOST>", command="docker network create -d macvlan --subnet=<LAN_SUBNET> --gateway=<LAN_GATEWAY> -o parent=<HOST_INTERFACE> dns_macvlan")
   ```

5. **Upload compose file:**
   Generate or use the reference compose file (see `references/compose-example.md`). Upload:
   ```
   ssh.upload(host="<DOCKER_HOST>", local_path="./docker-compose.yml", remote_path="/opt/docker/dns-filtering/docker-compose.yml")
   ```

6. **Deploy:**
   ```
   ssh.exec(host="<DOCKER_HOST>", command="docker compose up -d", cwd="/opt/docker/dns-filtering")
   ```

7. **Verify container is running:**
   ```
   ssh.exec(host="<DOCKER_HOST>", command="docker compose ps", cwd="/opt/docker/dns-filtering")
   ```

8. **Test DNS resolution from the host (macvlan requires shim, see references):**
   ```
   ssh.exec(host="<DOCKER_HOST>", command="dig @<DNS_IP> example.com +short")
   ssh.exec(host="<DOCKER_HOST>", command="dig @<DNS_IP> ads.google.com +short")
   ```
   The first query should return an IP. The second should return `0.0.0.0` or NXDOMAIN (blocked).

9. **Configure upstream DNS, blocklists, and local records** via the web UI or API.

10. **Test with a single client device** by manually setting its DNS to `<DNS_IP>`.

11. **Update router DHCP** to set `<DNS_IP>` as the primary DNS server for all clients.

12. **Verify network-wide:** After clients renew DHCP leases, confirm filtering is working from multiple devices.

### Deploying redundant secondary instance

1. Repeat steps 1-8 on a second Docker host with a different macvlan IP.
2. Apply the same blocklists and upstream DNS configuration.
3. Add local DNS records to match the primary instance.
4. In router DHCP, set primary DNS = first instance IP, secondary DNS = second instance IP.
5. Test failover by stopping the primary and verifying resolution continues via the secondary.

### Updating the deployment

1. Back up configuration (see Safety Rules).
2. Pull and recreate:
   ```
   ssh.exec(host="<DOCKER_HOST>", command="docker compose pull && docker compose up -d", cwd="/opt/docker/dns-filtering")
   ```
3. Verify DNS resolution still works:
   ```
   ssh.exec(host="<DOCKER_HOST>", command="dig @<DNS_IP> example.com +short")
   ```
4. Check container logs for errors:
   ```
   docker.container.logs(host="<DOCKER_HOST>", name="pihole", tail=30)
   ```

## Fast diagnosis cheatsheet

| Symptom | Most likely cause | Fix |
|---|---|---|
| Port 53 already in use | systemd-resolved or another DNS service | Use macvlan to avoid conflict, or disable stub listener (see lesson #1) |
| Clients not using DNS server | DHCP not updated, or client has hardcoded DNS (e.g., Android uses Google DNS) | Update router DHCP settings; check per-device DNS overrides |
| Too many false positives (sites broken) | Overly aggressive blocklists | Check query log for blocked domain; whitelist it; remove aggressive lists |
| Local names not resolving (e.g., `jellyfin.home.lan`) | No local DNS records or conditional forwarding configured | Add DNS records (Pi-hole: custom dnsmasq; AdGuard: DNS rewrites) |
| DNS queries slow | Too many blocklists, slow upstream DNS, or container resource-starved | Reduce blocklist count; switch upstream; check container CPU/memory |
| Pi-hole gravity update fails | Disk full, network issue, or blocklist URL down | Check disk space; test URL manually; check Pi-hole logs |
| AdGuard Home YAML corrupt after manual edit | Syntax error in `AdGuardHome.yaml` | Restore from backup; validate YAML syntax before applying |
| DNS not working over VPN | VPN DNS settings not pointing to Pi-hole/AdGuard IP, or IP not routable from VPN | Set VPN DNS to `<DNS_IP>`; ensure subnet routing or Tailscale ACLs allow access |
| Host cannot reach macvlan container | Docker macvlan limitation: host cannot reach its own macvlan containers | Create bridge shim interface (see `references/network-integration.md`) |
| Container starts but no DNS response | Container listening on wrong interface, or firewall blocking port 53 | Check container logs; verify listening address; check `ufw` / `iptables` rules |
| AdGuard Home setup wizard keeps appearing | Setup not completed, or config volume not persisted | Complete the wizard; verify volume mount is correct and writable |
| Secondary DNS has different blocking behavior | Blocklists or settings not synchronized | Manually sync blocklist URLs and settings; consider scripting the sync |

## Decision trees

### Macvlan vs bridge networking?

**Macvlan (recommended):**
- DNS container gets its own LAN IP.
- No port 53 conflicts.
- Clients point directly to a dedicated IP.
- Requires: knowledge of LAN subnet, available IP, host interface name.
- Tradeoff: host-to-container communication requires a bridge shim.

**Bridge:**
- Simpler Docker networking.
- Port 53 mapped to host IP.
- Requires resolving port 53 conflicts first.
- Host can reach container at `127.0.0.1`.
- Tradeoff: port 53 conflict resolution can be fragile; clients point to the Docker host IP.

### DHCP: router-managed vs Pi-hole/AdGuard-managed?

**Router-managed DHCP (recommended):**
- Safer -- if DNS container goes down, DHCP still works.
- Simpler -- one less thing for the container to manage.
- Revert is easy -- change one router setting.

**Container-managed DHCP:**
- More features -- static leases, hostname registration, PXE boot options.
- Riskier -- container outage means no new DHCP leases.
- Only recommended if the user has a specific need (e.g., PXE boot, detailed per-device control).

### Upstream: third-party vs recursive (Unbound)?

**Third-party upstream (Cloudflare, Quad9, Google):**
- Simpler setup, faster initial resolution (cached at provider).
- Privacy depends on the provider.
- Recommended for most users.

**Unbound recursive resolver:**
- Maximum privacy -- no third party sees your queries.
- Slightly slower for first resolution (no shared cache).
- More complex setup (additional container).
- Recommended for privacy-focused users. See `references/network-integration.md`.

## References

See supporting reference docs in `references/`:

- `compose-example.md` -- Docker Compose files for Pi-hole and AdGuard Home with macvlan and bridge networking variants.
- `network-integration.md` -- Macvlan setup, router DHCP configuration, split DNS, Unbound recursive resolver, VPN integration, and redundancy patterns.
