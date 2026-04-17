---
name: nextcloud
description: Deploy and configure a self-hosted Nextcloud instance with MariaDB, Redis, and cron. Covers image selection, reverse proxy integration, performance tuning, occ administration, upgrades, and common failure diagnosis.
version: 1.0.0
---

# Nextcloud Deployment

## Purpose

Use this skill when a Spacebot agent needs to deploy, configure, or troubleshoot a Nextcloud instance. Nextcloud is a self-hosted file sync and share platform with support for calendars, contacts, document editing, and extensible apps.

This playbook covers:

- **Image selection** -- choosing between `nextcloud:apache` and `nextcloud:fpm` based on requirements.
- **Database backend** -- MariaDB (recommended) or PostgreSQL. Never SQLite for production.
- **Redis** -- essential companion for file locking and session/memory caching.
- **Cron** -- reliable background job execution via sidecar container.
- **Reverse proxy integration** -- trusted proxies, protocol overwrite, trusted domains.
- **Administration** -- the `occ` CLI for maintenance, repair, and upgrades.
- **Performance tuning** -- PHP limits, Redis caching, database indices, preview generation.

## When to invoke this skill

- User wants to deploy Nextcloud (file sync, cloud storage, self-hosted Google Drive alternative).
- User wants to add MariaDB/PostgreSQL and Redis alongside Nextcloud.
- User needs to configure Nextcloud behind a reverse proxy (Traefik, Caddy, nginx).
- Nextcloud is showing errors related to trusted domains, file locking, cron, or performance.
- User wants to upgrade Nextcloud to a newer major version.
- User needs to run `occ` commands for maintenance or repair.
- User wants to add Collabora or OnlyOffice for online document editing.

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
| `<DOCKER_HOST>` | Docker host config name in Spacebot | `cloud1` |
| `<NEXTCLOUD_ROOT>` | Base directory for Nextcloud on host | `/opt/nextcloud` |
| `<NEXTCLOUD_DATA>` | User data directory on host | `/opt/nextcloud/data` |
| `<PUID>` | User ID (www-data = 33 inside official image) | `33` |
| `<PGID>` | Group ID | `33` |
| `<TZ>` | Timezone | `America/New_York` |
| `<MYSQL_ROOT_PASSWORD>` | MariaDB root password | (generated) |
| `<MYSQL_PASSWORD>` | Nextcloud database user password | (generated) |
| `<NEXTCLOUD_ADMIN_USER>` | Initial admin username | `admin` |
| `<NEXTCLOUD_ADMIN_PASSWORD>` | Initial admin password | (generated) |
| `<NEXTCLOUD_DOMAIN>` | Primary access domain | `cloud.example.com` |

## High-confidence lessons learned

These represent the most common mistakes and best practices for Nextcloud deployments. Ordered by frequency of occurrence.

### 1. Database: always use MariaDB or PostgreSQL, never SQLite

SQLite is only suitable for testing with a single user. Nextcloud itself warns against it. MariaDB is the most widely tested database backend for Nextcloud and the recommended choice for homelabs. PostgreSQL is also fully supported.

Deploy MariaDB as a companion container on the same Docker network. Use a dedicated database and user for Nextcloud.

### 2. Redis is essential

Redis serves two critical functions in Nextcloud:

- **Transactional file locking** -- prevents data corruption when multiple clients sync simultaneously. Without Redis, you get file locking errors and potential data loss.
- **Memory caching** -- accelerates page loads by caching frequently accessed data.

Deploy Redis as a companion container. Nextcloud connects via the `REDIS_HOST` environment variable on first install. The official image auto-configures `config.php` for Redis file locking and caching when `REDIS_HOST` is set.

### 3. Trusted domains must be configured

Nextcloud rejects HTTP requests where the `Host` header does not match an entry in the `trusted_domains` array. This is a security feature.

Set trusted domains at first install via the `NEXTCLOUD_TRUSTED_DOMAINS` environment variable (space-separated list). To add domains later, edit `config.php` or use occ:

```
ssh.exec(host="<DOCKER_HOST>", command="docker exec -u www-data nextcloud php occ config:system:set trusted_domains 2 --value=cloud.example.com")
```

### 4. Cron via system cron, not AJAX

The default AJAX-based background job trigger is unreliable -- it only fires when a user visits the web UI. For a properly functioning Nextcloud, background jobs must run every 5 minutes via system cron.

The recommended approach is a sidecar container using the same Nextcloud image:

```yaml
nextcloud-cron:
  image: nextcloud:apache
  restart: unless-stopped
  volumes_from:
    - nextcloud
  entrypoint: /cron.sh
  depends_on:
    - nextcloud
```

After deployment, set the background job mode to "Cron" in Administration Settings > Basic settings, or via occ:

```
ssh.exec(host="<DOCKER_HOST>", command="docker exec -u www-data nextcloud php occ background:cron")
```

### 5. Data directory placement

The default data directory is `/var/www/html/data` inside the container. For easier backup and management, mount user data on a separate volume from the Nextcloud application files:

- `/opt/nextcloud/html` -> `/var/www/html` (application files, config)
- `/opt/nextcloud/data` -> `/var/www/html/data` (user files)

This allows backing up user data independently from application state. The data directory can also be placed on a larger/slower disk while the app directory stays on fast storage.

### 6. Reverse proxy headers

Nextcloud behind a reverse proxy requires specific environment variables to avoid redirect loops and mixed content warnings:

| Variable | Purpose | Example |
|---|---|---|
| `OVERWRITEPROTOCOL` | Force HTTPS in generated URLs | `https` |
| `OVERWRITEHOST` | Override detected hostname | `cloud.example.com` |
| `OVERWRITECLIURL` | Full URL for CLI operations | `https://cloud.example.com` |
| `TRUSTED_PROXIES` | IP/CIDR of reverse proxy | `172.18.0.0/16` |

If using a Docker network, set `TRUSTED_PROXIES` to the Docker network subnet. Without these settings, Nextcloud generates HTTP links behind an HTTPS proxy, causing redirect loops.

### 7. PHP memory limit

Default is 512MB. For large file operations, many concurrent users, or heavy app usage, increase to 1GB or more:

```yaml
environment:
  - PHP_MEMORY_LIMIT=1G
```

Signs of insufficient memory: white pages, 500 errors during large uploads, or PHP fatal errors in logs.

### 8. Max upload size

The default upload limit is typically 512MB. For file sync use cases, increase substantially:

```yaml
environment:
  - PHP_UPLOAD_LIMIT=16G
```

This sets both `upload_max_filesize` and `post_max_size` in PHP. Note: if behind a reverse proxy, the proxy must also allow large request bodies (e.g., `client_max_body_size` in nginx, or no limit in Caddy).

### 9. Background jobs are critical

Nextcloud relies on background jobs for:
- File scanning and cleanup
- Notification delivery
- Trash and version expiry
- App updates checks
- Activity log processing
- External storage sync

If cron is not running, these tasks accumulate and the instance degrades over time. Always verify cron is working after deployment.

### 10. The occ command

`occ` is Nextcloud's admin CLI. It must be run as the `www-data` user inside the container:

```
docker exec -u www-data nextcloud php occ <command>
```

Essential occ commands:

| Command | Purpose |
|---|---|
| `occ status` | Show Nextcloud version and status |
| `occ maintenance:mode --on/--off` | Enable/disable maintenance mode |
| `occ maintenance:repair` | Repair the installation |
| `occ db:add-missing-indices` | Add missing database indices (performance) |
| `occ db:add-missing-columns` | Add missing database columns |
| `occ db:convert-filecache-bigint` | Convert filecache to bigint (required for large installs) |
| `occ files:scan --all` | Re-scan all user files |
| `occ files:cleanup` | Clean up orphaned file entries |
| `occ upgrade` | Run upgrade routines after image update |
| `occ config:system:set <key> --value=<val>` | Set config.php values |
| `occ config:system:get <key>` | Read config.php values |
| `occ app:list` | List installed apps |
| `occ app:enable <app>` | Enable an app |
| `occ app:disable <app>` | Disable an app |
| `occ user:list` | List all users |

## Decision trees

### Image choice

**`nextcloud:apache` (recommended for most homelabs):**
- Built-in Apache web server. Single container serves the application.
- Simplest deployment. Fewer moving parts.
- Suitable for small to medium installs (1-50 users).
- Use this unless you have a specific reason not to.

**`nextcloud:fpm`:**
- PHP-FPM only. Requires a separate web server container (nginx or Caddy).
- Slightly better performance under high concurrency due to PHP-FPM process management.
- More complex: two containers instead of one, plus an nginx config to maintain.
- Choose this if you already run nginx as a reverse proxy and want to terminate TLS + serve static files directly via nginx.

Recommendation: start with `nextcloud:apache`. Switch to `fpm` only if performance profiling shows a bottleneck at the web server layer.

### Database choice

**MariaDB (recommended):**
- Most tested and widely used with Nextcloud.
- Best community support and documentation.
- Use `mariadb:lts` image.

**PostgreSQL:**
- Fully supported. Some users prefer it for its advanced features.
- Use `postgres:16-alpine` or similar.
- Slightly different occ commands for some database operations.

**SQLite:**
- Testing only. Do not use in production. Nextcloud displays a persistent warning.

### Redis password

For single-host homelab setups where Redis is on an isolated Docker network, running Redis without a password is acceptable. For multi-host or exposed deployments, set a password:

```yaml
# Redis
command: redis-server --requirepass <REDIS_PASSWORD>

# Nextcloud
environment:
  - REDIS_HOST_PASSWORD=<REDIS_PASSWORD>
```

## Service configuration details

### Nextcloud

| Setting | Value |
|---------|-------|
| Image | `nextcloud:apache` |
| Default port | 80 (HTTP) |
| App volume | `/opt/nextcloud/html` -> `/var/www/html` |
| Data volume | `/opt/nextcloud/data` -> `/var/www/html/data` |
| Config location | `/var/www/html/config/config.php` |

### MariaDB

| Setting | Value |
|---------|-------|
| Image | `mariadb:lts` |
| Default port | 3306 (internal only, no host mapping needed) |
| Data volume | `/opt/nextcloud/db` -> `/var/lib/mysql` |
| Required env | `MYSQL_ROOT_PASSWORD`, `MYSQL_DATABASE`, `MYSQL_USER`, `MYSQL_PASSWORD` |

### Redis

| Setting | Value |
|---------|-------|
| Image | `redis:alpine` |
| Default port | 6379 (internal only) |
| Data volume | `/opt/nextcloud/redis` -> `/data` (optional persistence) |

## Safety rules

1. **Always enable maintenance mode before major upgrades.** Run `occ maintenance:mode --on` before pulling a new Nextcloud image. This prevents user access during the upgrade and avoids database corruption.

2. **Back up database AND data directory before upgrades.** Database backup:
   ```
   ssh.exec(host="<DOCKER_HOST>", command="docker exec nextcloud-db mariadb-dump -u root -p<MYSQL_ROOT_PASSWORD> nextcloud > /opt/nextcloud/backup-$(date +%Y%m%d).sql")
   ```
   Data backup: snapshot or rsync `/opt/nextcloud/data`.

3. **Never skip major versions during upgrade.** Nextcloud requires sequential major version upgrades: 27 -> 28 -> 29, never 27 -> 29. Each major version must complete its migration before proceeding. Check current version with `occ status`.

4. **Use `confirm_operation` before any destructive occ commands.** This includes `occ maintenance:repair`, `occ files:cleanup`, `occ trashbin:cleanup`, `occ user:delete`, and database operations.

5. **Do not change the data directory after initial setup without careful migration.** Moving the data directory requires updating `config.php`, moving all files, and fixing permissions. It is error-prone. Plan the data directory location before first install.

6. **Never run `docker compose down -v` on the Nextcloud stack.** This deletes volumes including the database. Always use `docker compose down` without `-v` unless explicitly confirmed via `confirm_operation`.

7. **Do not expose Nextcloud directly to the internet without HTTPS.** Always place behind a reverse proxy with TLS, or use the built-in Let's Encrypt support (not recommended for Docker setups).

8. **Pin the major version tag for stability.** Use `nextcloud:29-apache` instead of `nextcloud:apache` to prevent accidental major version jumps on `docker compose pull`. Update the tag deliberately when ready to upgrade.

## Recommended procedural flow for agents

### Full deployment

1. **Gather requirements:**
   - Target Docker host
   - Domain name for Nextcloud
   - Data storage location and size requirements
   - Whether a reverse proxy is already in place
   - Admin username preference
   - Timezone

2. **Create directory structure on host:**
   ```
   ssh.exec(host="<DOCKER_HOST>", command="mkdir -p /opt/nextcloud/{html,data,db,redis}")
   ssh.exec(host="<DOCKER_HOST>", command="chown -R 33:33 /opt/nextcloud/html /opt/nextcloud/data")
   ```

3. **Check for port conflicts:**
   ```
   ssh.exec(host="<DOCKER_HOST>", command="ss -tlnp | grep -E ':(8443|8080|3306) '")
   ```

4. **Generate passwords:**
   ```
   ssh.exec(host="<DOCKER_HOST>", command="openssl rand -base64 32")
   ```
   Generate separate passwords for: MariaDB root, MariaDB Nextcloud user, Nextcloud admin.

5. **Upload compose file:**
   Generate from the reference compose (see `references/compose-example.md`), substituting environment variables. Upload:
   ```
   ssh.upload(host="<DOCKER_HOST>", local_path="./docker-compose.yml", remote_path="/opt/nextcloud/docker-compose.yml")
   ```

6. **Pull all images:**
   ```
   ssh.exec(host="<DOCKER_HOST>", command="docker compose pull", cwd="/opt/nextcloud")
   ```

7. **Deploy the stack:**
   ```
   ssh.exec(host="<DOCKER_HOST>", command="docker compose up -d", cwd="/opt/nextcloud")
   ```

8. **Wait for first-boot initialization (1-2 minutes):**
   Nextcloud runs database migrations and installs default apps on first start. Monitor progress:
   ```
   docker.container.logs(host="<DOCKER_HOST>", name="nextcloud", tail=50)
   ```
   Wait until logs show "Initializing finished" or Apache starts listening.

9. **Verify containers are running:**
   ```
   ssh.exec(host="<DOCKER_HOST>", command="docker compose ps", cwd="/opt/nextcloud")
   ```

10. **Run post-install maintenance:**
    ```
    ssh.exec(host="<DOCKER_HOST>", command="docker exec -u www-data nextcloud php occ maintenance:repair")
    ssh.exec(host="<DOCKER_HOST>", command="docker exec -u www-data nextcloud php occ db:add-missing-indices")
    ssh.exec(host="<DOCKER_HOST>", command="docker exec -u www-data nextcloud php occ db:add-missing-columns")
    ssh.exec(host="<DOCKER_HOST>", command="docker exec -u www-data nextcloud php occ db:convert-filecache-bigint")
    ```

11. **Set background jobs to Cron:**
    ```
    ssh.exec(host="<DOCKER_HOST>", command="docker exec -u www-data nextcloud php occ background:cron")
    ```

12. **Verify cron is working:**
    Wait 5 minutes, then check:
    ```
    ssh.exec(host="<DOCKER_HOST>", command="docker exec -u www-data nextcloud php occ config:system:get maintenance_window_start")
    ssh.exec(host="<DOCKER_HOST>", command="docker logs nextcloud-cron --tail 5")
    ```

13. **Verify web access:**
    ```
    ssh.exec(host="<DOCKER_HOST>", command="curl -sI http://localhost:8080 | head -5")
    ```
    Should return HTTP 200 or 302 redirect to login page.

14. **Configure reverse proxy settings (if applicable):**
    If behind a reverse proxy, set overwrite variables. These should already be in the compose env, but verify:
    ```
    ssh.exec(host="<DOCKER_HOST>", command="docker exec -u www-data nextcloud php occ config:system:get overwriteprotocol")
    ssh.exec(host="<DOCKER_HOST>", command="docker exec -u www-data nextcloud php occ config:system:get overwrite.cli.url")
    ```

15. **(Optional) Deploy Collabora or OnlyOffice for document editing:**
    This requires an additional container and Nextcloud app. Outside the scope of this core playbook but can be added as an extension.

### Upgrading Nextcloud

1. **Check current and target versions:**
   ```
   ssh.exec(host="<DOCKER_HOST>", command="docker exec -u www-data nextcloud php occ status")
   ```
   Determine the next major version. Remember: sequential upgrades only.

2. **Enable maintenance mode:**
   ```
   ssh.exec(host="<DOCKER_HOST>", command="docker exec -u www-data nextcloud php occ maintenance:mode --on")
   ```

3. **Back up database:**
   ```
   ssh.exec(host="<DOCKER_HOST>", command="docker exec -e MYSQL_PWD=<MYSQL_ROOT_PASSWORD> nextcloud-db mariadb-dump -u root nextcloud | gzip > /opt/nextcloud/backup-pre-upgrade-$(date +%Y%m%d).sql.gz")
   ```

4. **Back up data (or ensure recent snapshot exists):**
   ```
   ssh.exec(host="<DOCKER_HOST>", command="rsync -a /opt/nextcloud/data/ /opt/nextcloud/data-backup-$(date +%Y%m%d)/")
   ```

5. **Update the image tag in compose file** to the next major version (e.g., `nextcloud:29-apache` -> `nextcloud:30-apache`).

6. **Pull and recreate:**
   ```
   ssh.exec(host="<DOCKER_HOST>", command="docker compose pull && docker compose up -d", cwd="/opt/nextcloud")
   ```

7. **Run upgrade if not automatic:**
   ```
   ssh.exec(host="<DOCKER_HOST>", command="docker exec -u www-data nextcloud php occ upgrade")
   ```

8. **Disable maintenance mode:**
   ```
   ssh.exec(host="<DOCKER_HOST>", command="docker exec -u www-data nextcloud php occ maintenance:mode --off")
   ```

9. **Post-upgrade maintenance:**
   ```
   ssh.exec(host="<DOCKER_HOST>", command="docker exec -u www-data nextcloud php occ maintenance:repair")
   ssh.exec(host="<DOCKER_HOST>", command="docker exec -u www-data nextcloud php occ db:add-missing-indices")
   ssh.exec(host="<DOCKER_HOST>", command="docker exec -u www-data nextcloud php occ db:add-missing-columns")
   ```

10. **Verify:**
    ```
    ssh.exec(host="<DOCKER_HOST>", command="docker exec -u www-data nextcloud php occ status")
    ```

### Adding a service to an existing stack

1. Add the new service definition to the compose file.
2. Upload the updated compose file.
3. Run `docker compose up -d` -- Compose only recreates changed services.
4. Configure the new service as needed.

## Fast diagnosis cheatsheet

| Symptom | Most likely cause | Fix |
|---|---|---|
| "Access through untrusted domain" | `trusted_domains` not configured for the access URL | Add domain via `occ config:system:set trusted_domains N --value=<domain>` |
| Redirect loop behind reverse proxy | `OVERWRITEPROTOCOL` and/or `OVERWRITEHOST` not set | Set `OVERWRITEPROTOCOL=https` and `TRUSTED_PROXIES` to proxy IP/subnet |
| File locking errors / "file is locked" | Redis not configured or Redis container not running | Verify Redis container is up; check `REDIS_HOST` env var; inspect config.php for `memcache.locking` |
| Slow page loads | Redis missing, low PHP memory, or missing database indices | Add Redis, increase `PHP_MEMORY_LIMIT`, run `occ db:add-missing-indices` |
| Upload fails for large files | `PHP_UPLOAD_LIMIT` too low, or reverse proxy body size limit | Increase `PHP_UPLOAD_LIMIT` env var; increase proxy `client_max_body_size` |
| Cron jobs not running / "Last job ran X hours ago" | Still on AJAX cron mode | Deploy cron sidecar; run `occ background:cron`; verify cron container logs |
| "Maintenance mode" stuck after failed upgrade | Maintenance mode not disabled | Run `occ maintenance:mode --off` |
| Database errors after upgrade | Missing indices or columns from migration | Run `occ db:add-missing-indices`, `occ db:add-missing-columns`, `occ maintenance:repair` |
| White page / 500 error | PHP memory exhaustion or config.php syntax error | Increase `PHP_MEMORY_LIMIT`; check `config.php` for syntax errors; check container logs |
| "Your data directory is readable by other users" | Permissions too open on data directory | `chmod 770 /opt/nextcloud/data`; ensure owned by `33:33` (www-data) |
| App store shows "Could not connect" | Nextcloud cannot reach the internet | Check container DNS; verify outbound connectivity: `docker exec nextcloud curl -s https://apps.nextcloud.com` |
| "The reverse proxy header configuration is incorrect" | `TRUSTED_PROXIES` not set or wrong subnet | Set `TRUSTED_PROXIES` to the Docker network subnet (e.g., `172.18.0.0/16`) |
| Previews not generating | Preview generator not configured or OOM | Install Preview Generator app; increase PHP memory; see performance tuning reference |
| CalDAV/CardDAV not syncing | Well-known URLs not redirected | Reverse proxy must redirect `/.well-known/caldav` and `/.well-known/carddav` to `/remote.php/dav` |

## References

See supporting reference docs in `references/`:

- `compose-example.md` -- Complete production docker-compose.yml with Nextcloud, MariaDB, Redis, and cron sidecar.
- `performance-tuning.md` -- PHP, Redis, MariaDB, and cron tuning for optimal Nextcloud performance.
