# Volume Strategies

## Bind mounts vs named volumes

| | Bind mounts | Named volumes |
|---|---|---|
| **Managed by** | You (filesystem path) | Docker |
| **Location** | Any host path you specify | `/var/lib/docker/volumes/<name>/_data` |
| **Direct host access** | Yes | Possible but not recommended |
| **Backup** | Standard filesystem tools | Requires `docker run` with a helper container |
| **Use case** | Config files, media, anything you need to see/edit | Database data, opaque application state |
| **Compose syntax** | `./config:/config` | `db-data:/var/lib/postgresql/data` |

## The /opt/docker/ bind mount convention

Standard directory layout for homelab Docker hosts:

```
/opt/docker/
├── jellyfin/
│   ├── docker-compose.yml
│   ├── .env
│   ├── config/                # → /config in container
│   └── cache/                 # → /cache in container
├── traefik/
│   ├── docker-compose.yml
│   ├── config/
│   │   ├── traefik.yml
│   │   └── dynamic/
│   └── certs/
├── gitea/
│   ├── docker-compose.yml
│   ├── .env
│   └── data/
├── postgres/
│   ├── docker-compose.yml
│   ├── .env
│   └── data/                  # Database files (or use named volume)
└── monitoring/
    ├── docker-compose.yml
    └── config/
        ├── prometheus.yml
        └── grafana/
```

### Why this layout works

1. **One directory per service** makes it obvious what's deployed.
2. **Compose file lives with the data** -- `docker compose up -d` from the service directory.
3. **Backups are trivial:** `tar czf /backup/docker-$(date +%Y%m%d).tar.gz /opt/docker/`.
4. **Migration is simple:** rsync `/opt/docker/` to a new host, `docker compose up -d` in each directory.

### Setting up a new service

```
ssh.exec(host="<DOCKER_HOST>", command="mkdir -p /opt/docker/<SERVICE_NAME>/{config,data}")
ssh.exec(host="<DOCKER_HOST>", command="chown -R 1000:1000 /opt/docker/<SERVICE_NAME>")
```

## Named volumes

### When to use

- **Databases** (Postgres, MariaDB, MongoDB, Redis) -- database files should not be manually touched.
- **Application internal state** that you never need to access from the host.

### Declaring in Compose

```yaml
services:
  db:
    volumes:
      - db-data:/var/lib/postgresql/data

volumes:
  db-data:          # Docker manages this
```

### Backing up named volumes

```bash
# Create a tar backup of a named volume
docker run --rm \
  -v db-data:/source:ro \
  -v /opt/docker/backups:/backup \
  busybox tar czf /backup/db-data-$(date +%Y%m%d).tar.gz -C /source .
```

### Restoring named volumes

```bash
# Restore from backup
docker run --rm \
  -v db-data:/target \
  -v /opt/docker/backups:/backup \
  busybox tar xzf /backup/db-data-20240115.tar.gz -C /target
```

## NFS mounts

### Host-level NFS mount (recommended)

Mount the NFS share on the Docker host, then bind-mount into containers:

```bash
# /etc/fstab
nas.home.arpa:/volume1/media  /mnt/media  nfs  defaults,_netdev,soft,timeo=150  0  0
```

Then in Compose:
```yaml
volumes:
  - /mnt/media/movies:/data/movies:ro
  - /mnt/media/tv:/data/tv:ro
```

**Advantages:**
- Simple and reliable.
- NFS connection is managed by the host OS.
- Multiple containers can share the same mount.

### Docker NFS volume driver (alternative)

```yaml
volumes:
  media:
    driver: local
    driver_opts:
      type: nfs
      o: addr=nas.home.arpa,soft,timeo=150,rw
      device: ":/volume1/media"

services:
  jellyfin:
    volumes:
      - media:/media:ro
```

**When to use:** When you can't or don't want to modify the host's `/etc/fstab`. The Docker daemon manages the NFS connection.

## Permission handling

### The PUID/PGID pattern

Many homelab images (LinuxServer.io, Hotio, etc.) accept environment variables to set the internal process UID/GID:

```yaml
environment:
  - PUID=1000
  - PGID=1000
```

The container's entrypoint script creates/modifies an internal user with the specified UID/GID, then runs the application as that user. This ensures files created in bind mounts match the host user's ownership.

### Finding the right PUID/PGID

On the Docker host:
```bash
# Check the user that should own the files
id username
# Output: uid=1000(username) gid=1000(username) ...
```

Use those values for PUID and PGID.

### Images without PUID/PGID support

For images that don't support the PUID/PGID pattern:

**Option 1: Set ownership on the host**
```bash
# Find the UID the container runs as
docker exec <container> id
# Output: uid=999(appuser) ...

# Set host directory ownership to match
chown -R 999:999 /opt/docker/<SERVICE_NAME>/data
```

**Option 2: Use the `user` directive in Compose**
```yaml
services:
  myapp:
    user: "1000:1000"
```

**Warning:** Not all images work with arbitrary UIDs. The application may expect specific users to exist inside the container.

### Common permission scenarios

| Scenario | Fix |
|----------|-----|
| Container writes files as root | Set PUID/PGID or use `user:` directive |
| Host user can't read container-created files | Match PUID/PGID to host UID/GID |
| Container can't write to bind mount | `chown` the directory to the container's UID or set PUID/PGID |
| NFS mount permission denied | Ensure NFS export allows the container's UID; or use `all_squash` with matching anonuid/anongid |

## Backup-friendly volume layout

### Full host backup

```bash
# Back up all Docker service configs and data
tar czf /backup/docker-full-$(date +%Y%m%d).tar.gz /opt/docker/

# Exclude large media directories if they're on NFS
tar czf /backup/docker-configs-$(date +%Y%m%d).tar.gz \
  --exclude='/opt/docker/*/media' \
  --exclude='/opt/docker/*/cache' \
  /opt/docker/
```

### Per-service backup

```bash
# Stop the service first for consistency (especially databases)
docker compose -f /opt/docker/<SERVICE_NAME>/docker-compose.yml down

# Back up
tar czf /backup/<SERVICE_NAME>-$(date +%Y%m%d).tar.gz /opt/docker/<SERVICE_NAME>/

# Restart
docker compose -f /opt/docker/<SERVICE_NAME>/docker-compose.yml up -d
```

### Database-safe backups

For databases, prefer a logical dump over filesystem copy:

```bash
# Postgres
docker exec postgres-container pg_dumpall -U postgres > /opt/docker/backups/pg-dump-$(date +%Y%m%d).sql

# MariaDB/MySQL
docker exec mariadb-container mysqldump -u root --all-databases > /opt/docker/backups/mysql-dump-$(date +%Y%m%d).sql
```

Logical dumps are portable, human-readable, and don't require stopping the database.
