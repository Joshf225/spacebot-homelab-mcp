---
name: home-assistant
description: Deploy and configure Home Assistant with optional companion services (Mosquitto MQTT, Zigbee2MQTT, MariaDB) using Docker. Covers deployment method selection, host networking for device discovery, USB passthrough for Zigbee/Z-Wave, HACS installation, database migration, remote access strategies, and common failure diagnosis.
version: 1.0.0
---

# Home Assistant Deployment

## Purpose

Use this skill when a Spacebot agent needs to deploy, configure, or troubleshoot Home Assistant and its companion services:

- **Home Assistant** -- home automation platform (automations, dashboards, integrations, device management)
- **Mosquitto** -- MQTT broker (message bus for Zigbee2MQTT, Tasmota, ESPHome, and other IoT devices)
- **Zigbee2MQTT** -- Zigbee coordinator bridge (connects Zigbee devices to HA via MQTT)
- **MariaDB** -- relational database (optional replacement for default SQLite recorder)

This playbook separates commonly conflated areas:

- **Deployment method vs features** -- Container mode gives you the core platform; HAOS gives you Supervisor and the add-on store. They are different tradeoffs, not better/worse.
- **Host networking vs bridge networking** -- device discovery (mDNS, SSDP, UPnP) requires host networking; web-only usage does not.
- **ZHA vs Zigbee2MQTT** -- both need a Zigbee coordinator stick, but ZHA is an HA integration while Zigbee2MQTT is a separate container.
- **Container deployment vs integration configuration** -- getting HA running is step one; configuring integrations, automations, and dashboards is step two.

## When to invoke this skill

- User wants to deploy Home Assistant in Docker.
- User wants to set up MQTT for IoT devices.
- User needs Zigbee or Z-Wave support in a containerized setup.
- User wants to migrate from SQLite to MariaDB for the HA recorder.
- Existing Home Assistant deployment has device discovery issues, USB passthrough problems, or performance degradation.
- User wants to install HACS in container mode.
- User wants remote access to Home Assistant (Tailscale, Nabu Casa, reverse proxy).

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
| `audit.verify_operation` | Verify a logged audit operation | No |
| `audit.verify_container_state` | Verify a logged container state audit entry | No |

## Environment variables

| Placeholder | Meaning | Example |
|---|---|---|
| `<DOCKER_HOST>` | Docker host config name in Spacebot | `ha-server` |
| `<HA_CONFIG_DIR>` | Home Assistant config directory on host | `/opt/homeassistant/config` |
| `<COMPOSE_DIR>` | Directory where compose file lives | `/opt/docker/home-assistant` |
| `<TZ>` | Timezone | `America/New_York` |
| `<MQTT_USER>` | MQTT broker username | `mqtt_user` |
| `<MQTT_PASS>` | MQTT broker password | `secure_password` |
| `<MARIADB_ROOT_PASS>` | MariaDB root password | `db_root_pass` |
| `<MARIADB_HA_PASS>` | MariaDB password for HA database | `ha_db_pass` |

## Decision tree: Deployment method

### Home Assistant Container (Docker) -- recommended for existing Docker infrastructure

Deploy the `homeassistant/home-assistant` image directly. This gives you:
- The full Home Assistant core platform (automations, integrations, dashboards, UI config)
- All HACS custom components (manually installed)
- Full control over networking, volumes, and companion containers

You do NOT get:
- The Supervisor
- The Add-on Store (add-ons are Supervisor-managed containers; without Supervisor, you manage companion containers yourself)
- One-click backups through the UI (you manage backups via host tools)

This is what this skill deploys. It is the right choice for users who already have Docker infrastructure and want to manage everything through Compose.

### Home Assistant OS (HAOS) -- alternative for appliance-style deployment

HAOS is a full operating system image that includes the Supervisor, add-on store, and managed networking. It takes over an entire machine or VM.

To deploy on Proxmox:
1. Download the qcow2 image from `https://www.home-assistant.io/installation/`
2. Create a new VM in Proxmox (2 CPU cores, 2GB RAM minimum, no disk)
3. Import the disk: `qm importdisk <VMID> haos_ova-<version>.qcow2 <storage>`
4. Attach the imported disk as SATA/SCSI
5. Set boot order to the imported disk
6. Start the VM; HA is available at `http://<VM_IP>:8123`

Choose HAOS if:
- The user wants the add-on ecosystem (one-click Zigbee2MQTT, Node-RED, Grafana, etc.)
- The user does not have existing Docker infrastructure
- The user prefers an appliance model over manual container management

### Home Assistant Supervised -- not recommended

Runs the Supervisor on a generic Debian Linux install. It is fragile, has strict OS requirements, and breaks easily with system updates. Do not recommend unless the user specifically insists and understands the maintenance burden.

## High-confidence lessons learned

### 1. Host network mode is strongly recommended

Home Assistant relies on mDNS (Multicast DNS), SSDP (Simple Service Discovery Protocol), and UPnP for automatic device discovery. These protocols use multicast traffic that does not traverse Docker bridge networks.

With bridge networking:
- Chromecast, Google Home, Sonos, and most smart speakers are invisible
- Philips Hue bridge discovery fails
- ESPHome device discovery fails
- HomeKit integration cannot advertise

With host networking:
- All discovery protocols work as if HA were running directly on the host
- HA binds to port 8123 on the host directly (no port mapping needed)
- No container network isolation (acceptable tradeoff for a home automation platform)

Always use `network_mode: host` unless the user explicitly does not need device discovery (rare).

### 2. USB device passthrough for Zigbee/Z-Wave

Zigbee coordinators (SONOFF Zigbee 3.0 USB Dongle Plus, ConBee II, etc.) and Z-Wave sticks appear as serial devices on the host, typically at `/dev/ttyUSB0` or `/dev/ttyACM0`.

To pass a USB device to a container:
```yaml
devices:
  - /dev/ttyUSB0:/dev/ttyUSB0
```

Common issues:
- The device path changes on reboot if multiple USB devices are connected. Use udev rules to create a stable symlink:
  ```
  ssh.exec(host="<DOCKER_HOST>", command="udevadm info -a -n /dev/ttyUSB0 | grep -E 'idVendor|idProduct|serial'")
  ```
  Then create a udev rule:
  ```
  SUBSYSTEM=="tty", ATTRS{idVendor}=="1a86", ATTRS{idProduct}=="55d4", SYMLINK+="zigbee-coordinator"
  ```
  Map `/dev/zigbee-coordinator` in the compose file instead.

- For VMs (Proxmox), pass the USB device through to the VM first via Proxmox UI or CLI before Docker can see it.

- Only ONE container should access a given USB device. If using Zigbee2MQTT, do not also enable the ZHA integration in HA for the same stick.

### 3. Bluetooth passthrough

BLE (Bluetooth Low Energy) device support in Docker requires either:
- `privileged: true` (easiest but least secure)
- Explicit `/dev` mounts and dbus socket sharing:
  ```yaml
  volumes:
    - /run/dbus:/run/dbus:ro
  devices:
    - /dev/serial1:/dev/serial1
  ```

Bluetooth in Docker is unreliable. If BLE devices are a priority, consider HAOS in a VM with USB passthrough, or use an ESPHome BLE proxy.

### 4. Persistent config -- back it up religiously

All Home Assistant state lives in the `/config` directory:
- `configuration.yaml` -- main configuration
- `automations.yaml` -- automations defined via UI
- `scripts.yaml` -- scripts defined via UI
- `scenes.yaml` -- scenes defined via UI
- `.storage/` -- UI-configured integrations, entity registry, device registry, auth
- `home-assistant_v2.db` -- default SQLite recorder database (can be large)
- `custom_components/` -- HACS and manually installed integrations
- `blueprints/` -- automation blueprints
- `www/` -- static files served by HA (images, custom cards)

The `.storage/` directory is critical. It contains all integration configurations made through the UI. Losing it means reconfiguring every integration.

Back up `/config` before any upgrade:
```
ssh.exec(host="<DOCKER_HOST>", command="cp -r <HA_CONFIG_DIR> <HA_CONFIG_DIR>-backup-$(date +%Y%m%d)")
```

Exclude `home-assistant_v2.db` from frequent backups if it is large (it can be recreated, just loses history).

### 5. Database choice -- SQLite vs MariaDB/PostgreSQL

The default SQLite recorder works for small setups (<50 entities, <7 days retention). Problems appear with:
- Large numbers of entities (>100) -- UI becomes sluggish
- Long history retention (>30 days) -- database grows to multiple GB
- Frequent state changes (power monitoring, motion sensors) -- write contention

To switch to MariaDB, add to `configuration.yaml`:
```yaml
recorder:
  db_url: mysql://homeassistant:<MARIADB_HA_PASS>@mariadb/homeassistant?charset=utf8mb4
  purge_keep_days: 14
  commit_interval: 5
```

Deploy MariaDB as a companion container (see compose example).

### 6. MQTT broker -- Mosquitto as companion container

Many integrations communicate via MQTT:
- Zigbee2MQTT publishes device states to MQTT topics
- Tasmota devices publish to MQTT natively
- ESPHome can use MQTT (though native API is preferred)
- Custom sensors using MQTT protocol

Deploy Mosquitto alongside HA. Configure with authentication (see `references/integration-patterns.md`).

In HA, add the MQTT integration: Settings > Devices & Services > Add Integration > MQTT. Set broker to `localhost` (if using host networking) or the container name/IP.

### 7. Zigbee2MQTT vs ZHA

**ZHA (Zigbee Home Automation):**
- Built-in HA integration, no extra containers
- Configuration through HA UI
- Device support is good but slightly behind Zigbee2MQTT
- Simpler to set up
- Depends on HA; if HA restarts, Zigbee restarts

**Zigbee2MQTT:**
- Separate container, communicates via MQTT
- More device support (koenkk/zigbee2mqtt device list is extensive)
- Independent of HA (Zigbee network stays up if HA restarts)
- Web UI for device management
- More configuration flexibility (channel, transmit power, etc.)
- Requires Mosquitto MQTT broker

Recommendation: Use Zigbee2MQTT for larger Zigbee networks (>20 devices) or if specific devices are only supported by Zigbee2MQTT. Use ZHA for smaller setups where simplicity matters.

### 8. HACS (Home Assistant Community Store)

In container mode, HACS is not available through an add-on store. Install manually:

```
ssh.exec(host="<DOCKER_HOST>", command="docker exec homeassistant bash -c 'wget -O - https://get.hacs.xyz | bash -'")
```

After installation:
1. Restart Home Assistant
2. Go to Settings > Devices & Services > Add Integration > HACS
3. Follow the GitHub OAuth flow to authorize HACS

HACS installs custom components into `/config/custom_components/` and custom frontend cards into `/config/www/community/`.

### 9. HTTPS and remote access

Never expose port 8123 directly to the internet. Options for remote access:

**Tailscale (recommended for homelab):**
- Install Tailscale on the Docker host
- Access HA via Tailscale IP (e.g., `http://100.x.x.x:8123`)
- Zero config, encrypted, no port forwarding needed
- Free for personal use

**Nabu Casa (paid, easiest):**
- Subscription service from the HA team ($6.50/mo)
- Provides `https://<id>.ui.nabu.casa` with automatic SSL
- Also enables voice assistants and supports HA development
- Configure in HA: Settings > Home Assistant Cloud

**Reverse proxy (self-hosted):**
- Use Traefik, Caddy, or nginx-proxy-manager
- Requires a domain name and Let's Encrypt certificate
- Add to HA `configuration.yaml` to trust the proxy:
  ```yaml
  http:
    use_x_forwarded_for: true
    trusted_proxies:
      - 172.16.0.0/12
      - 192.168.0.0/16
  ```

### 10. Image versioning strategy

Pin the HA image to a specific version for stability:
```yaml
image: ghcr.io/home-assistant/home-assistant:2024.12.0
```

HA releases monthly. Breaking changes are documented in release notes. Upgrade deliberately:
1. Read the release notes for breaking changes
2. Back up `/config`
3. Update the image tag in compose
4. `docker compose pull && docker compose up -d`
5. Check logs for migration errors

Using `:latest` or `:stable` is acceptable for users who want automatic updates but increases risk of unexpected breakage.

## Service configuration details

### Home Assistant

| Setting | Value |
|---------|-------|
| Image | `ghcr.io/home-assistant/home-assistant:stable` |
| Default port | 8123 (HTTP) |
| Config path | `<HA_CONFIG_DIR>` mapped to `/config` |
| Network mode | `host` (recommended) |
| Restart policy | `unless-stopped` |

After deployment:
1. Access web UI at `http://<DOCKER_HOST>:8123`
2. Complete onboarding wizard (create admin account, set location, configure units)
3. HA auto-discovers devices on the network (if host networking is used)
4. Add integrations via Settings > Devices & Services

### Mosquitto MQTT Broker

| Setting | Value |
|---------|-------|
| Image | `eclipse-mosquitto:2` |
| Default ports | 1883 (MQTT), 9001 (WebSocket) |
| Config path | `<COMPOSE_DIR>/mosquitto/config` mapped to `/mosquitto/config` |
| Data path | `<COMPOSE_DIR>/mosquitto/data` mapped to `/mosquitto/data` |
| Log path | `<COMPOSE_DIR>/mosquitto/log` mapped to `/mosquitto/log` |

Requires a config file at `<COMPOSE_DIR>/mosquitto/config/mosquitto.conf`:
```
persistence true
persistence_location /mosquitto/data/
log_dest file /mosquitto/log/mosquitto.log
listener 1883
listener 9001
protocol websockets
password_file /mosquitto/config/password_file
allow_anonymous false
```

Create the password file:
```
ssh.exec(host="<DOCKER_HOST>", command="docker exec mosquitto mosquitto_passwd -b /mosquitto/config/password_file <MQTT_USER> <MQTT_PASS>")
```

### Zigbee2MQTT

| Setting | Value |
|---------|-------|
| Image | `koenkk/zigbee2mqtt:latest` |
| Default port | 8080 (Web UI) |
| Config path | `<COMPOSE_DIR>/zigbee2mqtt/data` mapped to `/app/data` |
| USB device | `/dev/ttyUSB0` (or stable symlink) |

Requires `configuration.yaml` at `<COMPOSE_DIR>/zigbee2mqtt/data/configuration.yaml`:
```yaml
homeassistant: true
permit_join: false
mqtt:
  base_topic: zigbee2mqtt
  server: mqtt://localhost:1883
  user: <MQTT_USER>
  password: <MQTT_PASS>
serial:
  port: /dev/ttyUSB0
frontend:
  port: 8080
advanced:
  network_key: GENERATE
```

### MariaDB (optional)

| Setting | Value |
|---------|-------|
| Image | `mariadb:11` |
| Default port | 3306 |
| Data path | `<COMPOSE_DIR>/mariadb/data` mapped to `/var/lib/mysql` |

Environment variables:
- `MYSQL_ROOT_PASSWORD=<MARIADB_ROOT_PASS>`
- `MYSQL_DATABASE=homeassistant`
- `MYSQL_USER=homeassistant`
- `MYSQL_PASSWORD=<MARIADB_HA_PASS>`

## Safety rules

1. **Always snapshot HA config before major upgrades.**
   ```
   ssh.exec(host="<DOCKER_HOST>", command="cp -r <HA_CONFIG_DIR> <HA_CONFIG_DIR>-backup-$(date +%Y%m%d)")
   ```

2. **Never expose port 8123 to the internet without SSL and authentication.** HA has authentication by default, but without SSL, credentials are transmitted in cleartext. Use Tailscale, Nabu Casa, or a reverse proxy with TLS.

3. **Pin the HA image version for stability.** Upgrade deliberately after reading release notes. Monthly releases can contain breaking changes to integrations, YAML schema, or database schema.

4. **Use `confirm_operation` before stopping Home Assistant.** Stopping HA disables all automations, presence detection, and alarm systems. Users may have security-critical automations running.

5. **Never run `docker compose down -v` on the HA stack.** This deletes named volumes. Use `docker compose down` (without `-v`) unless explicitly confirmed.

6. **Do not run HA as `privileged` unless required.** Bluetooth passthrough may require it, but for most setups, specific device mounts are sufficient.

7. **Back up `configuration.yaml` and the `.storage/` directory before editing via container.** UI-configured integrations live in `.storage/` and are not human-editable.

8. **Only one process should own a USB device.** Do not enable ZHA and Zigbee2MQTT for the same Zigbee coordinator simultaneously.

9. **Do not delete `/config/.storage/` or `/config/home-assistant_v2.db` unless explicitly confirmed.** The former requires reconfiguring all integrations; the latter loses all history.

10. **Verify the Zigbee coordinator firmware matches the software.** Zigbee2MQTT and ZHA may require specific firmware versions on the coordinator. Flashing wrong firmware bricks the stick.

## Recommended procedural flow for agents

### Full deployment

1. **Gather requirements:**
   - Target Docker host
   - Timezone
   - Need MQTT? (Zigbee, Tasmota, ESPHome devices)
   - Have a Zigbee coordinator stick? Which model?
   - Zigbee2MQTT or ZHA?
   - Expected number of entities (for database choice)
   - Remote access needed? (Tailscale, Nabu Casa, reverse proxy)

2. **Create directory structure on host:**
   ```
   ssh.exec(host="<DOCKER_HOST>", command="mkdir -p <HA_CONFIG_DIR>")
   ssh.exec(host="<DOCKER_HOST>", command="mkdir -p <COMPOSE_DIR>/{mosquitto/{config,data,log},zigbee2mqtt/data,mariadb/data}")
   ```

3. **Check for port conflicts:**
   ```
   ssh.exec(host="<DOCKER_HOST>", command="ss -tlnp | grep -E ':(8123|1883|9001|8080|3306) '")
   ```

4. **(If using MQTT) Create Mosquitto config:**
   Generate `mosquitto.conf` and upload:
   ```
   ssh.upload(host="<DOCKER_HOST>", local_path="./mosquitto.conf", remote_path="<COMPOSE_DIR>/mosquitto/config/mosquitto.conf")
   ```

5. **(If using Zigbee2MQTT) Identify USB device:**
   ```
   ssh.exec(host="<DOCKER_HOST>", command="ls -la /dev/ttyUSB* /dev/ttyACM* 2>/dev/null")
   ssh.exec(host="<DOCKER_HOST>", command="dmesg | grep -i -E 'tty|usb|zigbee|serial' | tail -20")
   ```
   Create Zigbee2MQTT configuration and upload.

6. **Upload compose file:**
   Generate from reference (see `references/compose-example.md`). Upload:
   ```
   ssh.upload(host="<DOCKER_HOST>", local_path="./docker-compose.yml", remote_path="<COMPOSE_DIR>/docker-compose.yml")
   ```

7. **Pull all images:**
   ```
   ssh.exec(host="<DOCKER_HOST>", command="docker compose pull", cwd="<COMPOSE_DIR>")
   ```

8. **Deploy the stack:**
   ```
   ssh.exec(host="<DOCKER_HOST>", command="docker compose up -d", cwd="<COMPOSE_DIR>")
   ```

9. **Wait for first boot (2-3 minutes):**
   ```
   ssh.exec(host="<DOCKER_HOST>", command="docker compose ps", cwd="<COMPOSE_DIR>")
   docker.container.logs(host="<DOCKER_HOST>", container="homeassistant", tail=30)
   ```
   Look for "Home Assistant initialized" in logs.

10. **User completes onboarding wizard:**
    Direct user to `http://<DOCKER_HOST>:8123`. They must create an admin account and configure basic settings. This cannot be automated.

11. **(If using MQTT) Create Mosquitto user and configure HA:**
    ```
    ssh.exec(host="<DOCKER_HOST>", command="docker exec mosquitto mosquitto_passwd -b /mosquitto/config/password_file <MQTT_USER> <MQTT_PASS>")
    ssh.exec(host="<DOCKER_HOST>", command="docker restart mosquitto")
    ```
    Then in HA: Settings > Devices & Services > Add Integration > MQTT. Set broker to `localhost`, port `1883`, username and password.

12. **(If using Zigbee2MQTT) Verify Zigbee2MQTT is connected:**
    ```
    docker.container.logs(host="<DOCKER_HOST>", container="zigbee2mqtt", tail=30)
    ```
    Look for "Zigbee2MQTT started" and "Connected to MQTT server". Access Zigbee2MQTT UI at `http://<DOCKER_HOST>:8080`.

13. **(Optional) Install HACS:**
    ```
    ssh.exec(host="<DOCKER_HOST>", command="docker exec homeassistant bash -c 'wget -O - https://get.hacs.xyz | bash -'")
    ssh.exec(host="<DOCKER_HOST>", command="docker restart homeassistant")
    ```
    Then in HA: Settings > Devices & Services > Add Integration > HACS. Follow GitHub OAuth flow.

14. **Configure integrations:**
    Guide user through adding integrations based on their devices. Common first integrations: Met.no Weather, Sun, System Monitor, MQTT (if deployed).

### Updating the stack

1. Back up config:
   ```
   ssh.exec(host="<DOCKER_HOST>", command="cp -r <HA_CONFIG_DIR> <HA_CONFIG_DIR>-backup-$(date +%Y%m%d)")
   ```

2. Read HA release notes for breaking changes.

3. Update image tag in compose file (or pull `:stable`):
   ```
   ssh.exec(host="<DOCKER_HOST>", command="docker compose pull && docker compose up -d", cwd="<COMPOSE_DIR>")
   ```

4. Verify all containers are up:
   ```
   ssh.exec(host="<DOCKER_HOST>", command="docker compose ps", cwd="<COMPOSE_DIR>")
   ```

5. Check logs for migration errors:
   ```
   docker.container.logs(host="<DOCKER_HOST>", container="homeassistant", tail=50)
   ```

### Adding a companion service to an existing deployment

1. Add the new service definition to the compose file.
2. Create required config directories and files.
3. Upload the updated compose file.
4. Run `docker compose up -d` -- Compose only creates/recreates changed services.
5. Configure the new service and wire it to HA.

## Fast diagnosis cheatsheet

| Symptom | Most likely cause | Fix |
|---|---|---|
| Discovery not finding devices (Chromecast, Sonos, Hue) | Not using host network mode | Switch to `network_mode: host` in compose |
| Zigbee stick not detected | USB not passed through, or wrong /dev path | Check `ls /dev/tty*` on host; verify `devices:` in compose; check `dmesg` for USB events |
| Zigbee stick detected but Zigbee2MQTT fails to start | Another process (ZHA) is using the device, or wrong firmware | Stop ZHA integration in HA; verify firmware matches Zigbee2MQTT requirements |
| HA slow / UI laggy | SQLite database too large; too many entities with frequent state changes | Switch recorder to MariaDB; reduce `purge_keep_days`; exclude high-frequency entities from recorder |
| Database file growing uncontrollably | Default recorder keeps 10 days of all entities | Configure `recorder:` with `purge_keep_days` and `exclude` filters in `configuration.yaml` |
| Integration fails after container restart | Container name resolution issue (bridge networking) | Use host networking; or ensure containers are on the same Docker network |
| Cannot access HA remotely | No remote access configured | Set up Tailscale, Nabu Casa, or reverse proxy |
| Automation not triggering | Entity ID typo, state not updating, condition never true | Check HA logs (Settings > System > Logs); use Developer Tools > States to verify entity states; trace the automation |
| HACS not showing in integrations | `custom_components/hacs/` not present or HA not restarted | Verify directory exists in `/config/custom_components/hacs/`; restart HA container |
| MQTT connection refused | Mosquitto not running, wrong credentials, or anonymous access disabled without password file | Check Mosquitto logs; verify `mosquitto.conf`; recreate password file |
| "Platform not ready" errors on startup | Integration dependency (e.g., MQTT broker) not available yet | Add `depends_on` in compose; or HA retries automatically after a few minutes |
| Zigbee devices falling off network | Weak Zigbee mesh, coordinator too far, interference from USB3 | Add Zigbee router devices (mains-powered); use a USB extension cable to move coordinator away from USB3 ports |
| Restore from backup missing integrations | `.storage/` directory not included in backup | Always back up the entire `/config` directory including hidden directories |
| "400 Bad Request" behind reverse proxy | Trusted proxies not configured | Add `http.trusted_proxies` to `configuration.yaml` |

## Decision trees

### Which companion services to deploy?

**Minimum (HA only):**
- Home Assistant container with host networking
- Sufficient for: WiFi devices, cloud integrations, Bluetooth (with caveats)

**Add Mosquitto if:**
- User has or plans Zigbee devices (with Zigbee2MQTT)
- User has Tasmota-flashed devices
- User wants MQTT-based sensor publishing
- User plans ESPHome devices (though ESPHome native API is preferred)

**Add Zigbee2MQTT if:**
- User has a Zigbee coordinator stick
- User prefers Zigbee2MQTT over ZHA (larger device support, independent operation)

**Add MariaDB if:**
- User has >50 entities with frequent state changes
- User wants >14 days of history retention
- HA UI becomes sluggish when loading history graphs

### ZHA or Zigbee2MQTT?

**Choose ZHA if:**
- Small Zigbee network (<20 devices)
- User prefers simplicity (no extra containers)
- All devices are in the ZHA device database

**Choose Zigbee2MQTT if:**
- Large Zigbee network (>20 devices)
- Specific devices only supported by Zigbee2MQTT
- User wants the Zigbee network independent of HA restarts
- User wants the Zigbee2MQTT web UI for network visualization

### Deployment method?

**Docker Container (this skill):**
- User already has Docker infrastructure
- User wants full control over containers
- User is comfortable managing companion services manually

**HAOS (VM):**
- User wants the add-on store and Supervisor
- User does not have existing Docker infrastructure
- User prefers an appliance model
- User wants one-click backups and update management

## References

See supporting reference docs in `references/`:

- `compose-example.md` -- Complete production docker-compose.yml with Home Assistant, Mosquitto, Zigbee2MQTT, and optional MariaDB.
- `integration-patterns.md` -- MQTT setup, Zigbee2MQTT vs ZHA decision, ESPHome, database migration, and backup strategies.
