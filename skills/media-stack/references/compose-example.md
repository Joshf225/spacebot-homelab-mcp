# Media Stack -- Docker Compose Reference

This is a complete, production-quality `docker-compose.yml` for deploying the full media stack. Copy and adapt as needed.

## Complete docker-compose.yml

```yaml
# Media Stack - Jellyfin + Sonarr + Radarr + Prowlarr + qBittorrent + Bazarr
# 
# Prerequisites:
#   1. Create directories:
#      mkdir -p /data/{torrents/{movies,tv,music},media/{movies,tv,music}}
#      mkdir -p /opt/docker/media-stack/{jellyfin,sonarr,radarr,prowlarr,qbittorrent,bazarr}/config
#   2. Set ownership:
#      chown -R 1000:1000 /data /opt/docker/media-stack
#   3. Update PUID, PGID, and TZ below to match your environment
#
# Usage:
#   docker compose up -d          # Start all services
#   docker compose pull            # Pull latest images
#   docker compose up -d           # Recreate updated services
#   docker compose down            # Stop and remove containers (data preserved)
#   docker compose logs -f sonarr  # Follow logs for a specific service

networks:
  media-net:
    name: media-net
    driver: bridge

services:

  # ---------------------------------------------------------------------------
  # Jellyfin -- Media Server
  # ---------------------------------------------------------------------------
  # Serves media to clients (web, mobile, smart TV). Handles transcoding.
  # Access: http://<host>:8096
  jellyfin:
    image: lscr.io/linuxserver/jellyfin:latest
    container_name: jellyfin
    networks:
      - media-net
    environment:
      - PUID=1000
      - PGID=1000
      - TZ=America/New_York
    volumes:
      # Application config (database, metadata cache, plugins)
      - /opt/docker/media-stack/jellyfin/config:/config
      # Media library -- read-only is sufficient for Jellyfin
      - /data/media:/data/media:ro
    ports:
      - "8096:8096"       # HTTP web UI
      # - "8920:8920"     # HTTPS (uncomment if using SSL directly)
      # - "1900:1900/udp" # DLNA discovery (uncomment if needed)
      # - "7359:7359/udp" # Jellyfin client discovery (uncomment if needed)
    # ---------------------------------------------------------------------------
    # GPU Passthrough for Hardware Transcoding
    # ---------------------------------------------------------------------------
    # Option 1: Intel Quick Sync / VAAPI (most common for homelabs)
    # Uncomment the devices section below:
    #
    # devices:
    #   - /dev/dri:/dev/dri
    #
    # Then enable VAAPI or QSV in Jellyfin Dashboard > Playback > Transcoding.
    #
    # Option 2: NVIDIA GPU
    # Uncomment the runtime and environment lines below:
    #
    # runtime: nvidia
    # environment:
    #   - NVIDIA_VISIBLE_DEVICES=all
    #   - NVIDIA_DRIVER_CAPABILITIES=all
    #
    # Requires nvidia-container-toolkit installed on the Docker host.
    # See docker-deploy skill for full GPU passthrough setup.
    # ---------------------------------------------------------------------------
    restart: unless-stopped

  # ---------------------------------------------------------------------------
  # Sonarr -- TV Show Automation
  # ---------------------------------------------------------------------------
  # Monitors TV show releases, sends downloads to qBittorrent, organizes files.
  # Access: http://<host>:8989
  sonarr:
    image: lscr.io/linuxserver/sonarr:latest
    container_name: sonarr
    networks:
      - media-net
    environment:
      - PUID=1000
      - PGID=1000
      - TZ=America/New_York
    volumes:
      - /opt/docker/media-stack/sonarr/config:/config
      # Mount the entire /data tree so Sonarr can see both torrents and media.
      # This is critical for hardlinks to work.
      - /data:/data
    ports:
      - "8989:8989"
    restart: unless-stopped

  # ---------------------------------------------------------------------------
  # Radarr -- Movie Automation
  # ---------------------------------------------------------------------------
  # Monitors movie releases, sends downloads to qBittorrent, organizes files.
  # Access: http://<host>:7878
  radarr:
    image: lscr.io/linuxserver/radarr:latest
    container_name: radarr
    networks:
      - media-net
    environment:
      - PUID=1000
      - PGID=1000
      - TZ=America/New_York
    volumes:
      - /opt/docker/media-stack/radarr/config:/config
      # Same unified /data mount for hardlink support
      - /data:/data
    ports:
      - "7878:7878"
    restart: unless-stopped

  # ---------------------------------------------------------------------------
  # Prowlarr -- Indexer Manager
  # ---------------------------------------------------------------------------
  # Centralized indexer management. Syncs indexers to Sonarr and Radarr.
  # Access: http://<host>:9696
  prowlarr:
    image: lscr.io/linuxserver/prowlarr:latest
    container_name: prowlarr
    networks:
      - media-net
    environment:
      - PUID=1000
      - PGID=1000
      - TZ=America/New_York
    volumes:
      - /opt/docker/media-stack/prowlarr/config:/config
    ports:
      - "9696:9696"
    restart: unless-stopped

  # ---------------------------------------------------------------------------
  # qBittorrent -- Download Client
  # ---------------------------------------------------------------------------
  # Torrent client. Sonarr/Radarr send downloads here via API.
  # Access: http://<host>:8080
  # Default login: admin / (check container logs for temporary password)
  qbittorrent:
    image: lscr.io/linuxserver/qbittorrent:latest
    container_name: qbittorrent
    networks:
      - media-net
    environment:
      - PUID=1000
      - PGID=1000
      - TZ=America/New_York
      - WEBUI_PORT=8080
    volumes:
      - /opt/docker/media-stack/qbittorrent/config:/config
      # Only needs access to the torrents directory for downloads.
      # Mounted under /data so paths align with Sonarr/Radarr.
      - /data/torrents:/data/torrents
    ports:
      - "8080:8080"   # Web UI
      - "6881:6881"   # Torrent traffic (TCP)
      - "6881:6881/udp" # Torrent traffic (UDP)
    restart: unless-stopped

  # ---------------------------------------------------------------------------
  # Bazarr -- Subtitle Automation (Optional)
  # ---------------------------------------------------------------------------
  # Auto-downloads subtitles for media managed by Sonarr and Radarr.
  # Access: http://<host>:6767
  bazarr:
    image: lscr.io/linuxserver/bazarr:latest
    container_name: bazarr
    networks:
      - media-net
    environment:
      - PUID=1000
      - PGID=1000
      - TZ=America/New_York
    volumes:
      - /opt/docker/media-stack/bazarr/config:/config
      # Needs write access to media directories to save subtitle files
      - /data/media:/data/media
    ports:
      - "6767:6767"
    restart: unless-stopped
```

## Variations

### Minimal stack (no Bazarr, no qBittorrent)

Remove the `qbittorrent` and `bazarr` service blocks. The user must configure an external download client in Sonarr/Radarr instead.

### With VPN for qBittorrent

Replace the `qbittorrent` service with a VPN container (e.g., `gluetun`) and route qBittorrent traffic through it:

```yaml
  gluetun:
    image: qmcgaw/gluetun:latest
    container_name: gluetun
    cap_add:
      - NET_ADMIN
    networks:
      - media-net
    environment:
      - VPN_SERVICE_PROVIDER=<provider>  # e.g., mullvad, nordvpn, etc.
      - VPN_TYPE=wireguard
      # Add provider-specific env vars here
    ports:
      # qBittorrent ports are exposed through gluetun
      - "8080:8080"
      - "6881:6881"
      - "6881:6881/udp"
    restart: unless-stopped

  qbittorrent:
    image: lscr.io/linuxserver/qbittorrent:latest
    container_name: qbittorrent
    network_mode: "service:gluetun"  # Route all traffic through VPN
    environment:
      - PUID=1000
      - PGID=1000
      - TZ=America/New_York
      - WEBUI_PORT=8080
    volumes:
      - /opt/docker/media-stack/qbittorrent/config:/config
      - /data/torrents:/data/torrents
    depends_on:
      - gluetun
    restart: unless-stopped
```

Note: when using `network_mode: "service:gluetun"`, qBittorrent's ports are defined on the `gluetun` container, not on `qbittorrent` itself. Sonarr/Radarr must use `gluetun` as the download client host (since qBittorrent shares gluetun's network namespace).

### With Jellyfin host networking (for DLNA)

Replace the Jellyfin `networks` and `ports` sections:

```yaml
  jellyfin:
    image: lscr.io/linuxserver/jellyfin:latest
    container_name: jellyfin
    network_mode: host
    # No ports section needed -- Jellyfin binds directly to host ports
    environment:
      - PUID=1000
      - PGID=1000
      - TZ=America/New_York
    volumes:
      - /opt/docker/media-stack/jellyfin/config:/config
      - /data/media:/data/media:ro
    restart: unless-stopped
```

When using host networking, Jellyfin is no longer on `media-net`. Other containers cannot reach it by container name. This is fine because Jellyfin does not need to communicate with arr apps -- only clients connect to Jellyfin.

## Volume mount summary

This table clarifies why each service mounts what it mounts:

| Service | Mount | Container Path | Why |
|---------|-------|----------------|-----|
| jellyfin | `/data/media` | `/data/media:ro` | Read media files for streaming. Read-only is sufficient. |
| jellyfin | config dir | `/config` | Persist database, metadata cache, plugins |
| sonarr | `/data` | `/data` | Needs access to both `/data/torrents` (to see downloads) and `/data/media/tv` (to hardlink/move). Full `/data` mount enables hardlinks. |
| radarr | `/data` | `/data` | Same as Sonarr but for movies. Full `/data` mount enables hardlinks. |
| prowlarr | config dir | `/config` | Only needs its own config. No media access required. |
| qbittorrent | `/data/torrents` | `/data/torrents` | Downloads go here. Mounted under `/data` so the path matches what Sonarr/Radarr see. |
| bazarr | `/data/media` | `/data/media` | Needs write access to save `.srt`/`.ass` files next to media files. |

## Port summary

| Service | Port | Protocol | Purpose |
|---------|------|----------|---------|
| Jellyfin | 8096 | TCP | Web UI / API |
| Jellyfin | 8920 | TCP | HTTPS (optional) |
| Jellyfin | 1900 | UDP | DLNA discovery (optional) |
| Jellyfin | 7359 | UDP | Client discovery (optional) |
| Sonarr | 8989 | TCP | Web UI / API |
| Radarr | 7878 | TCP | Web UI / API |
| Prowlarr | 9696 | TCP | Web UI / API |
| qBittorrent | 8080 | TCP | Web UI / API |
| qBittorrent | 6881 | TCP+UDP | Torrent traffic |
| Bazarr | 6767 | TCP | Web UI / API |
