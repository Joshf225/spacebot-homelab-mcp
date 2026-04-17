# Docker Compose Patterns

Common `docker-compose.yml` patterns for homelab services. All examples follow the `/opt/docker/<service>/` bind mount convention.

## Standard service pattern

The base template for most homelab services:

```yaml
services:
  servicename:
    image: image:tag
    container_name: servicename
    restart: unless-stopped
    ports:
      - "8080:8080"
    environment:
      - PUID=1000
      - PGID=1000
      - TZ=America/New_York
    volumes:
      - ./config:/config
      - ./data:/data
```

Key points:
- `container_name` makes it easier to reference in logs and commands.
- `restart: unless-stopped` survives reboots, respects manual stops.
- Relative volume paths (`./*`) resolve relative to the compose file location.

## Example 1: Media server with GPU transcoding

```yaml
# /opt/docker/jellyfin/docker-compose.yml
services:
  jellyfin:
    image: lscr.io/linuxserver/jellyfin:latest
    container_name: jellyfin
    restart: unless-stopped
    runtime: nvidia
    ports:
      - "8096:8096"
    environment:
      - PUID=1000
      - PGID=1000
      - TZ=America/New_York
      - NVIDIA_VISIBLE_DEVICES=all
    volumes:
      - ./config:/config
      - /mnt/media/movies:/data/movies:ro
      - /mnt/media/tv:/data/tv:ro
    deploy:
      resources:
        reservations:
          devices:
            - capabilities: [gpu]
```

Notes:
- Media directories mounted read-only (`:ro`) -- Jellyfin only needs to read media files.
- NFS media share mounted at the host level (`/mnt/media`), then bind-mounted into the container.
- GPU access via `runtime: nvidia` and the `deploy.resources.reservations.devices` block.

## Example 2: Reverse proxy with automatic TLS

```yaml
# /opt/docker/traefik/docker-compose.yml
services:
  traefik:
    image: traefik:v3.1
    container_name: traefik
    restart: unless-stopped
    ports:
      - "80:80"
      - "443:443"
      - "8080:8080"   # Dashboard (restrict in production)
    environment:
      - CF_DNS_API_TOKEN=${CF_DNS_API_TOKEN}
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock:ro
      - ./config/traefik.yml:/etc/traefik/traefik.yml:ro
      - ./config/dynamic:/etc/traefik/dynamic:ro
      - ./certs:/certs
    networks:
      - proxy-net

networks:
  proxy-net:
    name: proxy-net
    external: false
```

Notes:
- Docker socket mounted read-only for automatic service discovery.
- Sensitive values (API tokens) loaded from `.env` file via `${VAR}` syntax.
- Custom network `proxy-net` allows other stacks to join for reverse proxying.

## Example 3: Application + database stack

```yaml
# /opt/docker/gitea/docker-compose.yml
services:
  gitea:
    image: gitea/gitea:1.22
    container_name: gitea
    restart: unless-stopped
    ports:
      - "3000:3000"
      - "2222:22"
    environment:
      - USER_UID=1000
      - USER_GID=1000
      - GITEA__database__DB_TYPE=postgres
      - GITEA__database__HOST=gitea-db:5432
      - GITEA__database__NAME=gitea
      - GITEA__database__USER=gitea
      - GITEA__database__PASSWD=${DB_PASSWORD}
    volumes:
      - ./data:/data
    depends_on:
      gitea-db:
        condition: service_healthy
    networks:
      - gitea-net

  gitea-db:
    image: postgres:16-alpine
    container_name: gitea-db
    restart: unless-stopped
    environment:
      - POSTGRES_DB=gitea
      - POSTGRES_USER=gitea
      - POSTGRES_PASSWORD=${DB_PASSWORD}
    volumes:
      - db-data:/var/lib/postgresql/data
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U gitea"]
      interval: 10s
      timeout: 5s
      retries: 5
    networks:
      - gitea-net
    deploy:
      resources:
        limits:
          memory: 1G

volumes:
  db-data:

networks:
  gitea-net:
```

Notes:
- Database uses a named volume (`db-data`) -- don't touch database files directly.
- `depends_on` with `condition: service_healthy` ensures the database is ready before the app starts.
- Database has a memory limit to prevent runaway queries from consuming all host RAM.
- Both services share a private network (`gitea-net`) for internal communication.
- Sensitive values in `.env` file next to the compose file.

## Example 4: Monitoring stack

```yaml
# /opt/docker/monitoring/docker-compose.yml
services:
  prometheus:
    image: prom/prometheus:v2.53.0
    container_name: prometheus
    restart: unless-stopped
    ports:
      - "9090:9090"
    volumes:
      - ./config/prometheus.yml:/etc/prometheus/prometheus.yml:ro
      - prometheus-data:/prometheus
    command:
      - '--config.file=/etc/prometheus/prometheus.yml'
      - '--storage.tsdb.retention.time=30d'
    networks:
      - monitoring

  grafana:
    image: grafana/grafana:11.1.0
    container_name: grafana
    restart: unless-stopped
    ports:
      - "3000:3000"
    environment:
      - GF_SECURITY_ADMIN_PASSWORD=${GRAFANA_ADMIN_PASSWORD}
    volumes:
      - grafana-data:/var/lib/grafana
      - ./config/grafana/provisioning:/etc/grafana/provisioning:ro
    depends_on:
      - prometheus
    networks:
      - monitoring

  node-exporter:
    image: prom/node-exporter:v1.8.1
    container_name: node-exporter
    restart: unless-stopped
    network_mode: host
    pid: host
    volumes:
      - /proc:/host/proc:ro
      - /sys:/host/sys:ro
      - /:/rootfs:ro
    command:
      - '--path.procfs=/host/proc'
      - '--path.sysfs=/host/sys'
      - '--path.rootfs=/rootfs'

volumes:
  prometheus-data:
  grafana-data:

networks:
  monitoring:
```

Notes:
- Node exporter uses `network_mode: host` and `pid: host` for full system metrics access.
- Prometheus and Grafana use named volumes for their time-series data.
- Config files mounted read-only.
- All services on a shared `monitoring` network for inter-service communication.

## The .env file pattern

Place a `.env` file next to `docker-compose.yml`. Compose loads it automatically.

```bash
# /opt/docker/gitea/.env
DB_PASSWORD=supersecretpassword
DOMAIN=git.home.arpa
```

Reference in compose with `${VAR}` syntax. Never commit `.env` files to version control.

## Labels for reverse proxy integration

When using Traefik or similar label-based reverse proxies:

```yaml
services:
  myapp:
    labels:
      - "traefik.enable=true"
      - "traefik.http.routers.myapp.rule=Host(`myapp.home.arpa`)"
      - "traefik.http.routers.myapp.tls.certresolver=letsencrypt"
      - "traefik.http.services.myapp.loadbalancer.server.port=8080"
    networks:
      - proxy-net

networks:
  proxy-net:
    external: true
```

The service must be on the same Docker network as the reverse proxy (`proxy-net`).
