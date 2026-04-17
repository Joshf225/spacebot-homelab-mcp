# Nextcloud Performance Tuning

Practical tuning guide for Nextcloud running in Docker with MariaDB and Redis.

## PHP tuning

### Memory limit

Set via environment variable in the Nextcloud container:

```yaml
environment:
  PHP_MEMORY_LIMIT: 1G
```

Guidelines:
- 512MB: minimum, fine for 1-5 users with small files
- 1GB: recommended for most homelab setups
- 2GB+: large installs, heavy app usage, or preview generation

### Upload limit

```yaml
environment:
  PHP_UPLOAD_LIMIT: 16G
```

This controls the maximum file size for single-file uploads. Set it to the largest file you expect to upload. The Nextcloud desktop and mobile clients use chunked uploads, so this primarily affects the web UI.

If behind a reverse proxy, the proxy must also allow large request bodies:
- **nginx**: `client_max_body_size 16G;`
- **Caddy**: No limit by default (no change needed).
- **Traefik**: Set `traefik.http.middlewares.<name>.buffering.maxRequestBodyBytes` or use streaming.

### OPcache

The official Nextcloud Docker image configures OPcache by default. Verify it is enabled:

```
ssh.exec(host="<DOCKER_HOST>", command="docker exec nextcloud php -i | grep opcache.enable")
```

For custom tuning (rarely needed):

```ini
opcache.enable=1
opcache.interned_strings_buffer=16
opcache.max_accelerated_files=10000
opcache.memory_consumption=256
opcache.save_comments=1
opcache.revalidate_freq=1
```

Mount a custom PHP ini file if needed:
```yaml
volumes:
  - ./custom-php.ini:/usr/local/etc/php/conf.d/zzz-custom.ini:ro
```

## Redis configuration

### Cache tiers

Nextcloud supports multiple cache layers. The optimal configuration uses Redis for distributed/file locking and APCu for local (per-request) caching:

```php
// config.php (auto-configured when REDIS_HOST is set)
'memcache.local' => '\\OC\\Memcache\\APCu',
'memcache.distributed' => '\\OC\\Memcache\\Redis',
'memcache.locking' => '\\OC\\Memcache\\Redis',
'redis' => [
    'host' => 'nextcloud-redis',
    'port' => 6379,
],
```

The official Docker image sets `memcache.distributed` and `memcache.locking` to Redis when `REDIS_HOST` is provided. APCu for `memcache.local` is included in the image and enabled by default.

To verify the cache configuration:
```
ssh.exec(host="<DOCKER_HOST>", command="docker exec -u www-data nextcloud php occ config:system:get memcache.local")
ssh.exec(host="<DOCKER_HOST>", command="docker exec -u www-data nextcloud php occ config:system:get memcache.distributed")
ssh.exec(host="<DOCKER_HOST>", command="docker exec -u www-data nextcloud php occ config:system:get memcache.locking")
```

### Redis memory policy

For a dedicated Nextcloud Redis instance, configure a memory limit and eviction policy:

```yaml
nextcloud-redis:
  image: redis:alpine
  command: redis-server --maxmemory 256mb --maxmemory-policy allkeys-lru --save 60 1
```

- `--maxmemory 256mb`: sufficient for most homelab installs. Increase for large user bases.
- `--maxmemory-policy allkeys-lru`: evict least recently used keys when memory is full.
- `--save 60 1`: persist to disk every 60 seconds if at least 1 key changed. Optional but prevents cache cold start.

### Monitoring Redis

Check Redis memory usage:
```
ssh.exec(host="<DOCKER_HOST>", command="docker exec nextcloud-redis redis-cli info memory | grep used_memory_human")
```

Check connected clients:
```
ssh.exec(host="<DOCKER_HOST>", command="docker exec nextcloud-redis redis-cli info clients | grep connected_clients")
```

## MariaDB tuning

### InnoDB buffer pool

The single most impactful MariaDB tuning parameter. Set to ~70% of available RAM dedicated to the database container, or at minimum 256MB for small installs:

```yaml
nextcloud-db:
  command: >-
    --transaction-isolation=READ-COMMITTED
    --log-bin=OFF
    --innodb-read-only-compressed=OFF
    --innodb-buffer-pool-size=512M
    --innodb-log-file-size=64M
    --innodb-flush-log-at-trx-commit=2
    --innodb-flush-method=O_DIRECT
```

Parameter explanations:
- `--innodb-buffer-pool-size=512M`: cache for table and index data. Bigger = fewer disk reads. 256MB minimum, 512MB-1GB recommended for homelab.
- `--innodb-log-file-size=64M`: redo log size. Larger logs improve write performance but increase recovery time. 64MB is a good default.
- `--innodb-flush-log-at-trx-commit=2`: flush logs once per second instead of every transaction. Slight durability risk (up to 1 second of data loss on crash) but significant write performance improvement.
- `--innodb-flush-method=O_DIRECT`: bypass OS cache for InnoDB data files. Reduces double-buffering since InnoDB has its own buffer pool.

### Slow query log

Enable temporarily for diagnosis:
```
ssh.exec(host="<DOCKER_HOST>", command="docker exec nextcloud-db mariadb -u root -p<PASSWORD> -e \"SET GLOBAL slow_query_log=1; SET GLOBAL long_query_time=1;\"")
```

Check slow queries:
```
ssh.exec(host="<DOCKER_HOST>", command="docker exec nextcloud-db mariadb -u root -p<PASSWORD> -e \"SHOW GLOBAL STATUS LIKE 'Slow_queries';\"")
```

Disable when done to avoid log growth.

## Database index optimization

After every Nextcloud install or upgrade, run:

```
ssh.exec(host="<DOCKER_HOST>", command="docker exec -u www-data nextcloud php occ db:add-missing-indices")
ssh.exec(host="<DOCKER_HOST>", command="docker exec -u www-data nextcloud php occ db:add-missing-columns")
```

For large installs (many files), convert the filecache to use bigint IDs:

```
ssh.exec(host="<DOCKER_HOST>", command="docker exec -u www-data nextcloud php occ db:convert-filecache-bigint")
```

This can take a long time on large databases. Run during a maintenance window with maintenance mode enabled.

## Cron optimization

### Sidecar approach (recommended)

The cron sidecar container runs `/cron.sh` which executes `cron.php` every 5 minutes. This is the most reliable approach for Docker deployments.

Verify cron is executing:
```
ssh.exec(host="<DOCKER_HOST>", command="docker exec -u www-data nextcloud php occ config:app:get core lastcron")
```

The returned timestamp should be within the last 5 minutes.

### Host crontab alternative

If you prefer not to run a sidecar container:

```
ssh.exec(host="<DOCKER_HOST>", command="echo '*/5 * * * * docker exec -u www-data nextcloud php -f /var/www/html/cron.php' | crontab -")
```

The sidecar approach is preferred because it shares the exact same PHP environment as Nextcloud.

## Preview generation

Nextcloud generates thumbnails for images, videos, and documents on demand. This causes slow first loads. The Preview Generator app pre-generates previews in the background.

### Install and configure

```
ssh.exec(host="<DOCKER_HOST>", command="docker exec -u www-data nextcloud php occ app:enable previewgenerator")
```

Configure preview sizes (optional -- reduce for storage savings):

```
ssh.exec(host="<DOCKER_HOST>", command="docker exec -u www-data nextcloud php occ config:system:set preview_max_x --value=2048")
ssh.exec(host="<DOCKER_HOST>", command="docker exec -u www-data nextcloud php occ config:system:set preview_max_y --value=2048")
ssh.exec(host="<DOCKER_HOST>", command="docker exec -u www-data nextcloud php occ config:system:set jpeg_quality --value=60")
```

Disable preview providers you do not need to save CPU and storage:

```
ssh.exec(host="<DOCKER_HOST>", command="docker exec -u www-data nextcloud php occ config:system:set enabledPreviewProviders 0 --value='OC\\Preview\\PNG'")
ssh.exec(host="<DOCKER_HOST>", command="docker exec -u www-data nextcloud php occ config:system:set enabledPreviewProviders 1 --value='OC\\Preview\\JPEG'")
ssh.exec(host="<DOCKER_HOST>", command="docker exec -u www-data nextcloud php occ config:system:set enabledPreviewProviders 2 --value='OC\\Preview\\GIF'")
ssh.exec(host="<DOCKER_HOST>", command="docker exec -u www-data nextcloud php occ config:system:set enabledPreviewProviders 3 --value='OC\\Preview\\MP4'")
```

### Initial generation

Generate all previews (can take hours on large libraries):

```
ssh.exec(host="<DOCKER_HOST>", command="docker exec -u www-data nextcloud php occ preview:generate-all")
```

### Ongoing generation

Add to cron (run after the regular cron job). Add a second crontab entry on the host, or extend the cron sidecar:

```
# Every 10 minutes, generate previews for newly added files
*/10 * * * * docker exec -u www-data nextcloud php occ preview:pre-generate
```

## File scan optimization

After bulk file additions (e.g., copying files directly to the data directory), run a file scan:

```
ssh.exec(host="<DOCKER_HOST>", command="docker exec -u www-data nextcloud php occ files:scan --all")
```

For a specific user:
```
ssh.exec(host="<DOCKER_HOST>", command="docker exec -u www-data nextcloud php occ files:scan <username>")
```

For very large libraries, scan specific paths to avoid scanning everything:
```
ssh.exec(host="<DOCKER_HOST>", command="docker exec -u www-data nextcloud php occ files:scan --path='/<username>/files/Photos'")
```

## PHP-FPM tuning (fpm image only)

If using `nextcloud:fpm`, PHP-FPM process management can be tuned. Create a custom FPM config:

```ini
; /usr/local/etc/php-fpm.d/zzz-custom.conf
[www]
pm = dynamic
pm.max_children = 30
pm.start_servers = 5
pm.min_spare_servers = 3
pm.max_spare_servers = 10
pm.max_requests = 500
```

Mount into the container:
```yaml
volumes:
  - ./zzz-custom-fpm.conf:/usr/local/etc/php-fpm.d/zzz-custom.conf:ro
```

Guidelines:
- `pm.max_children`: maximum concurrent PHP processes. Each uses ~50-100MB RAM. Set based on available memory.
- `pm.max_requests`: restart workers after N requests to prevent memory leaks.
- For low-traffic homelabs, `pm = ondemand` with `pm.max_children = 10` saves memory by spawning workers only when needed.

## Summary checklist

| Item | Setting | Default | Recommended |
|---|---|---|---|
| PHP memory limit | `PHP_MEMORY_LIMIT` | 512M | 1G |
| Upload limit | `PHP_UPLOAD_LIMIT` | 512M | 16G |
| Redis memory | `--maxmemory` | unlimited | 256mb |
| InnoDB buffer pool | `--innodb-buffer-pool-size` | 128M | 512M-1G |
| InnoDB flush commit | `--innodb-flush-log-at-trx-commit` | 1 | 2 |
| Background jobs | Admin > Basic settings | AJAX | Cron |
| Database indices | `occ db:add-missing-indices` | -- | Run after install/upgrade |
| Preview generation | Preview Generator app | On-demand | Pre-generate |
