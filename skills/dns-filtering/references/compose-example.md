# DNS Filtering -- Docker Compose Examples

## Pi-hole with macvlan (recommended)

```yaml
version: "3.8"

services:
  pihole:
    image: pihole/pihole:latest
    container_name: pihole
    hostname: pihole
    restart: unless-stopped
    environment:
      TZ: "<TZ>"
      WEBPASSWORD: "<WEBPASSWORD>"
      FTLCONF_LOCAL_IPV4: "<DNS_IP>"
      # Optional: set upstream DNS here instead of via UI
      # PIHOLE_DNS_: "1.1.1.1;1.0.0.1"
    volumes:
      - ./pihole/etc-pihole:/etc/pihole
      - ./pihole/dnsmasq.d:/etc/dnsmasq.d
    networks:
      dns_macvlan:
        ipv4_address: "<DNS_IP>"
    dns:
      - 127.0.0.1
      - 1.1.1.1

networks:
  dns_macvlan:
    external: true
```

**Notes:**
- `FTLCONF_LOCAL_IPV4` tells Pi-hole its own IP for the block page and API. Set it to the macvlan IP.
- `dns` entries ensure the container can resolve during startup (before Pi-hole itself is ready).
- The macvlan network must be created beforehand (see network-integration.md).
- Volumes use bind mounts for easy backup and inspection.

## AdGuard Home with macvlan (recommended)

```yaml
version: "3.8"

services:
  adguardhome:
    image: adguard/adguardhome:latest
    container_name: adguardhome
    hostname: adguardhome
    restart: unless-stopped
    volumes:
      - ./adguard/work:/opt/adguardhome/work
      - ./adguard/conf:/opt/adguardhome/conf
    networks:
      dns_macvlan:
        ipv4_address: "<DNS_IP>"

networks:
  dns_macvlan:
    external: true
```

**Notes:**
- AdGuard Home's initial setup wizard runs on port 3000. After setup, the web UI moves to port 80 (configurable).
- No port mappings needed with macvlan -- all ports are directly accessible on `<DNS_IP>`.
- The `work` directory contains the query log database and runtime data. The `conf` directory contains `AdGuardHome.yaml`.

## Pi-hole with bridge networking (alternative)

Use this if macvlan is not feasible (e.g., VM environments where macvlan is unsupported, or user prefers simplicity).

```yaml
version: "3.8"

services:
  pihole:
    image: pihole/pihole:latest
    container_name: pihole
    hostname: pihole
    restart: unless-stopped
    ports:
      - "53:53/tcp"
      - "53:53/udp"
      - "8080:80/tcp"    # Web UI on 8080 to avoid conflict with other services
    environment:
      TZ: "<TZ>"
      WEBPASSWORD: "<WEBPASSWORD>"
      FTLCONF_LOCAL_IPV4: "<HOST_IP>"
    volumes:
      - ./pihole/etc-pihole:/etc/pihole
      - ./pihole/dnsmasq.d:/etc/dnsmasq.d
    dns:
      - 127.0.0.1
      - 1.1.1.1
```

**Notes:**
- Port 53 on the host must be free. See SKILL.md lesson #1 for resolving systemd-resolved conflicts.
- Web UI is mapped to 8080 to avoid conflicts with reverse proxies or other web services on port 80.
- `FTLCONF_LOCAL_IPV4` should be set to the Docker host's LAN IP.
- Clients point to the Docker host's IP as their DNS server.

## AdGuard Home with bridge networking (alternative)

```yaml
version: "3.8"

services:
  adguardhome:
    image: adguard/adguardhome:latest
    container_name: adguardhome
    hostname: adguardhome
    restart: unless-stopped
    ports:
      - "53:53/tcp"
      - "53:53/udp"
      - "3000:3000/tcp"  # Initial setup wizard
      - "8080:80/tcp"    # Web UI after setup
    volumes:
      - ./adguard/work:/opt/adguardhome/work
      - ./adguard/conf:/opt/adguardhome/conf
```

**Notes:**
- Port 3000 is only needed for the initial setup wizard. It can be removed from the compose file after setup is complete.
- During setup, choose `0.0.0.0:80` as the web UI listen address and `0.0.0.0:53` as the DNS listen address.

## Macvlan network creation

The macvlan network must exist before running `docker compose up`. Create it manually:

```bash
docker network create -d macvlan \
  --subnet=<LAN_SUBNET> \
  --gateway=<LAN_GATEWAY> \
  -o parent=<HOST_INTERFACE> \
  dns_macvlan
```

**Example for a 10.0.0.0/24 network:**
```bash
docker network create -d macvlan \
  --subnet=10.0.0.0/24 \
  --gateway=10.0.0.1 \
  -o parent=eth0 \
  dns_macvlan
```

**Important:** The IP assigned to the container (`<DNS_IP>`) must be:
- Within the subnet range.
- Outside the router's DHCP pool (to prevent IP conflicts).
- Not already in use by another device.

Reserve this IP in your router's DHCP settings or choose an IP outside the DHCP range.

## Volume directory structure

Create these directories before deploying:

**Pi-hole:**
```bash
mkdir -p /opt/docker/dns-filtering/pihole/{etc-pihole,dnsmasq.d}
```

**AdGuard Home:**
```bash
mkdir -p /opt/docker/dns-filtering/adguard/{work,conf}
```

## Redundant deployment (two instances)

For a primary + secondary setup, deploy one instance per host (or two on the same host with different IPs):

```yaml
# primary-dns/docker-compose.yml
version: "3.8"

services:
  pihole-primary:
    image: pihole/pihole:latest
    container_name: pihole-primary
    restart: unless-stopped
    environment:
      TZ: "<TZ>"
      WEBPASSWORD: "<WEBPASSWORD>"
      FTLCONF_LOCAL_IPV4: "10.0.0.2"
    volumes:
      - ./pihole/etc-pihole:/etc/pihole
      - ./pihole/dnsmasq.d:/etc/dnsmasq.d
    networks:
      dns_macvlan:
        ipv4_address: "10.0.0.2"
    dns:
      - 127.0.0.1
      - 1.1.1.1

networks:
  dns_macvlan:
    external: true
```

```yaml
# secondary-dns/docker-compose.yml (on a different host, or same host with different IP)
version: "3.8"

services:
  pihole-secondary:
    image: pihole/pihole:latest
    container_name: pihole-secondary
    restart: unless-stopped
    environment:
      TZ: "<TZ>"
      WEBPASSWORD: "<WEBPASSWORD>"
      FTLCONF_LOCAL_IPV4: "10.0.0.3"
    volumes:
      - ./pihole/etc-pihole:/etc/pihole
      - ./pihole/dnsmasq.d:/etc/dnsmasq.d
    networks:
      dns_macvlan:
        ipv4_address: "10.0.0.3"
    dns:
      - 127.0.0.1
      - 1.1.1.1

networks:
  dns_macvlan:
    external: true
```

Configure router DHCP: Primary DNS = `10.0.0.2`, Secondary DNS = `10.0.0.3`.

Both instances should have identical blocklists and upstream DNS configuration. They do not sync with each other -- configure them independently.
