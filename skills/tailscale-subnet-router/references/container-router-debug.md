# Container Router Debug

Diagnostic and configuration reference for Tailscale subnet routers running in Docker, Podman, or any OCI-compatible container runtime.

## Known-good configuration

| Setting | Value |
|---|---|
| Network mode | `host` (`--network=host` or `network_mode: host`) |
| Subnet advertisement | `TS_ROUTES=<SUBNET_CIDR>` |
| SNAT | Enabled (`TS_SNAT_SUBNET_ROUTES=true` or default) |
| Firewall backend | Explicit: `TS_DEBUG_FIREWALL_MODE=nftables` (match host kernel) |
| IP forwarding | Enabled on host: `sysctl net.ipv4.ip_forward=1` |
| State volume | Dedicated, unique per router instance |
| Hostname | Distinct per router instance |

## High-value diagnostic commands

```bash
docker ps
docker logs <ROUTER_CONTAINER> --tail 200
docker exec -it <ROUTER_CONTAINER> tailscale status
docker exec -it <ROUTER_CONTAINER> tailscale ip -4
docker exec -it <ROUTER_CONTAINER> ping -c 3 <LAN_TARGET_IP>
docker exec -it <ROUTER_CONTAINER> nc -vz <LAN_TARGET_IP> <TARGET_PORT>
docker exec -it <ROUTER_CONTAINER> sh -lc 'ip route'
docker exec -it <ROUTER_CONTAINER> sh -lc 'env | grep ^TS_'
docker exec -it <ROUTER_CONTAINER> sh -lc 'sysctl net.ipv4.ip_forward'
```

For Podman, substitute `podman` for `docker` in all commands.

## Common failure patterns

### Firewall backend mismatch

**Symptom:** Router reachable via Tailscale, route advertised and approved, but LAN services unreachable.

**Root cause:** The container's Tailscale process assumes one firewall backend (e.g., iptables) while the host kernel uses another (e.g., nftables).

**Fix:**
```bash
TS_DEBUG_FIREWALL_MODE=nftables
```

Set this as an environment variable in the container's configuration. Use `iptables` instead if the host kernel requires it.

### IP forwarding disabled

**Symptom:** Same as above -- overlay healthy, LAN unreachable.

**Check:**
```bash
docker exec -it <ROUTER_CONTAINER> sh -lc 'sysctl net.ipv4.ip_forward'
```

**Fix (on the host):**
```bash
sysctl -w net.ipv4.ip_forward=1
# Make persistent:
echo 'net.ipv4.ip_forward = 1' >> /etc/sysctl.d/99-tailscale.conf
```

### SNAT/masquerade not active

**Symptom:** Router can reach the LAN target, but return traffic from the LAN target does not reach the Tailscale client (asymmetric routing).

**Fix:** Ensure SNAT is enabled. In the official Tailscale container image, this is the default. If explicitly disabled, re-enable:
```bash
TS_SNAT_SUBNET_ROUTES=true
```

### Route not approved

**Symptom:** Router online and advertising the subnet, but no client traffic flows through it.

**Fix:** Approve the route in the Tailscale admin console, or ensure ACL `autoApprovers` covers the subnet.

## Safety: concurrent host-network routers

Do not run two host-network, kernel-mode Tailscale containers on the same host simultaneously. Risks:
- Competing packet-filter rules.
- Ambiguous route ownership.
- Conflicting advertisements.
- Intermittent, hard-to-diagnose failures.

## Safe replacement pattern

1. Keep the existing router container stopped but available.
2. Prepare the replacement with a **distinct hostname** and **separate state volume**.
3. Stop the existing router first (if same host, host networking).
4. Start the replacement.
5. Verify route advertisement: `docker exec -it <ROUTER_CONTAINER> tailscale status`.
6. **Approve the new route** in the admin console.
7. Verify from a client: `tailscale ping`, route table, and service reachability.
8. If the replacement fails, stop it and restart the previous router.
