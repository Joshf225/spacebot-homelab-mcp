---
name: docker-deploy
description: Deploy, manage, and troubleshoot Docker containers and Compose stacks across homelab hosts. Covers single-container deployment via MCP tools, multi-container Compose stacks via ssh.exec, networking modes, volume strategies, GPU passthrough, image management, resource limits, health checks, and Docker-in-VM provisioning on Proxmox.
version: 1.0.0
---

# Docker Container Deployment

## Purpose

Use this skill when a Spacebot agent needs to deploy, update, troubleshoot, or manage Docker containers and Compose stacks on one or more Docker hosts. This includes single-container deployments, multi-container orchestration, image management, networking decisions, volume strategies, and provisioning new Docker host VMs.

This playbook separates five commonly conflated areas:

- **Single-container deployment vs Compose stacks** -- different tools, different workflows, different use cases.
- **Bind mounts vs named volumes** -- different access patterns, different backup strategies.
- **Bridge vs host vs macvlan networking** -- different isolation models, different port mapping behavior.
- **Container create vs Compose up** -- `docker.container.create` for simple services; `ssh.exec` with `docker compose` for multi-container stacks.
- **Container-level issues vs host-level issues** -- a container crash is different from a Docker daemon problem or a VM resource constraint.

## When to invoke this skill

- A new service needs to be deployed as a Docker container or Compose stack.
- An existing container needs to be updated, restarted, or reconfigured.
- Container logs need to be inspected for debugging.
- A multi-container stack (e.g., app + database + reverse proxy) needs to be deployed.
- Docker networking needs to be set up or troubleshot (bridge, macvlan, port conflicts).
- Volume mounts or data persistence needs to be planned or debugged.
- Images need to be pulled, updated, or pruned.
- A new Docker host VM needs to be provisioned on Proxmox.
- GPU passthrough is required for transcoding or ML workloads.
- Container resource limits need to be set or adjusted.

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

For provisioning Docker host VMs on Proxmox:

| Tool | Purpose | Confirmation Required |
|------|---------|----------------------|
| `proxmox.vm.create` / `proxmox.vm.clone` | Create Docker host VM | Yes |
| `proxmox.vm.config.update` | Configure VM resources (CPU, RAM, GPU passthrough) | Yes |

## Environment variables

Throughout this skill, the following placeholders are used. The agent must resolve these to actual values from the user's environment before executing operations.

| Placeholder | Meaning | Example |
|---|---|---|
| `<DOCKER_HOST>` | Docker host config name in Spacebot | `docker1` |
| `<SERVICE_NAME>` | Name of the service being deployed | `jellyfin` |
| `<IMAGE>` | Full image reference | `lscr.io/linuxserver/jellyfin:latest` |
| `<HOST_PORT>` | Port on the Docker host | `8096` |
| `<CONTAINER_PORT>` | Port inside the container | `8096` |
| `<HOST_PATH>` | Bind mount path on the host | `/opt/docker/jellyfin/config` |
| `<CONTAINER_PATH>` | Mount path inside the container | `/config` |
| `<NETWORK_NAME>` | Docker network name | `proxy-net` |
| `<COMPOSE_DIR>` | Directory on the host where compose file lives | `/opt/docker/jellyfin` |
| `<PVE_HOST>` | Proxmox host for Docker VM provisioning | `pve1` |

## High-confidence lessons learned

These patterns recur across homelab Docker environments and represent the most common mistakes and best practices.

1. **Compose via ssh.exec is the primary deployment method for multi-container stacks.**
   - There is no native Compose MCP tool. The workflow is: write a `docker-compose.yml`, upload it with `ssh.upload`, then run `docker compose up -d` via `ssh.exec`.
   - This is the preferred method for any stack with more than one container, shared networks, or shared volumes.
   - Single-container services can use `docker.container.create` directly for simplicity.

2. **Use `/opt/docker/<service>/` as the standard bind mount root.**
   - Every service gets its own directory: `/opt/docker/jellyfin/`, `/opt/docker/traefik/`, etc.
   - Within each: `config/`, `data/`, `compose.yml` (or `docker-compose.yml`).
   - This convention makes backups trivial (`tar` or `rsync` the entire `/opt/docker/` tree) and keeps the filesystem organized.
   - Create the directory structure before deploying: `ssh.exec` with `mkdir -p /opt/docker/<SERVICE_NAME>/{config,data}`.

3. **Always set a restart policy.**
   - Use `unless-stopped` for most homelab services. It survives host reboots but respects manual `docker stop`.
   - Use `always` only for critical infrastructure that must restart even after manual stops.
   - Use `on-failure` with a max retry count for services that should not restart indefinitely on crash loops.
   - Never leave the default (`no`) for production services.

4. **Pin image tags for stability; use `latest` only for non-critical services.**
   - `latest` is convenient but can break things on a pull. Use specific version tags for databases, reverse proxies, and anything where unexpected upgrades cause data issues.
   - For LinuxServer.io images and media apps, `latest` is generally safe because they use rolling releases designed for homelabs.

5. **Bridge networking is the default and right choice for most services.**
   - Use bridge with explicit port mappings (`-p HOST:CONTAINER`) for standard web services.
   - Use host networking only when the container needs the full host network stack (e.g., Pi-hole DHCP, Tailscale).
   - Use macvlan when the container needs its own IP on the LAN (e.g., a second DNS server that must appear as a separate device).

6. **The PUID/PGID pattern prevents permission headaches.**
   - Many homelab images (LinuxServer.io, etc.) accept `PUID` and `PGID` environment variables.
   - Set these to match the host user that owns the bind mount directories.
   - Without this, files created inside the container may be owned by root and inaccessible from the host (or vice versa).

7. **Stop the container before deleting it.**
   - Like Proxmox VMs, a running container should be stopped before deletion. Use `docker.container.stop` then `docker.container.delete`.

8. **Check port conflicts before deploying.**
   - Use `ssh.exec` with `ss -tlnp | grep <HOST_PORT>` to verify a port is free before creating a container with that port mapping.
   - Common conflicts: 80/443 (reverse proxies), 53 (DNS), 8080 (various web UIs).

9. **Docker Compose down removes containers but not volumes by default.**
   - `docker compose down` removes containers and networks but preserves named volumes and bind mounts.
   - `docker compose down -v` removes named volumes too -- this is destructive and should be used with care.
   - Bind mounts are never removed by `docker compose down`.

10. **GPU passthrough requires NVIDIA Container Toolkit on the host.**
    - Install `nvidia-container-toolkit` on the Docker host.
    - Use `--runtime=nvidia` or `--gpus all` (in Compose: `runtime: nvidia` or `deploy.resources.reservations.devices`).
    - The GPU must be passed through to the VM first (if Docker runs in a Proxmox VM) and the NVIDIA driver must be installed in the VM.

## Container deployment patterns

### Single container with docker.container.create

For simple, single-container services:

```
docker.container.create(
  host="<DOCKER_HOST>",
  name="<SERVICE_NAME>",
  image="<IMAGE>",
  ports={"<HOST_PORT>": "<CONTAINER_PORT>"},
  env=["PUID=1000", "PGID=1000", "TZ=America/New_York"],
  volumes=["/opt/docker/<SERVICE_NAME>/config:/config", "/opt/docker/<SERVICE_NAME>/data:/data"],
  restart_policy="unless-stopped"
)
```

Before creating:
1. `docker.image.pull` the image first (or let create pull it).
2. Verify port availability: `ssh.exec` with `ss -tlnp | grep <HOST_PORT>`.
3. Create host directories: `ssh.exec` with `mkdir -p /opt/docker/<SERVICE_NAME>/{config,data}`.

### Multi-container stack with Compose (primary method)

This is the **recommended approach** for most deployments:

1. **Prepare the compose file** locally or generate it.
2. **Create the directory on the host:**
   ```
   ssh.exec(host="<DOCKER_HOST>", command="mkdir -p /opt/docker/<SERVICE_NAME>")
   ```
3. **Upload the compose file:**
   ```
   ssh.upload(host="<DOCKER_HOST>", local_path="./docker-compose.yml", remote_path="/opt/docker/<SERVICE_NAME>/docker-compose.yml")
   ```
4. **Deploy the stack:**
   ```
   ssh.exec(host="<DOCKER_HOST>", command="docker compose up -d", cwd="/opt/docker/<SERVICE_NAME>")
   ```
5. **Verify:**
   ```
   ssh.exec(host="<DOCKER_HOST>", command="docker compose ps", cwd="/opt/docker/<SERVICE_NAME>")
   ```

To update a running stack:
```
ssh.exec(host="<DOCKER_HOST>", command="docker compose pull && docker compose up -d", cwd="/opt/docker/<SERVICE_NAME>")
```

To stop and remove:
```
ssh.exec(host="<DOCKER_HOST>", command="docker compose down", cwd="/opt/docker/<SERVICE_NAME>")
```

## Networking modes

### Bridge (default -- use for most services)

- Containers get their own network namespace.
- Ports must be explicitly mapped with `-p HOST:CONTAINER`.
- Containers on the same bridge network can communicate by container name (Docker DNS).
- Create custom bridge networks for groups of related services:
  ```
  ssh.exec(host="<DOCKER_HOST>", command="docker network create <NETWORK_NAME>")
  ```

### Host

- Container shares the host's network namespace. No port mapping needed -- the container binds directly to host ports.
- Use for: Pi-hole (DHCP needs raw network access), Tailscale containers, services needing multicast/broadcast.
- **Warning:** Port conflicts are directly with host services. Two host-network containers cannot bind the same port.

### Macvlan

- Container gets its own MAC address and IP on the physical LAN.
- Appears as a separate device on the network.
- Use for: secondary DNS servers, services that need to be discoverable on the LAN by IP.
- Setup requires creating a macvlan network:
  ```
  ssh.exec(host="<DOCKER_HOST>", command="docker network create -d macvlan --subnet=10.0.1.0/24 --gateway=10.0.1.1 -o parent=eth0 macvlan-net")
  ```
- **Gotcha:** The host cannot communicate with macvlan containers directly without a macvlan-shim interface.

### Ipvlan

- Similar to macvlan but shares the host's MAC address.
- Use when the switch or network enforces MAC limits or port security.
- Less common in homelabs.

## Volume strategies

### Bind mounts (preferred for most homelab services)

Convention: `/opt/docker/<SERVICE_NAME>/`

```
/opt/docker/
├── jellyfin/
│   ├── docker-compose.yml
│   ├── config/          # Application config (bind mount to /config)
│   └── data/            # Application data
├── traefik/
│   ├── docker-compose.yml
│   ├── config/
│   └── certs/
└── postgres/
    ├── docker-compose.yml
    └── data/            # Database files
```

Advantages:
- Easy to inspect, backup, and restore from the host.
- `rsync` or `tar` the entire `/opt/docker/` tree for host-level backups.
- Config files can be edited directly on the host.

### Named volumes (use for databases and opaque data)

- Better for data that should not be directly accessed from the host (database files, internal caches).
- Docker manages the storage location.
- Back up with `docker run --rm -v volume_name:/data -v /backup:/backup busybox tar czf /backup/volume.tar.gz /data`.

### NFS mounts (use for shared media libraries)

- Mount NFS shares either at the host level (preferred) and bind-mount into containers, or use Docker's NFS volume driver.
- Host-level NFS mount is simpler and more reliable:
  ```
  # In /etc/fstab on the Docker host:
  nas:/media /mnt/media nfs defaults,_netdev 0 0
  
  # Then bind-mount into the container:
  volumes:
    - /mnt/media:/media:ro
  ```

### Permission handling (PUID/PGID)

Most LinuxServer.io and homelab-oriented images support:
```yaml
environment:
  - PUID=1000
  - PGID=1000
```

For images that don't support PUID/PGID:
- Set ownership on the bind mount directory: `chown -R 1000:1000 /opt/docker/<SERVICE_NAME>/`
- Or use the `user:` directive in Compose to run as a specific UID.

## Multi-host Docker management

When managing containers across multiple Docker hosts:

1. **List available hosts:** Check Spacebot's Docker host configuration. Each host is a separate SSH target.
2. **Host selection criteria:**
   - **Resource availability:** Check with `ssh.exec` running `free -h` and `nproc`.
   - **GPU availability:** Services needing transcoding go on the GPU host.
   - **Network proximity:** Services that communicate heavily should share a host (or be on the same VLAN).
   - **Failure isolation:** Don't put all critical services on one host.
3. **Cross-host communication:** Containers on different hosts communicate via host IPs or Tailscale, not Docker networks.

## GPU passthrough for Docker

### Requirements

1. **Proxmox VM level:** GPU must be passed through to the VM via PCI passthrough (`proxmox.vm.config.update` with `hostpci` parameter).
2. **VM level:** NVIDIA drivers installed in the VM.
3. **Docker level:** `nvidia-container-toolkit` installed on the Docker host.

### Deploying GPU-enabled containers

Single container:
```
docker.container.create(
  host="<DOCKER_HOST>",
  name="jellyfin",
  image="lscr.io/linuxserver/jellyfin:latest",
  ports={"8096": "8096"},
  env=["PUID=1000", "PGID=1000", "NVIDIA_VISIBLE_DEVICES=all"],
  volumes=["/opt/docker/jellyfin/config:/config", "/mnt/media:/media:ro"],
  restart_policy="unless-stopped"
)
```

Compose:
```yaml
services:
  jellyfin:
    image: lscr.io/linuxserver/jellyfin:latest
    runtime: nvidia
    environment:
      - NVIDIA_VISIBLE_DEVICES=all
    deploy:
      resources:
        reservations:
          devices:
            - capabilities: [gpu]
```

### Verifying GPU access

```
ssh.exec(host="<DOCKER_HOST>", command="docker exec jellyfin nvidia-smi")
```

## Image management

### Pulling images

- Pull before creating: `docker.image.pull(host="<DOCKER_HOST>", image="<IMAGE>")`.
- For Compose stacks: `ssh.exec` with `docker compose pull` in the service directory.

### Updating running services

Single container:
1. `docker.image.pull` the new image.
2. `docker.container.stop` the running container.
3. `docker.container.delete` the old container.
4. `docker.container.create` with the same parameters and the new image.

Compose stack (simpler):
```
ssh.exec(host="<DOCKER_HOST>", command="docker compose pull && docker compose up -d", cwd="/opt/docker/<SERVICE_NAME>")
```
Compose handles the stop/remove/recreate cycle automatically.

### Pruning unused images

```
docker.image.prune(host="<DOCKER_HOST>")
```

This removes dangling (untagged) images. For a more aggressive cleanup:
```
ssh.exec(host="<DOCKER_HOST>", command="docker image prune -a --filter 'until=720h'")
```
This removes all unused images older than 30 days.

## Resource management

### Setting CPU and memory limits

In `docker.container.create`, use resource limit parameters when available. In Compose:

```yaml
services:
  myservice:
    deploy:
      resources:
        limits:
          cpus: '2.0'
          memory: 2G
        reservations:
          memory: 512M
```

### When to set limits

- **Always limit databases** -- prevent a runaway query from consuming all host RAM.
- **Always limit untrusted workloads** -- anything internet-facing.
- **Optional for media servers** -- transcoding is CPU/GPU intensive; limits can cause stuttering.
- **Optional for lightweight services** -- Pi-hole, small web UIs rarely need limits.

### Right-sizing

1. Deploy without limits initially.
2. Monitor with `docker.container.inspect` or `ssh.exec` running `docker stats --no-stream`.
3. Set limits at 1.5-2x observed peak usage.

## Docker-in-VM workflow

When a new Docker host is needed:

### Using a pre-built template (preferred)

1. `proxmox.vm.clone` from the Docker host template (e.g., VMID 9001).
2. `proxmox.vm.config.update` to set CPU, RAM, IP (via cloud-init), and optionally attach GPU.
3. `proxmox.vm.start` -- Docker is pre-installed and ready.
4. Verify: `ssh.exec(host="<new-host>", command="docker version")`.

### From scratch

1. `proxmox.vm.clone` from a base Ubuntu/Debian template.
2. `proxmox.vm.config.update` for resources.
3. `proxmox.vm.start`.
4. Install Docker via SSH:
   ```
   ssh.exec(host="<new-host>", command="curl -fsSL https://get.docker.com | sh")
   ssh.exec(host="<new-host>", command="sudo usermod -aG docker $USER")
   ```
5. For GPU hosts, also install NVIDIA drivers and `nvidia-container-toolkit`.

### Recommended VM sizing for Docker hosts

| Workload | Cores | RAM | Disk |
|----------|-------|-----|------|
| Light (1-5 containers, no media) | 2 | 2 GB | 32 GB |
| Medium (5-15 containers, mixed) | 4 | 8 GB | 64 GB |
| Heavy (15+ containers, databases, media) | 6-8 | 16 GB | 128 GB |
| GPU transcoding host | 4 | 8 GB | 64 GB + media NFS |

## Health checks and restart policies

### Restart policies

| Policy | Behavior | Use case |
|--------|----------|----------|
| `no` | Never restart | One-off tasks, debugging |
| `on-failure[:max]` | Restart on non-zero exit, up to max times | Services with known crash modes |
| `always` | Always restart, even after manual stop | Critical infra (reverse proxy, DNS) |
| `unless-stopped` | Like `always`, but not after manual `docker stop` | **Default for most services** |

### Health checks in Compose

```yaml
services:
  myservice:
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:8080/health"]
      interval: 30s
      timeout: 10s
      retries: 3
      start_period: 40s
```

Use health checks for:
- Services that other containers depend on (`depends_on` with `condition: service_healthy`).
- Services with slow startup (databases, Java applications).
- Monitoring container health via `docker.container.inspect`.

## Decision trees

### Single container vs Compose stack?

**Use `docker.container.create` if:**
- Single container with no dependencies.
- Simple port/volume/env configuration.
- Quick one-off deployment.

**Use Compose (via `ssh.exec`) if:**
- Multiple containers that work together (app + db + cache).
- Shared networks or volumes between containers.
- Complex configuration that benefits from a declarative YAML file.
- You want easy `docker compose pull && docker compose up -d` update workflow.
- **When in doubt, use Compose.** It's more maintainable and self-documenting.

### Which Docker host?

1. **Check resource availability** on each host: `ssh.exec` with `docker stats --no-stream` and `free -h`.
2. **GPU needed?** Must go on a GPU-enabled host.
3. **Heavy I/O?** Put databases on hosts with SSD/NVMe storage, not NFS.
4. **Related services?** Co-locate services that communicate frequently (app + its database).
5. **Failure isolation:** Spread critical services across hosts when possible.

### Bind mount vs named volume?

**Bind mount when:**
- You need to edit config files directly from the host.
- You want straightforward filesystem backups.
- Media libraries or shared data from NFS.
- You want to see and manage the files outside Docker.

**Named volume when:**
- Database data files (Postgres, MariaDB, Redis) -- you shouldn't manually touch these.
- Internal application caches.
- You want Docker to manage the lifecycle.

### Bridge vs host vs macvlan networking?

**Bridge (default):**
- Most web services, APIs, media servers.
- Any service where explicit port mapping is acceptable.
- When you need container-to-container DNS resolution.

**Host:**
- Pi-hole with DHCP (needs raw network access).
- Tailscale subnet routers.
- Services needing multicast, broadcast, or mDNS.
- When port mapping overhead is unacceptable.

**Macvlan:**
- Service needs its own LAN IP (appears as separate device).
- Secondary DNS server.
- IoT hubs that need to be discoverable.

## Symptoms classification

### 1) Container won't start

Typical signs:
- `docker.container.start` returns an error or container immediately exits.
- `docker.container.list` shows the container in "exited" or "created" state.

Most likely causes:
- **Port conflict:** another container or host service already binds the port.
- **Missing bind mount path:** the host directory doesn't exist.
- **Permission error:** container process can't read/write to mounted volumes.
- **Bad image:** corrupt or incompatible image for the platform (arm64 vs amd64).
- **Missing environment variables:** required config not provided.

First actions:
1. `docker.container.logs(host="<DOCKER_HOST>", name="<SERVICE_NAME>", tail=50)` -- check error output.
2. `ssh.exec` with `ss -tlnp | grep <HOST_PORT>` -- check port conflict.
3. `ssh.exec` with `ls -la /opt/docker/<SERVICE_NAME>/` -- check directory exists and permissions.

### 2) Container starts but service is unreachable

Typical signs:
- Container is running (status "Up").
- Cannot connect to `<DOCKER_HOST>:<HOST_PORT>`.

Most likely causes:
- **Wrong port mapping:** host port mapped to wrong container port.
- **Service not listening inside container:** the app inside hasn't started yet or is listening on a different port/interface.
- **Host firewall:** `ufw` or `iptables` blocking the port.
- **Container on wrong network:** if using custom bridge, the container may not be reachable from outside.

First actions:
1. `docker.container.inspect` -- verify port mappings.
2. `docker.container.logs` -- check if the service started successfully inside.
3. `ssh.exec` with `curl -I http://localhost:<HOST_PORT>` from the Docker host itself.
4. `ssh.exec` with `ufw status` or `iptables -L -n` to check firewall.

### 3) Container OOM killed

Typical signs:
- Container repeatedly exits.
- `docker.container.inspect` shows `OOMKilled: true`.
- Logs may show "Killed" or no graceful shutdown message.

Most likely causes:
- **Memory limit too low** for the workload.
- **Memory leak** in the application.

First actions:
1. `docker.container.inspect` -- confirm OOM kill.
2. Increase memory limit in Compose or container create parameters.
3. Monitor with `ssh.exec` running `docker stats <SERVICE_NAME> --no-stream` to see actual usage.

### 4) Volume permission issues

Typical signs:
- Container starts but logs show "Permission denied" errors.
- Application can't write to config or data directories.

Most likely causes:
- **UID/GID mismatch:** container runs as a different user than the bind mount owner.
- **PUID/PGID not set** for images that support it.
- **Root-owned directories** with a non-root container process.

First actions:
1. `docker.container.logs` -- find the specific permission error.
2. `ssh.exec` with `ls -lan /opt/docker/<SERVICE_NAME>/` -- check ownership.
3. `ssh.exec` with `docker exec <SERVICE_NAME> id` -- check container user UID/GID.
4. Set `PUID`/`PGID` or `chown` the directories to match.

### 5) Image pull failures

Typical signs:
- `docker.image.pull` fails or times out.

Most likely causes:
- **Docker Hub rate limit:** unauthenticated pulls are limited to 100/6h per IP.
- **Wrong image name or tag:** typo in image reference.
- **DNS issue on Docker host:** can't resolve registry hostname.
- **Registry authentication required:** private registry needs login.

First actions:
1. `ssh.exec` with `docker pull <IMAGE>` to see the full error message.
2. `ssh.exec` with `nslookup registry-1.docker.io` to test DNS.
3. For rate limits: `ssh.exec` with `docker login` to authenticate (raises limit to 200/6h).

### 6) Compose stack partially up

Typical signs:
- `docker compose ps` shows some containers up, some exited or restarting.

Most likely causes:
- **Dependency not healthy:** a service depends on another that hasn't passed its health check yet.
- **Shared resource conflict:** two services trying to use the same port or volume.
- **Image pull failure:** one image failed to pull while others succeeded.

First actions:
1. `ssh.exec` with `docker compose ps` to see which services failed.
2. `ssh.exec` with `docker compose logs <failed-service>` to see errors.
3. Check `depends_on` and health check configuration.

## Safety rules

1. **Never run `docker compose down -v` without explicit user confirmation.** The `-v` flag deletes named volumes and their data. This is irreversible.

2. **Stop before delete.** Always `docker.container.stop` before `docker.container.delete`. Always verify the container is stopped first.

3. **Back up before updating.** Before updating a stateful service (database, wiki, etc.), back up the data directory: `ssh.exec` with `tar czf /opt/docker/<SERVICE_NAME>-backup-$(date +%Y%m%d).tar.gz /opt/docker/<SERVICE_NAME>/`.

4. **Check port availability before deploying.** Verify the port is free to avoid silent conflicts.

5. **Don't prune images on a host running critical containers.** `docker.image.prune` with the `-a` flag removes all unused images, including base images for stopped containers. Only prune when you're sure no stopped containers need their images.

6. **Confirm destructive tools require two steps.** Tools marked with "Confirmation Required" return a confirmation token on first call. The agent must call `confirm_operation` with both that token and the original tool name to execute.

7. **Never store secrets in compose files that are committed to version control.** Use environment variables, Docker secrets, or `.env` files that are excluded from version control.

8. **Test GPU access before deploying GPU workloads.** Run `nvidia-smi` inside a test container before deploying the actual service.

## Recommended procedural flow for agents

When deploying a Docker service:

1. **Clarify requirements:** What service? Single container or multi-container? GPU needed? What ports, volumes, environment?
2. **Select Docker host:** Check available hosts, resources, GPU availability, network proximity.
3. **Check for conflicts:** `docker.container.list` on the target host. `ssh.exec` with `ss -tlnp` for port conflicts.
4. **Prepare the host:** Create `/opt/docker/<SERVICE_NAME>/` directory structure via `ssh.exec`.
5. **Choose deployment method:** Single container → `docker.container.create`. Multi-container → Compose via `ssh.upload` + `ssh.exec`.
6. **Pull images:** `docker.image.pull` or `docker compose pull` to pre-fetch images.
7. **Deploy:** Create container or `docker compose up -d`.
8. **Verify:** `docker.container.list` to confirm running state. `docker.container.logs` to check for errors. Test service reachability from expected clients.
9. **Document:** Note the host, ports, volumes, and any special configuration for future reference.

When troubleshooting:

1. **Check container state:** `docker.container.list` and `docker.container.inspect`.
2. **Read logs:** `docker.container.logs` with appropriate tail/since.
3. **Check host-level issues:** `ssh.exec` for disk space (`df -h`), memory (`free -h`), port conflicts (`ss -tlnp`).
4. **Test connectivity:** `ssh.exec` with `curl` from the Docker host to the container port.
5. **Check permissions:** compare host directory ownership with container user.
6. **Classify the symptom** using the categories above and follow the recommended first actions.

## Fast diagnosis cheatsheet

| Symptom | Most likely fix |
|---|---|
| Container exits immediately | Check logs: `docker.container.logs`; usually missing env var or port conflict |
| Port already in use | `ssh.exec` with `ss -tlnp \| grep <PORT>` to find the conflict; stop the conflicting service |
| Permission denied on volume | Set PUID/PGID env vars or `chown` the host directory to match container UID |
| Container OOM killed | Increase memory limit in Compose `deploy.resources.limits.memory` |
| Image pull rate limited | `docker login` to authenticate, or wait 6 hours |
| Compose service depends_on not working | Add `condition: service_healthy` and a proper `healthcheck` to the dependency |
| Container can't resolve DNS | Check Docker daemon DNS config: `ssh.exec` with `cat /etc/docker/daemon.json` |
| GPU not available in container | Verify `nvidia-container-toolkit` installed; use `runtime: nvidia` in Compose |
| Service unreachable from outside host | Check port mapping, host firewall (`ufw status`), and that container is on bridge network |
| Compose stack won't start after host reboot | Ensure `restart: unless-stopped` is set; check Docker daemon is enabled: `systemctl status docker` |
| Named volume data missing after recreate | Named volumes persist across `docker compose down` but NOT `docker compose down -v` |
| Container runs but shows old config | Recreate the container: `docker compose up -d --force-recreate` |

## References

See supporting reference docs in `references/`:

- `compose-patterns.md` -- common docker-compose.yml patterns with complete examples for typical homelab services.
- `networking-guide.md` -- Docker networking modes, custom networks, DNS resolution, port mapping strategies.
- `volume-strategies.md` -- bind mount conventions, named volumes, NFS mounts, backup layouts, permission handling.
