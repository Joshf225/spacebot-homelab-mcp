# Arr App Integration Guide

This document covers how to wire Sonarr, Radarr, Prowlarr, Bazarr, and qBittorrent together after the containers are running. All configuration is done through the web UIs of each service.

## Prerequisites

- All containers are running and accessible on their respective ports
- All containers are on the same Docker network (`media-net`)
- Directory structure is created: `/data/torrents/{movies,tv}` and `/data/media/{movies,tv}`
- All containers share the unified `/data` mount (see compose-example.md)

## Integration order

Configure services in this order to minimize back-and-forth:

1. qBittorrent (set download paths and categories)
2. Prowlarr (add indexers, note API key)
3. Sonarr (note API key, set root folder, add download client)
4. Radarr (note API key, set root folder, add download client)
5. Prowlarr again (add Sonarr + Radarr as applications, trigger sync)
6. Bazarr (connect to Sonarr + Radarr)

## Step 1: qBittorrent setup

### Default credentials

On first launch, qBittorrent generates a temporary password. Find it in the logs:

```
docker.container.logs(host="<DOCKER_HOST>", container="qbittorrent", tail=30)
```

Look for a line like: `The WebUI administrator password was not set. A temporary password is provided: <password>`

Login at `http://<DOCKER_HOST>:8080` with username `admin` and the temporary password.

### Change password

Options > Web UI > Authentication > change the password.

### Configure download paths

Options > Downloads:
- Default Save Path: `/data/torrents`
- Keep incomplete torrents in: `/data/torrents/incomplete` (optional, useful for monitoring)

### Configure categories

Right-click in the left sidebar under "Categories" > Add Category:

| Category | Save Path |
|----------|-----------|
| `tv` | `/data/torrents/tv` |
| `movies` | `/data/torrents/movies` |
| `music` | `/data/torrents/music` |

These categories are used by Sonarr/Radarr to tell qBittorrent where to save downloads.

### Recommended settings

- Options > Downloads > uncheck "Create subfolder when content is single file"
- Options > BitTorrent > Seeding Limits: configure based on ratio goals or tracker requirements
- Options > Advanced > Network Interface: bind to VPN interface if using a VPN (e.g., `tun0` or `wg0`)

## Step 2: Prowlarr setup

Access `http://<DOCKER_HOST>:9696`.

### Note the API key

Settings > General > API Key. Copy this -- you will need it when configuring Sonarr/Radarr as apps.

### Add indexers

Indexers > Add Indexer (+ button):

1. Search for your indexer by name
2. Fill in the required fields (URL, API key, credentials -- depends on the indexer)
3. Test the connection
4. Save

Repeat for each indexer. Prowlarr supports hundreds of public and private indexers/trackers.

### Indexer categories

Each indexer in Prowlarr has category mappings. Ensure that:
- Movie categories are mapped (e.g., Movies/HD, Movies/UHD)
- TV categories are mapped (e.g., TV/HD, TV/UHD)

Prowlarr handles this automatically for most indexers but verify if search results seem incomplete.

## Step 3: Sonarr setup

Access `http://<DOCKER_HOST>:8989`.

### Note the API key

Settings > General > API Key. Copy this for Prowlarr and Bazarr.

### Add root folder

Settings > Media Management > Root Folders > Add Root Folder:
- Path: `/data/media/tv`

This is where Sonarr organizes TV show files.

### Add qBittorrent as download client

Settings > Download Clients > Add (+ button) > qBittorrent:

| Field | Value |
|-------|-------|
| Name | qBittorrent |
| Host | `qbittorrent` |
| Port | `8080` |
| Username | `admin` |
| Password | (the password you set) |
| Category | `tv` |

Click "Test" to verify the connection, then "Save".

The category `tv` tells qBittorrent to save Sonarr's downloads to `/data/torrents/tv`.

### Do NOT add indexers

Indexers will be synced automatically from Prowlarr in Step 5.

### Media management settings

Settings > Media Management:

- **Rename Episodes**: Yes
- **Standard Episode Format**: `{Series TitleYear} - S{season:00}E{episode:00} - {Episode CleanTitle} [{Custom Formats }{Quality Full}]{[MediaInfo VideoDynamicRangeType]}{[Mediainfo AudioCodec}{ Mediainfo AudioChannels]}{MediaInfo AudioLanguages}{-Release Group}`
  (Or use the TRaSH Guides recommended naming scheme)
- **Use Hardlinks instead of Copy**: Yes (critical for performance)
- **Import Extra Files**: Yes, with extensions: `srt,ass,ssa` (for subtitle files)

### Quality profiles

Settings > Profiles:

Configure quality profiles based on your preferences. The TRaSH Guides provide optimized profiles:
- https://trash-guides.info/Sonarr/sonarr-setup-quality-profiles/

At minimum, ensure the profile includes your preferred quality tiers (e.g., Bluray-1080p, WEB-1080p) and set an appropriate cutoff.

## Step 4: Radarr setup

Access `http://<DOCKER_HOST>:7878`.

### Note the API key

Settings > General > API Key. Copy this for Prowlarr and Bazarr.

### Add root folder

Settings > Media Management > Root Folders > Add Root Folder:
- Path: `/data/media/movies`

### Add qBittorrent as download client

Settings > Download Clients > Add (+ button) > qBittorrent:

| Field | Value |
|-------|-------|
| Name | qBittorrent |
| Host | `qbittorrent` |
| Port | `8080` |
| Username | `admin` |
| Password | (the password you set) |
| Category | `movies` |

Click "Test" then "Save".

The category `movies` tells qBittorrent to save Radarr's downloads to `/data/torrents/movies`.

### Do NOT add indexers

Indexers will be synced from Prowlarr.

### Media management settings

Settings > Media Management:

- **Rename Movies**: Yes
- **Standard Movie Format**: `{Movie CleanTitle} {(Release Year)} {imdb-{ImdbId}} [{Custom Formats }{Quality Full}]{[MediaInfo VideoDynamicRangeType]}{[Mediainfo AudioCodec}{ Mediainfo AudioChannels]}{[MediaInfo AudioLanguages]}{-Release Group}`
  (Or use TRaSH Guides recommended naming)
- **Use Hardlinks instead of Copy**: Yes
- **Import Extra Files**: Yes, extensions: `srt,ass,ssa`

### Quality profiles

Settings > Profiles:

See TRaSH Guides for optimized profiles:
- https://trash-guides.info/Radarr/radarr-setup-quality-profiles/

## Step 5: Prowlarr -- add Sonarr and Radarr as applications

Back in Prowlarr (`http://<DOCKER_HOST>:9696`):

Settings > Apps > Add Application (+ button):

### Add Sonarr

| Field | Value |
|-------|-------|
| Name | Sonarr |
| Sync Level | Full Sync |
| Prowlarr Server | `http://prowlarr:9696` |
| Sonarr Server | `http://sonarr:8989` |
| API Key | `<SONARR_API_KEY>` |

Click "Test" then "Save".

### Add Radarr

| Field | Value |
|-------|-------|
| Name | Radarr |
| Sync Level | Full Sync |
| Prowlarr Server | `http://prowlarr:9696` |
| Radarr Server | `http://radarr:7878` |
| API Key | `<RADARR_API_KEY>` |

Click "Test" then "Save".

### Trigger sync

After adding both applications, click the "Sync App Indexers" button (circular arrow icon) in Settings > Apps. This pushes all configured indexers to Sonarr and Radarr.

Verify by checking Sonarr > Settings > Indexers and Radarr > Settings > Indexers. You should see all Prowlarr indexers listed there automatically.

### Sync levels explained

| Level | Behavior |
|-------|----------|
| Disabled | No sync -- manual only |
| Add and Remove Only | Prowlarr adds/removes indexers but does not update settings |
| Full Sync | Prowlarr manages all indexer settings. Changes in Sonarr/Radarr are overwritten. **Recommended.** |

Always use Full Sync so Prowlarr is the single source of truth.

## Step 6: Bazarr setup (optional)

Access `http://<DOCKER_HOST>:6767`.

### Connect to Sonarr

Settings > Sonarr:

| Field | Value |
|-------|-------|
| Hostname or IP | `sonarr` |
| Port | `8989` |
| API Key | `<SONARR_API_KEY>` |
| SSL | No |

Click "Test" then "Save".

### Connect to Radarr

Settings > Radarr:

| Field | Value |
|-------|-------|
| Hostname or IP | `radarr` |
| Port | `7878` |
| API Key | `<RADARR_API_KEY>` |
| SSL | No |

Click "Test" then "Save".

### Configure languages

Settings > Languages:

- Add your desired subtitle languages in order of preference
- Enable "Single Language" if you only need one language
- Enable "Forced" if you want forced subtitles (foreign language parts only)

### Configure subtitle providers

Settings > Providers > Add Provider:

Common providers:
- **OpenSubtitles.com** -- largest subtitle database, requires free account
- **Subscene** -- good for non-English subtitles
- **Addic7ed** -- good for English TV subtitles
- **Podnapisi** -- European languages

Add at least 2-3 providers for redundancy. Each provider requires its own account/credentials.

### Anti-captcha (optional)

Some providers require captcha solving. Bazarr supports anti-captcha services. Configure under Settings > Anti-Captcha if needed.

## Path mapping verification

After all services are configured, verify that paths are consistent. Run these checks from the Docker host:

```bash
# Verify all containers see the same /data structure
docker exec sonarr ls /data/torrents/tv
docker exec sonarr ls /data/media/tv
docker exec radarr ls /data/torrents/movies
docker exec radarr ls /data/media/movies
docker exec qbittorrent ls /data/torrents
docker exec jellyfin ls /data/media
docker exec bazarr ls /data/media
```

All commands should succeed and show the expected directories.

### Testing the full pipeline

To verify the entire chain works end-to-end:

1. In Sonarr or Radarr, add a piece of media and trigger a search
2. Verify the search hits indexers (check Activity > History)
3. Verify qBittorrent starts the download (check qBittorrent Web UI)
4. Once downloaded, verify the import completes (Sonarr/Radarr Activity)
5. Verify the file appears in the media directory with a hardlink:
   ```
   ls -li /data/torrents/tv/<downloaded-file>
   ls -li /data/media/tv/<show>/<season>/<imported-file>
   ```
   Same inode number confirms hardlink.
6. Verify Jellyfin picks up the new media (trigger library scan or wait for scheduled scan)
7. If Bazarr is configured, verify subtitles are downloaded automatically

## Troubleshooting integration issues

### Prowlarr cannot connect to Sonarr/Radarr

Symptoms: "Unable to connect" error when adding Sonarr/Radarr as apps in Prowlarr.

Check:
1. Are all containers on `media-net`?
   ```
   docker network inspect media-net
   ```
2. Can Prowlarr resolve the container name?
   ```
   docker exec prowlarr nslookup sonarr
   ```
3. Is the API key correct? Copy it directly from Sonarr/Radarr Settings > General.
4. Is the arr app running and healthy?
   ```
   docker exec prowlarr curl -s http://sonarr:8989/api/v3/system/status?apikey=<KEY>
   ```

### qBittorrent connection refused from Sonarr/Radarr

Symptoms: "Unable to connect to qBittorrent" in Download Clients.

Check:
1. Is the host set to `qbittorrent` (not `localhost`)?
2. Is the port `8080`?
3. Are credentials correct?
4. If using a VPN container (gluetun), the host must be `gluetun` instead of `qbittorrent`.

### Imports fail with "file not found"

Symptoms: Sonarr/Radarr downloads complete but import fails with path errors.

This is almost always a path mapping issue:
1. Check what path qBittorrent reports for the download (qBittorrent Web UI > completed torrent > save path)
2. Check what path Sonarr/Radarr are looking for (Activity > History > import event details)
3. If paths differ, the `/data` mount is not consistent. Fix the volume mounts in docker-compose.yml.
4. If using Remote Path Mappings (Settings > Download Clients > Remote Path Mappings), verify they are correct. Better yet, remove them and use unified `/data` mounts.

### Hardlinks not working (copies instead)

Symptoms: Disk usage doubles after import; `ls -li` shows different inodes.

Check:
1. Are source and destination on the same filesystem?
   ```
   df /data/torrents /data/media
   ```
   Must show the same filesystem/mount.
2. Is "Use Hardlinks instead of Copy" enabled in Media Management settings?
3. Is the container using the same `/data` mount? (Not `/downloads` mapped to one path and `/media` to another.)

### Bazarr not finding episodes/movies

Symptoms: Bazarr shows "No episodes found" or media list is empty.

Check:
1. Are Sonarr/Radarr connections configured and tested in Bazarr settings?
2. Does Bazarr mount the same `/data/media` path as the media actually uses?
3. Has Sonarr/Radarr imported any media yet? Bazarr only sees media that Sonarr/Radarr know about.

## API key reference

Each arr app generates a unique API key on first run. These keys are used for inter-service communication.

| Service | Where to find API key | Used by |
|---------|-----------------------|---------|
| Sonarr | Settings > General > API Key | Prowlarr, Bazarr |
| Radarr | Settings > General > API Key | Prowlarr, Bazarr |
| Prowlarr | Settings > General > API Key | (used if other tools need Prowlarr API access) |

qBittorrent and Jellyfin use username/password authentication, not API keys.

## Default ports quick reference

| Service | Port | Container hostname |
|---------|------|--------------------|
| Jellyfin | 8096 | `jellyfin` |
| Sonarr | 8989 | `sonarr` |
| Radarr | 7878 | `radarr` |
| Prowlarr | 9696 | `prowlarr` |
| qBittorrent | 8080 | `qbittorrent` |
| Bazarr | 6767 | `bazarr` |

When configuring inter-service connections, always use the container hostname (not IP addresses), as container IPs can change on restart.
