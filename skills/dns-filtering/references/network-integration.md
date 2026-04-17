# DNS Filtering -- Network Integration

## Macvlan setup step-by-step

### 1. Identify the host network interface

```
ssh.exec(host="<DOCKER_HOST>", command="ip route | grep default")
```

The output shows the default route interface (e.g., `default via 10.0.0.1 dev eth0`). Use that interface name as `<HOST_INTERFACE>`.

### 2. Identify an available IP

Choose an IP within the LAN subnet that is:
- Outside the router's DHCP range (check router admin UI for the DHCP pool).
- Not assigned to any other device.

Common approach: if DHCP range is `10.0.0.100-10.0.0.254`, use `10.0.0.2` for the DNS container.

Verify the IP is unused:
```
ssh.exec(host="<DOCKER_HOST>", command="ping -c 1 -W 1 <DNS_IP> && echo 'IP IN USE' || echo 'IP AVAILABLE'")
```

### 3. Create the macvlan Docker network

```
ssh.exec(host="<DOCKER_HOST>", command="docker network create -d macvlan --subnet=<LAN_SUBNET> --gateway=<LAN_GATEWAY> -o parent=<HOST_INTERFACE> dns_macvlan")
```

### 4. Host-to-container communication shim

Docker hosts cannot directly communicate with their own macvlan containers. This is a known Docker limitation. To allow the host to reach the DNS container (e.g., for the host itself to use it as DNS), create a macvlan shim interface:

```
ssh.exec(host="<DOCKER_HOST>", command="ip link add dns-shim link <HOST_INTERFACE> type macvlan mode bridge")
ssh.exec(host="<DOCKER_HOST>", command="ip addr add <HOST_SHIM_IP>/32 dev dns-shim")
ssh.exec(host="<DOCKER_HOST>", command="ip link set dns-shim up")
ssh.exec(host="<DOCKER_HOST>", command="ip route add <DNS_IP>/32 dev dns-shim")
```

Where `<HOST_SHIM_IP>` is another unused IP on the LAN (e.g., `10.0.0.4`).

**Making the shim persistent across reboots:**

Create a systemd service or add to `/etc/rc.local`:
```
ssh.exec(host="<DOCKER_HOST>", command="cat > /etc/systemd/system/dns-shim.service << 'UNIT'\n[Unit]\nDescription=Macvlan shim for DNS container\nAfter=network-online.target\n\n[Service]\nType=oneshot\nRemainAfterExit=yes\nExecStart=/sbin/ip link add dns-shim link <HOST_INTERFACE> type macvlan mode bridge\nExecStart=/sbin/ip addr add <HOST_SHIM_IP>/32 dev dns-shim\nExecStart=/sbin/ip link set dns-shim up\nExecStart=/sbin/ip route add <DNS_IP>/32 dev dns-shim\nExecStop=/sbin/ip link del dns-shim\n\n[Install]\nWantedBy=multi-user.target\nUNIT")
ssh.exec(host="<DOCKER_HOST>", command="systemctl enable --now dns-shim.service")
```

### 5. Verify macvlan connectivity

From another LAN device (not the Docker host):
```
ping <DNS_IP>
dig @<DNS_IP> example.com
```

From the Docker host (after shim is set up):
```
ssh.exec(host="<DOCKER_HOST>", command="dig @<DNS_IP> example.com +short")
```

## Router DHCP configuration

The goal is to make all LAN clients use the Pi-hole/AdGuard Home IP as their DNS server automatically.

### Generic steps (applies to most routers):

1. Log in to router admin interface (typically `http://10.0.0.1` or `http://192.168.1.1`).
2. Find DHCP settings (often under LAN, Network, or DHCP).
3. Set "Primary DNS" or "DNS Server 1" to `<DNS_IP>`.
4. Optionally set "Secondary DNS" to:
   - A second Pi-hole/AdGuard instance IP (best for redundancy with filtering).
   - The router's own IP or `1.1.1.1` (fallback without filtering -- clients may bypass filtering when primary is down).
5. Save and apply.
6. Existing clients continue using old DNS until their DHCP lease renews. Force renewal:
   - Windows: `ipconfig /release && ipconfig /renew`
   - macOS: `sudo ipconfig set en0 DHCP`
   - Linux: `sudo dhclient -r && sudo dhclient`
   - Or just disconnect and reconnect to the network.

### Router-specific notes:

- **UniFi:** Settings > Networks > (network) > DHCP Name Server > set to manual, enter `<DNS_IP>`.
- **pfSense/OPNsense:** Services > DHCP Server > (interface) > DNS Servers > enter `<DNS_IP>`.
- **OpenWrt:** Network > Interfaces > LAN > DHCP Server > Advanced Settings > DHCP-Options: `6,<DNS_IP>`.
- **Consumer routers (TP-Link, ASUS, Netgear):** Varies by model. Look for DHCP settings in LAN configuration. Some routers only allow setting DNS for the router itself (WAN DNS), not for DHCP clients -- in that case, the DNS container must also be the DHCP server, or use per-device configuration.

## Split DNS / conditional forwarding

### Use case

Your router assigns hostnames to DHCP clients (e.g., `laptop.home.lan`). You want Pi-hole/AdGuard to resolve these names by forwarding queries for `home.lan` to the router's DNS.

### Pi-hole

Settings > DNS > Conditional Forwarding:
- Enable conditional forwarding
- Local network in CIDR: `<LAN_SUBNET>` (e.g., `10.0.0.0/24`)
- IP address of DHCP server (router): `<LAN_GATEWAY>` (e.g., `10.0.0.1`)
- Local domain name: `<LOCAL_DOMAIN>` (e.g., `home.lan`)

Or via dnsmasq config:
```
ssh.exec(host="<DOCKER_HOST>", command="echo 'server=/home.lan/10.0.0.1' > /opt/docker/dns-filtering/pihole/dnsmasq.d/10-conditional.conf")
ssh.exec(host="<DOCKER_HOST>", command="docker exec pihole pihole restartdns")
```

### AdGuard Home

DNS Settings > Upstream DNS servers > add a domain-specific entry:
```
[/home.lan/]10.0.0.1
```

This tells AdGuard Home to forward all `*.home.lan` queries to `10.0.0.1` (the router).

## Unbound as a recursive resolver

Unbound resolves queries by talking directly to authoritative nameservers (root servers, TLD servers, domain nameservers). No third-party DNS provider sees your queries.

### Compose snippet (add to existing dns-filtering compose file):

```yaml
  unbound:
    image: mvance/unbound:latest
    container_name: unbound
    restart: unless-stopped
    volumes:
      - ./unbound/config:/opt/unbound/etc/unbound
    networks:
      dns_macvlan:
        ipv4_address: "<UNBOUND_IP>"  # e.g., 10.0.0.5
```

Minimal Unbound config (`./unbound/config/unbound.conf`):
```
server:
    interface: 0.0.0.0
    port: 53
    access-control: <LAN_SUBNET> allow
    access-control: 127.0.0.0/8 allow

    # Performance
    num-threads: 2
    msg-cache-size: 64m
    rrset-cache-size: 128m
    cache-min-ttl: 300
    cache-max-ttl: 86400

    # Privacy
    hide-identity: yes
    hide-version: yes
    qname-minimisation: yes

    # DNSSEC
    auto-trust-anchor-file: "/opt/unbound/etc/unbound/root.key"
```

Then configure Pi-hole or AdGuard Home to use Unbound as its upstream:
- Pi-hole: Settings > DNS > Custom upstream: `<UNBOUND_IP>#53`
- AdGuard Home: DNS Settings > Upstream DNS: `<UNBOUND_IP>:53`

### Verify Unbound is working:
```
ssh.exec(host="<DOCKER_HOST>", command="dig @<UNBOUND_IP> example.com +short")
ssh.exec(host="<DOCKER_HOST>", command="dig @<UNBOUND_IP> sigfail.verteiltesysteme.net")  # Should return SERVFAIL (DNSSEC validation)
ssh.exec(host="<DOCKER_HOST>", command="dig @<UNBOUND_IP> sigok.verteiltesysteme.net")    # Should return an IP (DNSSEC valid)
```

## Integration with Tailscale / WireGuard

### Tailscale

If the Docker host runs Tailscale and has subnet routing enabled for the LAN:

1. In Tailscale admin console: Access Controls > DNS > add `<DNS_IP>` as a nameserver.
2. Optionally restrict to specific domains: add `<DNS_IP>` as a nameserver for `home.lan` only (split DNS in Tailscale).
3. All Tailscale clients will use Pi-hole/AdGuard for DNS, getting ad-blocking remotely.

If the DNS container itself has a Tailscale IP (e.g., Pi-hole runs on a host with Tailscale), use the Tailscale IP (`100.x.y.z`) as the DNS server in Tailscale settings.

### WireGuard

In the WireGuard client configuration:
```ini
[Interface]
PrivateKey = ...
Address = 10.10.0.2/32
DNS = <DNS_IP>

[Peer]
PublicKey = ...
Endpoint = ...
AllowedIPs = 0.0.0.0/0
```

The `DNS = <DNS_IP>` line routes all DNS queries through the VPN to Pi-hole/AdGuard.

**Requirement:** The WireGuard tunnel must be able to reach `<DNS_IP>`. If the DNS container is on a macvlan IP on the LAN, ensure the WireGuard server routes the LAN subnet through the tunnel. In most homelab WireGuard setups with `AllowedIPs = 0.0.0.0/0` on the client and proper server-side routing, this works automatically.

## Redundancy patterns

### Two instances on separate hosts (best)

- Host A: Pi-hole/AdGuard at `10.0.0.2`
- Host B: Pi-hole/AdGuard at `10.0.0.3`
- Router DHCP: Primary DNS = `10.0.0.2`, Secondary = `10.0.0.3`

If Host A goes down, clients fail over to Host B. DHCP failover is automatic (clients try secondary DNS after primary times out, typically 1-3 seconds).

### Two instances on same host (protects against container crashes)

- Same Docker host, same macvlan network.
- Container 1 at `10.0.0.2`, Container 2 at `10.0.0.3`.
- Does NOT protect against host failure.

### Sync between instances

Pi-hole and AdGuard Home do not natively sync with each other. Options:
- **Manual:** Apply the same configuration changes to both instances.
- **Pi-hole Gravity Sync:** Third-party tool (`https://github.com/vmstan/gravity-sync`) that replicates Pi-hole settings between two instances.
- **AdGuard Home:** Copy `AdGuardHome.yaml` between instances and restart.
- **Scripted:** Write a cron job that copies blocklists and config between instances.

For most homelabs, manual sync is acceptable because configuration changes are infrequent.
