# Docker Networking Guide

## Networking modes overview

| Mode | Isolation | Port mapping | Container gets own IP | Use case |
|------|-----------|-------------|----------------------|----------|
| Bridge (default) | Network namespace | Required (`-p`) | On bridge subnet only | Most services |
| Host | None | Not needed | Uses host IP | DHCP, multicast, Tailscale |
| Macvlan | Full | Not needed | Own LAN IP + MAC | LAN-visible services |
| Ipvlan | Full | Not needed | Own LAN IP, shared MAC | Switch with MAC limits |
| None | Complete | N/A | No networking | Security-sensitive batch jobs |

## Bridge networking (default)

### How it works

- Docker creates a virtual bridge (`docker0` or a custom one).
- Each container gets a `veth` pair connecting it to the bridge.
- Containers on the same bridge can communicate by container name (Docker's embedded DNS).
- External access requires explicit port mappings (`-p HOST:CONTAINER`).

### Default bridge vs custom bridge

**Default bridge (`docker0`):**
- Containers can communicate by IP but NOT by name.
- Legacy; not recommended for new deployments.

**Custom bridge (recommended):**
- Containers can communicate by container name (automatic DNS).
- Better isolation -- only containers on the same network can talk.
- Create with: `docker network create <NETWORK_NAME>`

### Creating a custom bridge network

Via SSH:
```
ssh.exec(host="<DOCKER_HOST>", command="docker network create proxy-net")
```

In Compose:
```yaml
networks:
  app-net:
    # Created automatically by Compose
```

For networks shared across multiple Compose stacks:
```yaml
networks:
  proxy-net:
    external: true  # Must be created manually first
```

### Port mapping strategies

| Syntax | Meaning |
|--------|---------|
| `8080:80` | Host 8080 → container 80, all interfaces |
| `127.0.0.1:8080:80` | Host 8080 → container 80, localhost only |
| `8080:80/udp` | UDP port mapping |
| `8080-8090:8080-8090` | Port range |

**Binding to localhost only** (`127.0.0.1:PORT:PORT`) is useful when a reverse proxy on the same host handles external traffic. The service is not directly accessible from the network.

## Host networking

### How it works

- Container shares the host's network namespace entirely.
- No port mapping -- the container binds directly to host ports.
- Full access to host network interfaces, including broadcast and multicast.

### When to use

- **Pi-hole with DHCP:** DHCP requires raw network access that bridge mode can't provide.
- **Tailscale containers:** Needs to manage host routing tables and network interfaces.
- **mDNS/Avahi services:** Multicast discovery doesn't cross bridge boundaries.
- **Performance-sensitive:** Eliminates NAT overhead (rarely matters in practice).

### Gotchas

- Port conflicts are with host services, not just other containers.
- Cannot run two host-network containers that bind the same port.
- Container can see all host traffic -- reduced isolation.

## Macvlan networking

### How it works

- Container gets its own MAC address and IP on the physical network.
- Appears as a separate device to other hosts, switches, and routers.
- Can receive DHCP lease or use a static IP.

### Setup

```bash
# Create the macvlan network
docker network create -d macvlan \
  --subnet=10.0.1.0/24 \
  --gateway=10.0.1.1 \
  --ip-range=10.0.1.192/26 \
  -o parent=eth0 \
  macvlan-net
```

- `--subnet`: the LAN subnet.
- `--gateway`: the LAN gateway.
- `--ip-range`: subset of IPs reserved for macvlan containers (prevents conflicts with DHCP).
- `-o parent`: the host's physical NIC.

### Assigning a static IP

```yaml
services:
  pihole-secondary:
    image: pihole/pihole:latest
    networks:
      macvlan-net:
        ipv4_address: 10.0.1.200

networks:
  macvlan-net:
    external: true
```

### Host-to-macvlan communication

**Important:** The Docker host cannot communicate with macvlan containers directly. Traffic between the host and macvlan containers is blocked by the kernel.

Workaround -- create a macvlan shim interface on the host:
```bash
ip link add macvlan-shim link eth0 type macvlan mode bridge
ip addr add 10.0.1.254/32 dev macvlan-shim
ip link set macvlan-shim up
ip route add 10.0.1.192/26 dev macvlan-shim
```

This gives the host a way to reach the macvlan subnet. Add this to a startup script or systemd unit for persistence.

## DNS resolution between containers

### Same Compose stack

Containers in the same Compose file automatically resolve each other by service name:
```yaml
services:
  app:
    environment:
      - DB_HOST=db    # Resolves to the db container's IP
  db:
    image: postgres:16
```

### Cross-stack communication

Containers in different Compose stacks need a shared external network:

```bash
# Create once
docker network create shared-net
```

Stack A:
```yaml
networks:
  shared-net:
    external: true
services:
  service-a:
    networks:
      - shared-net
```

Stack B:
```yaml
networks:
  shared-net:
    external: true
services:
  service-b:
    networks:
      - shared-net
```

Now `service-a` can reach `service-b` by container name.

### Custom DNS for containers

If containers need to resolve custom DNS names (e.g., internal domains):

```yaml
services:
  myservice:
    dns:
      - 10.0.1.53    # Local DNS server
      - 1.1.1.1      # Fallback
```

Or set globally in Docker daemon config (`/etc/docker/daemon.json`):
```json
{
  "dns": ["10.0.1.53", "1.1.1.1"]
}
```

## Common networking problems

### Port conflict

**Symptom:** Container fails to start with "port already in use".

**Diagnosis:**
```
ssh.exec(host="<DOCKER_HOST>", command="ss -tlnp | grep <PORT>")
```

**Fix:** Stop the conflicting service or use a different host port.

### Container-to-container DNS not working

**Symptom:** One container can't reach another by name.

**Diagnosis:** Are they on the same custom bridge network? Default bridge does NOT support DNS.

**Fix:** Create a custom network and attach both containers.

### External access blocked

**Symptom:** Container is running, port mapped, but inaccessible from other hosts.

**Diagnosis:**
1. Test from the Docker host itself: `curl http://localhost:<PORT>`
2. If that works, it's a firewall issue: `ufw status` or `iptables -L -n`
3. Docker manipulates iptables directly -- `ufw` rules may be bypassed.

**Fix:** If using UFW, be aware that Docker adds iptables rules that bypass UFW. For strict firewall control, use `iptables` directly or configure Docker's iptables behavior in `/etc/docker/daemon.json`:
```json
{
  "iptables": false
}
```
**Warning:** Disabling Docker's iptables management means you must manually manage port forwarding rules.
