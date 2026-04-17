# Home Assistant Docker Compose Example

## Complete compose file

```yaml
# Home Assistant stack with optional companion services.
# Uncomment services as needed based on your requirements.
#
# Required: homeassistant
# Optional: mosquitto (MQTT broker), zigbee2mqtt (Zigbee bridge), mariadb (recorder database)

services:

  # ---------------------------------------------------------------------------
  # Home Assistant Core
  # ---------------------------------------------------------------------------
  # The main home automation platform. Uses host networking for mDNS/SSDP/UPnP
  # device discovery. All other protocols (Chromecast, Sonos, Hue, ESPHome)
  # depend on multicast traffic that does not traverse Docker bridge networks.
  homeassistant:
    container_name: homeassistant
    image: ghcr.io/home-assistant/home-assistant:stable
    # Pin a specific version for stability:
    # image: ghcr.io/home-assistant/home-assistant:2024.12.0
    restart: unless-stopped
    network_mode: host
    environment:
      - TZ=${TZ:-America/New_York}
    volumes:
      - ${HA_CONFIG_DIR:-/opt/homeassistant/config}:/config
      # Uncomment to enable dbus access (required for Bluetooth):
      # - /run/dbus:/run/dbus:ro
    # Uncomment to pass through a USB device (Zigbee/Z-Wave stick) directly
    # to HA (only needed if using ZHA integration, NOT Zigbee2MQTT):
    # devices:
    #   - /dev/ttyUSB0:/dev/ttyUSB0
    # Uncomment for Bluetooth support (alternative to dbus mount):
    # privileged: true
    depends_on:
      - mosquitto
      # Uncomment if using MariaDB for recorder:
      # - mariadb

  # ---------------------------------------------------------------------------
  # Mosquitto MQTT Broker
  # ---------------------------------------------------------------------------
  # Message broker for MQTT-based integrations: Zigbee2MQTT, Tasmota, custom
  # sensors. Listens on port 1883 (MQTT) and 9001 (WebSocket).
  #
  # Requires a config file at ${COMPOSE_DIR}/mosquitto/config/mosquitto.conf
  # and a password file created with:
  #   docker exec mosquitto mosquitto_passwd -b /mosquitto/config/password_file <user> <pass>
  mosquitto:
    container_name: mosquitto
    image: eclipse-mosquitto:2
    restart: unless-stopped
    ports:
      - "1883:1883"
      - "9001:9001"
    environment:
      - TZ=${TZ:-America/New_York}
    volumes:
      - ${COMPOSE_DIR:-/opt/docker/home-assistant}/mosquitto/config:/mosquitto/config
      - ${COMPOSE_DIR:-/opt/docker/home-assistant}/mosquitto/data:/mosquitto/data
      - ${COMPOSE_DIR:-/opt/docker/home-assistant}/mosquitto/log:/mosquitto/log

  # ---------------------------------------------------------------------------
  # Zigbee2MQTT (optional)
  # ---------------------------------------------------------------------------
  # Bridges Zigbee devices to MQTT. Requires a Zigbee coordinator USB stick
  # (e.g., SONOFF Zigbee 3.0 USB Dongle Plus, ConBee II).
  #
  # The serial port must match the actual device path on the host. Use
  # `ls /dev/ttyUSB* /dev/ttyACM*` to find it. For stable paths, create a
  # udev rule and use the symlink (e.g., /dev/zigbee-coordinator).
  #
  # Requires a configuration.yaml at ${COMPOSE_DIR}/zigbee2mqtt/data/configuration.yaml
  # zigbee2mqtt:
  #   container_name: zigbee2mqtt
  #   image: koenkk/zigbee2mqtt:latest
  #   restart: unless-stopped
  #   ports:
  #     - "8080:8080"
  #   environment:
  #     - TZ=${TZ:-America/New_York}
  #   volumes:
  #     - ${COMPOSE_DIR:-/opt/docker/home-assistant}/zigbee2mqtt/data:/app/data
  #   devices:
  #     - /dev/ttyUSB0:/dev/ttyUSB0
  #     # Use a stable symlink if available:
  #     # - /dev/zigbee-coordinator:/dev/ttyUSB0
  #   depends_on:
  #     - mosquitto
  #   # If using host networking for HA, Zigbee2MQTT can use bridge networking
  #   # since it only needs to reach Mosquitto (which exposes port 1883).
  #   # Update the MQTT server in zigbee2mqtt configuration.yaml to point to
  #   # the host IP or use the Mosquitto container name if on the same network.

  # ---------------------------------------------------------------------------
  # MariaDB (optional)
  # ---------------------------------------------------------------------------
  # Replacement for the default SQLite recorder database. Recommended for
  # setups with >50 entities or >14 days history retention.
  #
  # After deploying, add to HA's configuration.yaml:
  #   recorder:
  #     db_url: mysql://homeassistant:${MARIADB_HA_PASS}@127.0.0.1/homeassistant?charset=utf8mb4
  #     purge_keep_days: 14
  #     commit_interval: 5
  #
  # Note: Use 127.0.0.1 (not container name) because HA uses host networking.
  # MariaDB exposes port 3306 to the host.
  # mariadb:
  #   container_name: ha-mariadb
  #   image: mariadb:11
  #   restart: unless-stopped
  #   ports:
  #     - "3306:3306"
  #   environment:
  #     - MYSQL_ROOT_PASSWORD=${MARIADB_ROOT_PASS}
  #     - MYSQL_DATABASE=homeassistant
  #     - MYSQL_USER=homeassistant
  #     - MYSQL_PASSWORD=${MARIADB_HA_PASS}
  #     - TZ=${TZ:-America/New_York}
  #   volumes:
  #     - ${COMPOSE_DIR:-/opt/docker/home-assistant}/mariadb/data:/var/lib/mysql
```

## Environment file (.env)

Place this alongside `docker-compose.yml`:

```env
TZ=America/New_York
HA_CONFIG_DIR=/opt/homeassistant/config
COMPOSE_DIR=/opt/docker/home-assistant
MQTT_USER=mqtt_user
MQTT_PASS=secure_password_here
MARIADB_ROOT_PASS=db_root_pass_here
MARIADB_HA_PASS=ha_db_pass_here
```

## Mosquitto config file

Place at `${COMPOSE_DIR}/mosquitto/config/mosquitto.conf`:

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

Note: The `listener 9001` and `protocol websockets` lines must be adjacent. The `protocol` directive applies to the listener immediately above it. The default listener (1883) uses the MQTT protocol.

## Zigbee2MQTT config file

Place at `${COMPOSE_DIR}/zigbee2mqtt/data/configuration.yaml`:

```yaml
homeassistant: true
permit_join: false
mqtt:
  base_topic: zigbee2mqtt
  server: mqtt://<HOST_IP>:1883
  user: mqtt_user
  password: secure_password_here
serial:
  port: /dev/ttyUSB0
frontend:
  port: 8080
advanced:
  # Generate a random network key on first start.
  # After first start, Zigbee2MQTT writes the actual key here.
  # Back up this file to preserve your Zigbee network.
  network_key: GENERATE
  last_seen: ISO_8601
  homeassistant_legacy_entity_attributes: false
  legacy_api: false
  legacy_availability_payload: false
device_options:
  legacy: false
```

Note: Use the host's LAN IP for the MQTT server address (not `localhost`) because Zigbee2MQTT runs in bridge networking while Mosquitto exposes port 1883 to the host. If both are on the same Docker bridge network, use `mqtt://mosquitto:1883` instead.

## Networking notes

- **Home Assistant** uses `network_mode: host`. It cannot use Docker DNS to resolve container names. It accesses Mosquitto at `localhost:1883` and MariaDB at `127.0.0.1:3306` (because those containers expose ports to the host).
- **Mosquitto** uses bridge networking with port mapping. Accessible from the host and from HA (via localhost) and from other bridge-networked containers (via container name or host IP).
- **Zigbee2MQTT** uses bridge networking. Reaches Mosquitto via the host IP or container name (if on the same Docker network).
- **MariaDB** uses bridge networking with port mapping. HA reaches it via `127.0.0.1:3306`.

This mixed networking approach is necessary because HA requires host networking for device discovery, while companion services work fine with bridge networking and port mapping.
