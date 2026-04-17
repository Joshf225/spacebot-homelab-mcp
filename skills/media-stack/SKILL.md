---
name: media-stack
description: Deploy and configure a complete media automation stack (Sonarr, Radarr, Prowlarr, Jellyfin, qBittorrent, Bazarr) with correct path mappings, hardlink support, and inter-service wiring. Covers directory structure, Docker Compose deployment, arr-app integration, subtitle automation, hardware transcoding, and common failure diagnosis.
version: 1.0.0
---

# Media Stack Deployment

## Purpose

Use this skill when a Spacebot agent needs to deploy, configure, or troubleshoot a media automation stack consisting of any combination of:

- **Jellyfin** -- media server (streaming, transcoding, library management)
- **Sonarr** -- TV show automation (search, download, organize)
- **Radarr** -- movie automation (search, download, organize)
- **Prowlarr** -- indexer manager (centralized indexer config, syncs to Sonarr/Radarr)
- **qBittorrent** -- download client (torrent downloads)
- **Bazarr** -- subtitle automation (auto-download subtitles for Sonarr/Radarr libraries)

This playbook separates commonly conflated areas:

- **Directory structure vs volume mounts** -- the host filesystem layout is separate from how containers see it, but they must align for hardlinks to work.
- **Core stack vs optional services** -- Jellyfin + Sonarr + Radarr + Prowlarr are the core; qBittorrent and Bazarr are optional extras with their own configuration steps.
- **Container deployment vs service configuration** -- getting containers running is step one; wiring the services together via APIs is step two.
- **Hardlinks vs copies** -- same filesystem = instant hardlinks; different filesystems = slow copy + delete with double disk usage.

## When to invoke this skill

- User wants to deploy a media server (Jellyfin, Plex, Emby) with automation.
- User wants to set up Sonarr, Radarr, or Prowlarr.
- User needs a download client (qBittorrent) integrated with arr apps.
- User wants subtitle automation with Bazarr.
- Existing media stack has path mapping issues, permission errors, or integration failures.
- User wants to add a new arr app to an existing stack.
- Media imports are failing or producing copies instead of hardlinks.
- Jellyfin is not detecting new media or transcoding is failing.

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
| `<DOCKER_HOST>` | Docker host config name in Spacebot | `media1` |
| `<MEDIA_ROOT>` | Root data directory on the host | `/data` |
| `<COMPOSE_DIR>` | Directory where compose file lives | `/opt/docker/media-stack` |
| `<PUID>` | User ID for container processes | `1000` |
| `<PGID>` | Group ID for container processes | `1000` |
| `<TZ>` | Timezone | `America/New_York` |
| `<SONARR_API_KEY>` | Sonarr API key (from Settings > General) | `abcdef1234567890` |
| `<RADARR_API_KEY>` | Radarr API key (from Settings > General) | `abcdef1234567890` |
| `<PROWLARR_API_KEY>` | Prowlarr API key (from Settings > General) | `abcdef1234567890` |

## High-confidence lessons learned

These represent the most common mistakes and best practices for media stack deployments. They are ordered by frequency of occurrence.

### 1. Directory structure is the foundation -- get it right first

The single most common cause of media stack failures is incorrect directory structure. All containers must see the same unified path hierarchy so that hardlinks work and arr apps can find files after download.

Use the TRaSH Guides recommended structure:

```
/data/
├── torrents/
│   ├── movies/
│   ├── tv/
│   └── music/
└── media/
    ├── movies/
    ├── tv/
    └── music/
```

The `/data` directory is the shared root. Every container mounts `/data:/data`. This means:
- qBittorrent downloads to `/data/torrents/movies/` (or `/data/torrents/tv/`)
- Sonarr/Radarr see the completed download at `/data/torrents/tv/Show.Name.S01E01/`
- Sonarr/Radarr hardlink (or move) to `/data/media/tv/Show Name/Season 01/`
- Jellyfin reads from `/data/media/`

Because all containers share the same `/data` mount, the paths are identical inside every container. No path mapping is needed. No remote path mappings. No translation.

Create this structure on the host before deploying:

```
ssh.exec(host="<DOCKER_HOST>", command="mkdir -p /data/{torrents/{movies,tv,music},media/{movies,tv,music}}")
ssh.exec(host="<DOCKER_HOST>", command="chown -R <PUID>:<PGID> /data")
```

### 2. Hardlinks vs copies -- same filesystem is mandatory

When the download directory and media library are on the **same filesystem**, Sonarr/Radarr create hardlinks. A hardlink is an instant operation that uses zero additional disk space -- the file exists at two paths but occupies storage only once.

When they are on **different filesystems** (e.g., download on SSD, media on NAS), hardlinks are impossible. Sonarr/Radarr fall back to copy + delete, which:
- Uses double the disk space during the copy
- Takes time proportional to file size
- Creates I/O load

**Plan volume mounts so `/data/torrents` and `/data/media` are on the same filesystem.** If media lives on a NAS, download to the NAS too.

To verify hardlinks are working, check that imported files share an inode:
```
ssh.exec(host="<DOCKER_HOST>", command="ls -li /data/torrents/movies/Movie.Name/ /data/media/movies/Movie\ Name\ \(2024\)/")
```
Same inode number = hardlink. Different inode = copy.

### 3. User/group consistency across all containers

All containers in the media stack must run as the same PUID/PGID. If Sonarr runs as 1000:1000 but qBittorrent runs as 0:0 (root), files downloaded by qBittorrent will be owned by root, and Sonarr cannot move/hardlink them.

Every service in the compose file must have:
```yaml
environment:
  - PUID=1000
  - PGID=1000
```

Verify ownership after deployment:
```
ssh.exec(host="<DOCKER_HOST>", command="ls -la /data/torrents/")
ssh.exec(host="<DOCKER_HOST>", command="docker exec sonarr id")
```

### 4. Networking -- container names as hostnames

All arr apps and the download client must be on the same Docker network. This allows them to communicate by container name (Docker's internal DNS).

When configuring qBittorrent as a download client in Sonarr, the host is `qbittorrent` (the container name), not `localhost` or `127.0.0.1`.

When configuring Sonarr/Radarr as applications in Prowlarr:
- Prowlarr URL: `http://prowlarr:9696`
- Sonarr URL: `http://sonarr:8989`
- Radarr URL: `http://radarr:7878`

Jellyfin is a special case. For DLNA discovery on the local network, Jellyfin may need:
- `network_mode: host` -- simplest, but loses container isolation and port control
- A macvlan network -- Jellyfin gets its own LAN IP
- Standard bridge -- works fine if you only access via web browser / apps (no DLNA)

For most users, standard bridge networking with port mapping is sufficient. Only use host networking if DLNA is required.

### 5. Prowlarr is the single source of truth for indexers

Do not add indexers directly in Sonarr or Radarr. Configure all indexers in Prowlarr, then add Sonarr and Radarr as "Applications" in Prowlarr. Prowlarr syncs indexer configurations to both automatically.

Benefits:
- Add an indexer once, it appears in all arr apps
- Indexer stats and health monitoring in one place
- Update credentials in one place

### 6. Download client path mapping -- the #1 silent failure

If the download client (qBittorrent) sees files at a different path than Sonarr/Radarr, imports fail silently or with cryptic errors. The solution from lesson #1 applies: mount the same `/data` root in all containers.

If for some reason you cannot use unified paths, you must configure "Remote Path Mappings" in Sonarr/Radarr under Settings > Download Clients. But this is fragile and error-prone. The unified `/data` mount approach eliminates this entirely.

### 7. Jellyfin hardware transcoding

Jellyfin supports hardware-accelerated transcoding via:
- **Intel Quick Sync / VAAPI**: Pass through `/dev/dri` to the container
- **NVIDIA GPU**: Use `nvidia-container-toolkit` and `runtime: nvidia` (see docker-deploy skill for GPU passthrough details)

For Intel (most common in homelabs):
```yaml
devices:
  - /dev/dri:/dev/dri
```

Verify hardware transcoding is available after deployment:
```
ssh.exec(host="<DOCKER_HOST>", command="docker exec jellyfin ls -la /dev/dri")
```

Then enable hardware transcoding in Jellyfin Dashboard > Playback > Transcoding.

### 8. Bazarr for automatic subtitles (optional)

Bazarr connects to Sonarr and Radarr via their APIs and automatically downloads subtitles for all media. It is optional but highly recommended for non-English content or accessibility.

Bazarr needs:
- Sonarr/Radarr API keys and URLs
- Access to the same `/data` mount (to write subtitle files next to media)
- Configured subtitle providers (OpenSubtitles, Subscene, etc.)

### 9. Reverse proxy base URLs

All services support configurable base URLs for reverse proxy setups:
- Sonarr: Settings > General > URL Base (e.g., `/sonarr`)
- Radarr: Settings > General > URL Base (e.g., `/radarr`)
- Prowlarr: Settings > General > URL Base (e.g., `/prowlarr`)
- Bazarr: Settings > General > URL Base (e.g., `/bazarr`)
- Jellyfin: Dashboard > Networking > Base URL (e.g., `/jellyfin`)
- qBittorrent: Options > Web UI > Alternative Web UI > URL

This skill does not deploy a reverse proxy. If one is needed, use the docker-deploy skill to set up Traefik, Caddy, or nginx-proxy-manager. Just note the base URL settings here for when the user adds a reverse proxy later.

### 10. Image versioning strategy

For homelab media stacks, using `:latest` is generally acceptable because:
- LinuxServer.io images use rolling releases designed for homelabs
- Arr apps have built-in database migration on startup
- Jellyfin handles upgrades gracefully

However, note the tradeoff: a `docker compose pull && docker compose up -d` could introduce breaking changes. For more stability, pin to specific version tags (e.g., `lscr.io/linuxserver/sonarr:4.0.9`).

Always back up before pulling new images (see Safety Rules).

## Service configuration details

### Jellyfin

| Setting | Value |
|---------|-------|
| Image | `lscr.io/linuxserver/jellyfin:latest` |
| Default port | 8096 (HTTP), 8920 (HTTPS) |
| Config path | `/opt/docker/media-stack/jellyfin/config` mapped to `/config` |
| Media path | `/data/media` mapped to `/data/media` (read-only is fine) |
| DLNA ports | 1900/udp, 7359/udp (only if using DLNA) |

After deployment:
1. Access web UI at `http://<DOCKER_HOST>:8096`
2. Complete initial setup wizard
3. Add media libraries: Movies -> `/data/media/movies`, TV Shows -> `/data/media/tv`
4. Enable hardware transcoding if GPU/iGPU is available

### Sonarr

| Setting | Value |
|---------|-------|
| Image | `lscr.io/linuxserver/sonarr:latest` |
| Default port | 8989 |
| Config path | `/opt/docker/media-stack/sonarr/config` mapped to `/config` |
| Data path | `/data` mapped to `/data` |

After deployment:
1. Access web UI at `http://<DOCKER_HOST>:8989`
2. Go to Settings > General, note the API key
3. Set root folder to `/data/media/tv`
4. Configure quality profiles (see TRaSH Guides)
5. Do NOT add indexers manually -- Prowlarr handles this

### Radarr

| Setting | Value |
|---------|-------|
| Image | `lscr.io/linuxserver/radarr:latest` |
| Default port | 7878 |
| Config path | `/opt/docker/media-stack/radarr/config` mapped to `/config` |
| Data path | `/data` mapped to `/data` |

After deployment:
1. Access web UI at `http://<DOCKER_HOST>:7878`
2. Go to Settings > General, note the API key
3. Set root folder to `/data/media/movies`
4. Configure quality profiles (see TRaSH Guides)
5. Do NOT add indexers manually -- Prowlarr handles this

### Prowlarr

| Setting | Value |
|---------|-------|
| Image | `lscr.io/linuxserver/prowlarr:latest` |
| Default port | 9696 |
| Config path | `/opt/docker/media-stack/prowlarr/config` mapped to `/config` |

After deployment:
1. Access web UI at `http://<DOCKER_HOST>:9696`
2. Go to Settings > General, note the API key
3. Add indexers (Indexers > Add Indexer)
4. Add Sonarr and Radarr as Applications (Settings > Apps)
5. Prowlarr syncs indexers to both automatically

### qBittorrent

| Setting | Value |
|---------|-------|
| Image | `lscr.io/linuxserver/qbittorrent:latest` |
| Default port | 8080 (Web UI), 6881 (torrents) |
| Config path | `/opt/docker/media-stack/qbittorrent/config` mapped to `/config` |
| Download path | `/data/torrents` mapped to `/data/torrents` |

After deployment:
1. Access web UI at `http://<DOCKER_HOST>:8080`
2. Default credentials: admin / check container logs for temporary password
3. Change the default password immediately
4. Set default save path to `/data/torrents`
5. Create categories: `movies` (save path: `/data/torrents/movies`), `tv` (save path: `/data/torrents/tv`)
6. Disable "Create subfolder" if you want cleaner paths

### Bazarr

| Setting | Value |
|---------|-------|
| Image | `lscr.io/linuxserver/bazarr:latest` |
| Default port | 6767 |
| Config path | `/opt/docker/media-stack/bazarr/config` mapped to `/config` |
| Data path | `/data/media` mapped to `/data/media` |

After deployment:
1. Access web UI at `http://<DOCKER_HOST>:6767`
2. Configure Sonarr connection: host `sonarr`, port `8989`, API key
3. Configure Radarr connection: host `radarr`, port `7878`, API key
4. Add subtitle providers (Settings > Providers)
5. Configure languages (Settings > Languages)

## Safety rules

1. **Always back up arr app databases before upgrades.** Each arr app stores its database in `/config/*.db`. Back up before pulling new images:
   ```
   ssh.exec(host="<DOCKER_HOST>", command="for svc in sonarr radarr prowlarr bazarr; do cp /opt/docker/media-stack/$svc/config/*.db /opt/docker/media-stack/$svc/config/backup/; done")
   ```

2. **Never delete media directories without explicit user confirmation.** Use `confirm_operation` before any `rm -rf` on paths under `/data/media` or `/data/torrents`.

3. **Use `confirm_operation` before stopping the entire stack.** Stopping the stack interrupts active downloads and may leave incomplete files.

4. **Do not expose qBittorrent WebUI to the internet without authentication.** The default temporary password is logged to stdout on first run. Change it immediately. If behind a reverse proxy, ensure auth is enforced.

5. **Pin image versions for stability or accept the tradeoff with `:latest`.** Document which approach is used so future updates are predictable.

6. **Never run `docker compose down -v` on the media stack.** This deletes named volumes. While this stack primarily uses bind mounts, some users add named volumes. Always use `docker compose down` (without `-v`) unless explicitly confirmed.

7. **Test path mappings before adding media.** After deployment, create a test file to verify the path is visible across containers:
   ```
   ssh.exec(host="<DOCKER_HOST>", command="touch /data/torrents/test-file && docker exec sonarr ls /data/torrents/test-file && docker exec radarr ls /data/torrents/test-file && rm /data/torrents/test-file")
   ```

8. **Do not configure indexers directly in Sonarr/Radarr.** This creates maintenance burden and inconsistency. Always use Prowlarr as the central indexer manager.

## Recommended procedural flow for agents

### Full stack deployment

1. **Gather requirements:**
   - Which services? (core: Jellyfin + Sonarr + Radarr + Prowlarr; optional: qBittorrent, Bazarr)
   - Target Docker host
   - PUID/PGID (check with `ssh.exec`: `id <username>`)
   - Timezone
   - Hardware transcoding needed? (Intel iGPU or NVIDIA?)
   - Existing data directory or fresh install?

2. **Create directory structure on host:**
   ```
   ssh.exec(host="<DOCKER_HOST>", command="mkdir -p /data/{torrents/{movies,tv,music},media/{movies,tv,music}}")
   ssh.exec(host="<DOCKER_HOST>", command="mkdir -p /opt/docker/media-stack/{jellyfin,sonarr,radarr,prowlarr,qbittorrent,bazarr}/config")
   ssh.exec(host="<DOCKER_HOST>", command="chown -R <PUID>:<PGID> /data /opt/docker/media-stack")
   ```

3. **Check for port conflicts:**
   ```
   ssh.exec(host="<DOCKER_HOST>", command="ss -tlnp | grep -E ':(8096|8989|7878|9696|8080|6767) '")
   ```

4. **Upload compose file:**
   Generate or use the reference compose file (see `references/compose-example.md`). Upload:
   ```
   ssh.upload(host="<DOCKER_HOST>", local_path="./docker-compose.yml", remote_path="/opt/docker/media-stack/docker-compose.yml")
   ```

5. **Pull all images:**
   ```
   ssh.exec(host="<DOCKER_HOST>", command="docker compose pull", cwd="/opt/docker/media-stack")
   ```

6. **Deploy the stack:**
   ```
   ssh.exec(host="<DOCKER_HOST>", command="docker compose up -d", cwd="/opt/docker/media-stack")
   ```

7. **Wait for containers to be healthy:**
   ```
   ssh.exec(host="<DOCKER_HOST>", command="docker compose ps", cwd="/opt/docker/media-stack")
   ```
   If any container is not "Up", check logs:
   ```
   docker.container.logs(host="<DOCKER_HOST>", name="<SERVICE_NAME>", tail=50)
   ```

8. **Configure Prowlarr first:**
   - Access `http://<DOCKER_HOST>:9696`
   - Add indexers (user must provide indexer credentials)
   - Note the Prowlarr API key from Settings > General

9. **Configure qBittorrent:**
   - Access `http://<DOCKER_HOST>:8080`
   - Check logs for temporary password: `docker.container.logs(host="<DOCKER_HOST>", name="qbittorrent", tail=20)`
   - Change password
   - Set download paths and categories (movies, tv)

10. **Add Sonarr + Radarr as applications in Prowlarr:**
    - Prowlarr > Settings > Apps > Add Application
    - For Sonarr: Prowlarr Server = `http://prowlarr:9696`, Sonarr Server = `http://sonarr:8989`, API Key = `<SONARR_API_KEY>`
    - For Radarr: Prowlarr Server = `http://prowlarr:9696`, Radarr Server = `http://radarr:7878`, API Key = `<RADARR_API_KEY>`
    - Test and Save. Indexers sync automatically.

11. **Configure Sonarr/Radarr:**
    - Add root folders: `/data/media/tv` (Sonarr), `/data/media/movies` (Radarr)
    - Add qBittorrent as download client: host = `qbittorrent`, port = `8080`, category = `tv` (Sonarr) or `movies` (Radarr)
    - Configure quality profiles per TRaSH Guides
    - Enable hardlinks: Settings > Media Management > Use Hardlinks instead of Copy = Yes

12. **Configure Bazarr (optional):**
    - Access `http://<DOCKER_HOST>:6767`
    - Add Sonarr connection: host = `sonarr`, port = `8989`, API key
    - Add Radarr connection: host = `radarr`, port = `7878`, API key
    - Configure subtitle providers and languages

13. **Verify Jellyfin media libraries:**
    - Access `http://<DOCKER_HOST>:8096`
    - Add libraries if not already configured: Movies -> `/data/media/movies`, TV -> `/data/media/tv`
    - Trigger a library scan
    - Enable hardware transcoding if applicable

14. **Verify path integrity across the stack:**
    ```
    ssh.exec(host="<DOCKER_HOST>", command="docker exec sonarr ls /data/torrents && docker exec sonarr ls /data/media/tv")
    ssh.exec(host="<DOCKER_HOST>", command="docker exec radarr ls /data/torrents && docker exec radarr ls /data/media/movies")
    ssh.exec(host="<DOCKER_HOST>", command="docker exec qbittorrent ls /data/torrents")
    ssh.exec(host="<DOCKER_HOST>", command="docker exec jellyfin ls /data/media")
    ```

### Updating the stack

1. Back up databases:
   ```
   ssh.exec(host="<DOCKER_HOST>", command="tar czf /opt/docker/media-stack-backup-$(date +%Y%m%d).tar.gz /opt/docker/media-stack/*/config/*.db")
   ```

2. Pull and recreate:
   ```
   ssh.exec(host="<DOCKER_HOST>", command="docker compose pull && docker compose up -d", cwd="/opt/docker/media-stack")
   ```

3. Verify all containers are up:
   ```
   ssh.exec(host="<DOCKER_HOST>", command="docker compose ps", cwd="/opt/docker/media-stack")
   ```

4. Check logs for migration errors:
   ```
   ssh.exec(host="<DOCKER_HOST>", command="docker compose logs --tail=20", cwd="/opt/docker/media-stack")
   ```

### Adding a service to an existing stack

1. Add the new service definition to the compose file.
2. Upload the updated compose file.
3. Run `docker compose up -d` -- Compose only recreates changed services.
4. Configure the new service and wire it to existing services.

## Fast diagnosis cheatsheet

| Symptom | Most likely cause | Fix |
|---|---|---|
| Import failed / file not found | Path mapping mismatch between download client and arr apps | Ensure all containers mount `/data:/data`; remove any Remote Path Mappings in Sonarr/Radarr if using unified paths |
| Permission denied on media files | PUID/PGID mismatch between containers | Set identical PUID/PGID in all services; `chown -R <PUID>:<PGID> /data` |
| Jellyfin not finding new media | Library scan not triggered, or wrong root folder | Check Jellyfin library paths match `/data/media/*`; trigger manual scan; enable real-time monitoring in Jellyfin |
| Prowlarr sync failed to Sonarr/Radarr | API key mismatch or network connectivity | Verify API keys; test connectivity: `docker exec prowlarr curl -s http://sonarr:8989/api/v3/system/status?apikey=<KEY>` |
| Slow imports / double disk usage | Download and media dirs on different filesystems | Move both under same mount; verify with `df /data/torrents /data/media` (should be same filesystem) |
| qBittorrent downloads stuck | VPN issues, port not forwarded, or tracker down | Check qBittorrent logs; verify port 6881 is reachable; check if VPN container (if used) is healthy |
| Transcoding failing in Jellyfin | GPU device not passed through or driver mismatch | Verify `/dev/dri` exists in container: `docker exec jellyfin ls /dev/dri`; check Jellyfin transcoding logs |
| Arr app shows "Connection refused" for download client | Wrong hostname (using localhost instead of container name) | Use container name `qbittorrent` as host, not `localhost` or `127.0.0.1` |
| Indexer search returns no results | Prowlarr indexers not synced to arr apps | Check Prowlarr > Settings > Apps; verify sync status; test indexer directly in Prowlarr |
| Bazarr not finding media | Bazarr not connected to Sonarr/Radarr or wrong paths | Verify API connections in Bazarr settings; ensure Bazarr mounts `/data/media` |
| Sonarr/Radarr quality profile not grabbing | Quality profile too restrictive or no indexer results | Check wanted/cutoff settings; verify indexers return results in Prowlarr |
| Container restarts in a loop | Configuration error or corrupt database | Check logs: `docker.container.logs`; restore database from backup if corrupt |
| "Disk space" warning in arr apps | Root folder on small partition or wrong path | Verify root folder path exists and is on the correct filesystem with adequate space |

## Decision trees

### Which services to deploy?

**Minimum viable stack:**
- Jellyfin + Sonarr + Radarr + Prowlarr + a download client (qBittorrent)
- This covers: media serving, TV automation, movie automation, indexer management, downloading

**Add Bazarr if:**
- User watches non-English content
- User wants forced subtitles for foreign language segments
- User has hearing accessibility requirements

**Use existing download client if:**
- User already has qBittorrent/Deluge/Transmission running
- Just ensure the existing client is on the same Docker network and mounts `/data`

### Jellyfin networking mode?

**Bridge (default, recommended):**
- Access via web browser, mobile apps, smart TV apps
- Port mapping: 8096:8096
- Works for 95% of users

**Host networking:**
- Required for DLNA device discovery on LAN
- Required for HDHomeRun live TV integration
- Loses port isolation

### Fresh install vs migration?

**Fresh install:**
- Follow the procedural flow above
- All paths and permissions are set correctly from the start

**Migration from existing setup:**
- Map old paths to new unified `/data` structure
- Update root folders in Sonarr/Radarr
- Re-scan libraries in Jellyfin
- May need to update Remote Path Mappings temporarily during transition

## References

See supporting reference docs in `references/`:

- `compose-example.md` -- Complete production docker-compose.yml with per-service explanations and optional GPU passthrough.
- `arr-integration.md` -- Step-by-step guide for wiring Sonarr, Radarr, Prowlarr, Bazarr, and qBittorrent together via APIs and path configuration.
