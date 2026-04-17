# Home Assistant Integration Patterns

## MQTT setup

### Mosquitto configuration

Mosquitto is the recommended MQTT broker for Home Assistant. The configuration covers three areas:

**1. Broker deployment:**
Deploy Mosquitto as a companion container (see `compose-example.md`). Ensure the config, data, and log directories exist before first start.

**2. Authentication:**
Always enable authentication. Anonymous access is a security risk, especially if the broker is reachable from the LAN.

Create the password file after the container is running:
```
ssh.exec(host="<DOCKER_HOST>", command="docker exec mosquitto mosquitto_passwd -c /mosquitto/config/password_file <MQTT_USER>")
```

The `-c` flag creates a new file (use only the first time). To add additional users:
```
ssh.exec(host="<DOCKER_HOST>", command="docker exec mosquitto mosquitto_passwd -b /mosquitto/config/password_file <USERNAME> <PASSWORD>")
```

Restart Mosquitto after password changes:
```
ssh.exec(host="<DOCKER_HOST>", command="docker restart mosquitto")
```

**3. HA MQTT integration:**
In Home Assistant: Settings > Devices & Services > Add Integration > MQTT.

| Setting | Value |
|---------|-------|
| Broker | `localhost` (HA uses host networking, Mosquitto exposes 1883 to host) |
| Port | `1883` |
| Username | `<MQTT_USER>` |
| Password | `<MQTT_PASS>` |
| Discovery | Enable (allows auto-discovery of MQTT devices) |

### Common MQTT topic patterns

| Source | Topic pattern | Example |
|--------|--------------|---------|
| Zigbee2MQTT device state | `zigbee2mqtt/<device_name>` | `zigbee2mqtt/living_room_motion` |
| Zigbee2MQTT availability | `zigbee2mqtt/<device_name>/availability` | Online/offline status |
| Zigbee2MQTT bridge state | `zigbee2mqtt/bridge/state` | `online` / `offline` |
| Zigbee2MQTT set command | `zigbee2mqtt/<device_name>/set` | `{"state": "ON"}` |
| Tasmota telemetry | `tele/<device>/SENSOR` | Sensor readings |
| Tasmota command | `cmnd/<device>/POWER` | ON/OFF commands |
| Tasmota state | `stat/<device>/RESULT` | Command results |
| Custom sensor | `homeassistant/sensor/<id>/state` | HA MQTT discovery format |

### Testing MQTT connectivity

From inside the HA container or any host with `mosquitto_pub`/`mosquitto_sub`:
```
# Subscribe to all topics (useful for debugging):
ssh.exec(host="<DOCKER_HOST>", command="docker exec mosquitto mosquitto_sub -h localhost -u <MQTT_USER> -P <MQTT_PASS> -t '#' -v &")

# Publish a test message:
ssh.exec(host="<DOCKER_HOST>", command="docker exec mosquitto mosquitto_pub -h localhost -u <MQTT_USER> -P <MQTT_PASS> -t 'test/topic' -m 'hello'")
```

## Zigbee2MQTT vs ZHA

### Feature comparison

| Feature | ZHA | Zigbee2MQTT |
|---------|-----|-------------|
| Installation | HA integration (built-in) | Separate container + MQTT broker |
| Configuration | HA UI | YAML file + web UI |
| Device support | Good (~2500 devices) | Excellent (~3500+ devices) |
| Independence from HA | No (restarts with HA) | Yes (Zigbee network stays up) |
| Network visualization | Basic (via ZHA map) | Detailed (built-in web UI) |
| OTA updates | Supported | Supported, more device coverage |
| Group management | Via HA | Via Zigbee2MQTT + HA |
| External antenna support | Depends on coordinator | Full control via config |
| Community support | HA community | Large dedicated community |

### When to migrate from ZHA to Zigbee2MQTT

Migration requires re-pairing all devices (there is no state migration between ZHA and Zigbee2MQTT). Consider migrating if:
- Devices are not supported in ZHA but are in Zigbee2MQTT
- The Zigbee network is unstable and independent operation would help
- Network has grown beyond 30-40 devices and more control is needed

Migration steps:
1. Document all device names, entity IDs, and automations
2. Remove ZHA integration from HA
3. Deploy Mosquitto and Zigbee2MQTT containers
4. Put Zigbee2MQTT in permit_join mode
5. Re-pair all devices (start with routers/mains-powered, then end devices/battery)
6. Update automations to use new entity IDs (Zigbee2MQTT uses `mqtt.` prefix by default, but with `homeassistant: true` in config, entities appear as normal HA entities)

## ESPHome integration

ESPHome devices communicate with HA via two methods:

**Native API (recommended):**
- Direct encrypted connection between ESPHome device and HA
- Faster, lower latency than MQTT
- Auto-discovered by HA if on the same network (requires host networking)
- No broker needed

**MQTT (alternative):**
- Use if ESPHome device is on a different network segment
- Use if you want the device data available outside of HA
- Requires Mosquitto broker

For most homelab setups, the Native API is preferred. ESPHome devices are auto-discovered by HA when using host networking. Add them via Settings > Devices & Services > ESPHome.

ESPHome Dashboard can be run as a separate container for firmware compilation and OTA updates:
```yaml
esphome:
  container_name: esphome
  image: ghcr.io/esphome/esphome:stable
  restart: unless-stopped
  ports:
    - "6052:6052"
  environment:
    - TZ=${TZ:-America/New_York}
  volumes:
    - ${COMPOSE_DIR}/esphome/config:/config
```

## Common first integrations

After completing the onboarding wizard, these integrations provide immediate value with no additional hardware:

| Integration | What it provides | Setup |
|-------------|-----------------|-------|
| **Met.no Weather** | Weather forecast for your location | Auto-discovered during onboarding; or add manually with coordinates |
| **Sun** | Sunrise/sunset times, solar elevation | Auto-configured based on location |
| **System Monitor** | CPU, memory, disk usage of the HA host | Add via Settings > Devices & Services; select sensors to monitor |
| **Uptime** | HA uptime sensor | Add via Settings > Devices & Services |
| **Time & Date** | Date/time sensors for automations | Add via Settings > Devices & Services |
| **Mobile App** | HA Companion app for iOS/Android; provides phone sensors and notifications | Install HA app on phone; auto-discovers HA on local network |

For users with smart devices, the next integrations depend on their ecosystem:
- **Google Cast** -- Chromecast, Google Home, Nest speakers (auto-discovered with host networking)
- **Philips Hue** -- Hue bridge and lights (auto-discovered with host networking)
- **Sonos** -- Sonos speakers (auto-discovered with host networking)
- **TP-Link Kasa/Tapo** -- Smart plugs, bulbs (auto-discovered with host networking)

## Database migration: SQLite to MariaDB

### When to migrate

Signs that SQLite is becoming a bottleneck:
- History page takes >5 seconds to load
- `home-assistant_v2.db` exceeds 1 GB
- HA logs show "database is locked" warnings
- Logbook and history graphs are slow or timeout

### Migration steps

1. **Deploy MariaDB container** (see `compose-example.md`, uncomment the mariadb service).

2. **Wait for MariaDB to initialize:**
   ```
   docker.container.logs(host="<DOCKER_HOST>", name="ha-mariadb", tail=20)
   ```
   Look for "mariadbd: ready for connections".

3. **Add recorder configuration to HA:**
   Edit `<HA_CONFIG_DIR>/configuration.yaml`:
   ```yaml
   recorder:
     db_url: mysql://homeassistant:<MARIADB_HA_PASS>@127.0.0.1/homeassistant?charset=utf8mb4
     purge_keep_days: 14
     commit_interval: 5
   ```

4. **Restart Home Assistant:**
   ```
   ssh.exec(host="<DOCKER_HOST>", command="docker restart homeassistant")
   ```

5. **Verify the new database is in use:**
   ```
   docker.container.logs(host="<DOCKER_HOST>", name="homeassistant", tail=50)
   ```
   Look for "Setting up recorder" and "Connected to recorder database". There should be no SQLite references.

6. **(Optional) Remove old SQLite database:**
   After confirming MariaDB is working (wait at least 24 hours):
   ```
   ssh.exec(host="<DOCKER_HOST>", command="rm <HA_CONFIG_DIR>/home-assistant_v2.db")
   ```

Note: History from the SQLite database is NOT migrated. The MariaDB database starts fresh. If historical data is important, keep the SQLite file as an archive.

### Recorder tuning

For setups with many entities or high-frequency updates, configure recorder exclusions:

```yaml
recorder:
  db_url: mysql://homeassistant:<MARIADB_HA_PASS>@127.0.0.1/homeassistant?charset=utf8mb4
  purge_keep_days: 14
  commit_interval: 5
  exclude:
    domains:
      - automation
      - updater
    entity_globs:
      - sensor.weather_*
    entities:
      - sun.sun
      - sensor.last_boot
```

This reduces database writes significantly and keeps the database size manageable.

## Backup strategies for /config

### What to back up

| Path | Importance | Notes |
|------|-----------|-------|
| `configuration.yaml` | Critical | Main config; may reference other YAML files |
| `automations.yaml` | Critical | All UI-created automations |
| `scripts.yaml` | Critical | All UI-created scripts |
| `scenes.yaml` | Critical | All UI-created scenes |
| `.storage/` | Critical | All UI-configured integrations, entity/device registry, auth tokens |
| `custom_components/` | Important | HACS integrations; can be reinstalled but takes time |
| `blueprints/` | Important | Custom blueprints |
| `www/` | Low | Static files, custom cards (reinstallable via HACS) |
| `home-assistant_v2.db` | Optional | SQLite database; large, can be recreated (loses history) |
| `tts/` | Skip | Cached text-to-speech files; regenerated automatically |
| `deps/` | Skip | Python dependency cache; regenerated on start |

### Backup methods

**Method 1: Simple tar backup (recommended for container deployments):**
```
ssh.exec(host="<DOCKER_HOST>", command="tar czf /opt/backups/ha-config-$(date +%Y%m%d).tar.gz --exclude='home-assistant_v2.db' --exclude='deps' --exclude='tts' -C <HA_CONFIG_DIR> .")
```

Run this on a cron schedule:
```
ssh.exec(host="<DOCKER_HOST>", command="echo '0 3 * * * tar czf /opt/backups/ha-config-$(date +\\%Y\\%m\\%d).tar.gz --exclude=home-assistant_v2.db --exclude=deps --exclude=tts -C <HA_CONFIG_DIR> .' | crontab -")
```

**Method 2: Git version control for YAML files:**
```
ssh.exec(host="<DOCKER_HOST>", command="cd <HA_CONFIG_DIR> && git init && git add *.yaml .storage/ custom_components/ blueprints/ && git commit -m 'backup'")
```

Add a `.gitignore`:
```
home-assistant_v2.db
deps/
tts/
__pycache__/
*.log
.cloud/
```

This gives you version history of configuration changes but does not replace full backups.

**Method 3: Rsync to remote storage:**
```
ssh.exec(host="<DOCKER_HOST>", command="rsync -avz --exclude='home-assistant_v2.db' --exclude='deps' --exclude='tts' <HA_CONFIG_DIR>/ /mnt/nas/backups/homeassistant/")
```

### Restore procedure

1. Stop Home Assistant:
   ```
   ssh.exec(host="<DOCKER_HOST>", command="docker stop homeassistant")
   ```

2. Extract backup:
   ```
   ssh.exec(host="<DOCKER_HOST>", command="tar xzf /opt/backups/ha-config-<DATE>.tar.gz -C <HA_CONFIG_DIR>")
   ```

3. Start Home Assistant:
   ```
   ssh.exec(host="<DOCKER_HOST>", command="docker start homeassistant")
   ```

4. Verify:
   ```
   docker.container.logs(host="<DOCKER_HOST>", name="homeassistant", tail=50)
   ```
   Check for successful startup and integration loading.
