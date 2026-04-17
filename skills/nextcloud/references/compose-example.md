# Nextcloud Docker Compose Example

Complete production-ready compose file for Nextcloud with MariaDB, Redis, and cron sidecar.

## docker-compose.yml

```yaml
version: "3.8"

services:
  nextcloud-db:
    image: mariadb:lts
    container_name: nextcloud-db
    restart: unless-stopped
    command: --transaction-isolation=READ-COMMITTED --log-bin=OFF --innodb-read-only-compressed=OFF
    volumes:
      - /opt/nextcloud/db:/var/lib/mysql
    environment:
      MYSQL_ROOT_PASSWORD: ${MYSQL_ROOT_PASSWORD}
      MYSQL_DATABASE: nextcloud
      MYSQL_USER: nextcloud
      MYSQL_PASSWORD: ${MYSQL_PASSWORD}
    networks:
      - nextcloud
    healthcheck:
      test: ["CMD", "healthcheck.sh", "--connect", "--innodb_initialized"]
      interval: 30s
      timeout: 10s
      retries: 5
      start_period: 30s

  nextcloud-redis:
    image: redis:alpine
    container_name: nextcloud-redis
    restart: unless-stopped
    volumes:
      - /opt/nextcloud/redis:/data
    networks:
      - nextcloud
    healthcheck:
      test: ["CMD", "redis-cli", "ping"]
      interval: 30s
      timeout: 5s
      retries: 3

  nextcloud:
    image: nextcloud:29-apache
    container_name: nextcloud
    restart: unless-stopped
    ports:
      - "8080:80"
    volumes:
      - /opt/nextcloud/html:/var/www/html
      - /opt/nextcloud/data:/var/www/html/data
    environment:
      # Database
      MYSQL_HOST: nextcloud-db
      MYSQL_DATABASE: nextcloud
      MYSQL_USER: nextcloud
      MYSQL_PASSWORD: ${MYSQL_PASSWORD}

      # Redis
      REDIS_HOST: nextcloud-redis
      REDIS_HOST_PORT: 6379

      # Admin account (only used on first install)
      NEXTCLOUD_ADMIN_USER: ${NEXTCLOUD_ADMIN_USER}
      NEXTCLOUD_ADMIN_PASSWORD: ${NEXTCLOUD_ADMIN_PASSWORD}

      # Trusted domains (space-separated)
      NEXTCLOUD_TRUSTED_DOMAINS: ${NEXTCLOUD_DOMAIN} localhost

      # Reverse proxy settings (uncomment if behind a reverse proxy)
      # OVERWRITEPROTOCOL: https
      # OVERWRITEHOST: ${NEXTCLOUD_DOMAIN}
      # OVERWRITECLIURL: https://${NEXTCLOUD_DOMAIN}
      # TRUSTED_PROXIES: 172.18.0.0/16

      # PHP tuning
      PHP_MEMORY_LIMIT: 1G
      PHP_UPLOAD_LIMIT: 16G
    depends_on:
      nextcloud-db:
        condition: service_healthy
      nextcloud-redis:
        condition: service_healthy
    networks:
      - nextcloud

  nextcloud-cron:
    image: nextcloud:29-apache
    container_name: nextcloud-cron
    restart: unless-stopped
    volumes:
      - /opt/nextcloud/html:/var/www/html
      - /opt/nextcloud/data:/var/www/html/data
    entrypoint: /cron.sh
    depends_on:
      - nextcloud
    networks:
      - nextcloud

networks:
  nextcloud:
    name: nextcloud
    driver: bridge
```

## .env file

Place this alongside docker-compose.yml:

```env
MYSQL_ROOT_PASSWORD=change-me-root-password
MYSQL_PASSWORD=change-me-nextcloud-password
NEXTCLOUD_ADMIN_USER=admin
NEXTCLOUD_ADMIN_PASSWORD=change-me-admin-password
NEXTCLOUD_DOMAIN=cloud.example.com
```

## Notes

### Volume layout

| Host path | Container path | Purpose |
|---|---|---|
| `/opt/nextcloud/html` | `/var/www/html` | Nextcloud application files, config, apps |
| `/opt/nextcloud/data` | `/var/www/html/data` | User files (largest volume) |
| `/opt/nextcloud/db` | `/var/lib/mysql` | MariaDB data |
| `/opt/nextcloud/redis` | `/data` | Redis persistence (optional but recommended) |

### MariaDB command flags

- `--transaction-isolation=READ-COMMITTED` -- required by Nextcloud for proper transaction handling.
- `--log-bin=OFF` -- disables binary logging (not needed for single-server setups, saves disk I/O).
- `--innodb-read-only-compressed=OFF` -- required for MariaDB 10.6+ compatibility with some Nextcloud tables.

### Cron sidecar

The `nextcloud-cron` container shares the same volumes as the main Nextcloud container. It runs `/cron.sh` which executes `php -f /var/www/html/cron.php` every 5 minutes. This is more reliable than host crontab because it has direct access to the PHP environment and all Nextcloud files.

### Image version pinning

The example uses `nextcloud:29-apache`. Pin to the current major version to prevent accidental major upgrades. When ready to upgrade, change the tag to the next major version (e.g., `30-apache`) and follow the upgrade procedure in the main skill.

### Reverse proxy

If Nextcloud is accessed directly (no reverse proxy), the compose file works as-is on port 8080. If behind a reverse proxy (Traefik, Caddy, nginx), uncomment the `OVERWRITEPROTOCOL`, `OVERWRITEHOST`, `OVERWRITECLIURL`, and `TRUSTED_PROXIES` lines.

For Traefik, add labels instead of port mapping:

```yaml
nextcloud:
  # ... (remove ports section)
  labels:
    - "traefik.enable=true"
    - "traefik.http.routers.nextcloud.rule=Host(`cloud.example.com`)"
    - "traefik.http.routers.nextcloud.tls.certresolver=letsencrypt"
    - "traefik.http.routers.nextcloud.middlewares=nextcloud-redirects"
    - "traefik.http.middlewares.nextcloud-redirects.redirectregex.permanent=true"
    - "traefik.http.middlewares.nextcloud-redirects.redirectregex.regex=https://(.*)/.well-known/(?:card|cal)dav"
    - "traefik.http.middlewares.nextcloud-redirects.redirectregex.replacement=https://$${1}/remote.php/dav"
```

### Network

All services are on a single `nextcloud` bridge network. Services communicate by container name (e.g., `nextcloud-db`, `nextcloud-redis`). No ports are exposed for MariaDB or Redis -- they are only accessible within the Docker network.

### Healthchecks

MariaDB and Redis have healthchecks. Nextcloud depends on both being healthy before starting. This prevents Nextcloud from failing on first boot because the database is not ready yet.
