---
name: tailscale-subnet-router
description: Diagnose, deploy, replace, and verify Tailscale subnet routers across any environment. Covers client-side route acceptance, admin route approval, DNS vs transport failures, containerized and bare-metal router patterns, firewall backend mismatches, safe cutovers, and multi-platform client debugging.
version: 2.0.0
---

# Tailscale Subnet Router

## Purpose

Use this skill when a Spacebot agent needs to diagnose, deploy, verify, replace, or document a Tailscale subnet router for any private network accessible through a router node.

This playbook separates five commonly conflated failure modes:

- **Exit-node behavior vs subnet-router behavior** -- different symptoms, different fixes.
- **DNS failures vs packet transport failures** -- hostname resolution problems masquerade as routing failures.
- **Route advertisement vs route approval vs route acceptance** -- three independent gates that must all pass.
- **Router overlay reachability vs LAN service reachability** -- Tailscale ping success does not mean the LAN path works.
- **Firewall backend mismatches** -- iptables vs nftables conflicts in containerized routers.

## When to invoke this skill

- Internet connectivity appears to break on a client after Tailscale connects.
- `tailscale ping` succeeds but LAN services behind the subnet router still time out.
- `curl` fails with DNS resolution errors when connected over cellular or non-home networks.
- A subnet route appears installed on the client but traffic does not reach LAN services.
- A replacement subnet router is being introduced and must be tested safely.
- An agent needs to validate the end-to-end routing path from client to LAN target through a Tailscale subnet router.

## Environment variables

Throughout this skill, the following placeholders are used. The agent must resolve these to actual values from the user's environment before executing commands.

| Placeholder | Meaning | Example |
|---|---|---|
| `<SUBNET_CIDR>` | The private subnet advertised by the router | `10.0.1.0/24` |
| `<LAN_TARGET_IP>` | A known-reachable host on the private subnet | `10.0.1.50` |
| `<TARGET_PORT>` | A service port on the LAN target | `443` |
| `<ROUTER_NODE>` | Tailscale node name or 100.x.y.z IP of the router | `subnet-router-1` |
| `<ROUTER_CONTAINER>` | Docker/Podman container name for containerized routers | `ts-subnet-router` |
| `<ROUTER_LAN_IP>` | The router host's LAN IP address | `10.0.1.1` |

## High-confidence lessons learned

These patterns recur across environments and are the most common root causes.

1. **Client DNS override causes internet failure on non-home networks.**
   - Symptom: internet appears broken when Tailscale connects over cellular data or a network where MagicDNS resolvers are unreachable.
   - Fix: disable Tailscale DNS override on the affected client while keeping route acceptance enabled.
     ```bash
     tailscale set --accept-routes=true --accept-dns=false
     ```

2. **Containerized router fails silently due to firewall backend mismatch.**
   - Symptom: route advertised, Tailscale overlay healthy, but LAN services unreachable through the router.
   - Root cause: container assumes iptables but the host kernel uses nftables (or vice versa).
   - Fix: force the correct backend explicitly.
     ```bash
     TS_DEBUG_FIREWALL_MODE=nftables
     ```

3. **Route approval is a separate gate from route advertisement.**
   - A router can be online and advertising a subnet. The client can have `--accept-routes=true`. Traffic still will not flow until the route is **approved in the Tailscale admin console** (or via ACL autoApprovers).
   - This is the single most commonly missed step during router deployment or replacement.

## Symptoms classification

Use the first matching category.

### 1) Internet dies when Tailscale connects

Typical signs:
- Public websites stop loading after Tailscale activates.
- Worse on cellular or non-home networks.
- Disabling Tailscale restores connectivity.

Most likely cause: **Client DNS / MagicDNS conflict**, not subnet routing.

First actions:
```bash
tailscale status
tailscale netcheck
curl -I https://example.com        # DNS-dependent
curl -I --insecure https://1.1.1.1 # DNS-independent
```
If hostname-based requests fail but IP-based succeed, the problem is DNS. Fix:
```bash
tailscale set --accept-routes=true --accept-dns=false
```

### 2) `tailscale ping` works but LAN service access times out

Typical signs:
- `tailscale ping <ROUTER_NODE>` succeeds.
- Router appears online in `tailscale status`.
- Connections to `<LAN_TARGET_IP>:<TARGET_PORT>` hang or time out.

Most likely cause: **Subnet routing path broken** after the Tailscale overlay. Firewall/NAT/backend mismatch, unapproved route, or forwarding failure.

First actions:
- Confirm client accepted routes.
- Confirm admin approved the advertised route.
- Confirm router can reach the LAN target directly.
- Confirm router firewall mode and IP forwarding state.

### 3) `curl` fails with DNS resolution errors

Typical signs:
- "Could not resolve host" errors.
- IP-based connectivity may work fine.

Most likely cause: **DNS failure**, not transport.

First actions:
```bash
curl -I https://example.com
curl -I --insecure https://1.1.1.1
```
Compare results. If IP connectivity works but DNS resolution fails, isolate and test the DNS override path.

Platform-specific DNS inspection:
- **macOS:** `scutil --dns`
- **Linux:** `resolvectl status` or `cat /etc/resolv.conf`
- **Windows:** `Get-DnsClientServerAddress` (PowerShell)

### 4) Subnet route installed but no service reachability

Typical signs:
- Client route table shows `<SUBNET_CIDR>` via Tailscale interface.
- Router is online. `tailscale ping` works.
- Actual LAN connections fail.

Most likely causes:
- Router not forwarding/NATing packets to the LAN.
- Route approval missing or stale.
- Firewall backend mismatch on router.
- Target host firewall blocking traffic from the router's source IP.

First actions:
- Verify admin route approval.
- Test router-to-LAN connectivity from inside the router environment.
- Verify SNAT/masquerade is enabled.
- Verify firewall backend mode.

## Decision tree

Follow in order.

### Step 1: Exit-node problem or subnet-router problem?

**Exit-node indicators:**
- All internet traffic affected.
- Public websites fail immediately.
- Issue appears before contacting any LAN IP.

Go to: **Step 2 (DNS vs transport)**.

**Subnet-router indicators:**
- General internet works fine.
- Only resources in `<SUBNET_CIDR>` are impacted.
- Tailscale overlay reachability is fine but LAN access is not.

Go to: **Step 3 (route acceptance vs route approval)**.

### Step 2: DNS failure vs transport failure

**DNS-oriented checks:**
```bash
curl -I https://example.com
dig example.com
```
Platform-specific:
- macOS: `scutil --dns`
- Linux: `resolvectl status`

**Transport-oriented checks:**
```bash
curl -I --insecure https://1.1.1.1
ping -c 2 1.1.1.1
tailscale netcheck
```

Interpretation:
- Hostname fails, raw IP works -> **DNS failure**.
- Both fail -> broader transport or policy issue.

### Step 3: Client route acceptance vs admin route approval

These are **separate, independent gates**.

**Client route acceptance:**
```bash
tailscale status
tailscale set --accept-routes=true
```
Verify the route is in the system route table:
- macOS: `netstat -rn -f inet | grep <SUBNET_CIDR>`
- Linux: `ip route show | grep <SUBNET_CIDR>`
- Windows: `route print` (PowerShell)

If the client is not accepting routes, fix that first.

**Admin route approval:**

Even with a healthy router and route advertisement, traffic will not flow until the route is approved.

Options:
- **Admin console:** Find the subnet router node, verify the route is approved.
- **ACL autoApprovers:** Ensure the subnet is covered by an `autoApprovers` entry in the Tailscale ACL policy.
- **`tailscale set` on the router:** Confirm the router is advertising the correct subnet: `tailscale set --advertise-routes=<SUBNET_CIDR>`.

If both gates are correct, continue.

### Step 4: Router reachability vs LAN reachability

**Router reachability:**
```bash
tailscale ping <ROUTER_NODE>
tailscale status
```

**LAN reachability (from client):**
```bash
ping -c 3 <LAN_TARGET_IP>
nc -vz <LAN_TARGET_IP> <TARGET_PORT>
curl -kI https://<LAN_TARGET_IP>:<TARGET_PORT>
```

Interpretation:
- Router reachable, LAN target not -> forwarding/NAT/firewall issue on the router, or target host firewall.
- Both reachable -> application-layer issue.

### Step 5: NAT/SNAT/firewall backend mismatch

High-probability branch for containerized subnet routers.

Known failing pattern:
- Route advertised and approved.
- Node online, Tailscale overlay healthy.
- LAN services still unreachable.
- Container firewall backend does not match host kernel reality.

Fix: force the correct backend.
```bash
TS_DEBUG_FIREWALL_MODE=nftables   # or iptables, depending on host
```

Also verify:
- Host-network mode is used (for Docker/Podman routers).
- `<SUBNET_CIDR>` is advertised.
- SNAT/masquerade is enabled.
- IP forwarding is enabled (`sysctl net.ipv4.ip_forward`).

## Client diagnostic commands

### macOS

```bash
tailscale status
tailscale netcheck
tailscale ping <ROUTER_NODE>
tailscale set --accept-routes=true
tailscale set --accept-routes=true --accept-dns=false
netstat -rn -f inet | grep <SUBNET_CIDR>
scutil --dns
```

### Linux

```bash
tailscale status
tailscale netcheck
tailscale ping <ROUTER_NODE>
tailscale set --accept-routes=true
tailscale set --accept-routes=true --accept-dns=false
ip route show | grep <SUBNET_CIDR>
resolvectl status
```

### Windows (PowerShell)

```powershell
tailscale status
tailscale netcheck
tailscale ping <ROUTER_NODE>
tailscale set --accept-routes=true
tailscale set --accept-routes=true --accept-dns=false
route print
Get-DnsClientServerAddress
```

## Service reachability tests

Unix-like platforms:

```bash
ping -c 3 <LAN_TARGET_IP>
nc -vz <LAN_TARGET_IP> <TARGET_PORT>
curl -kI https://<LAN_TARGET_IP>:<TARGET_PORT>
curl --connect-timeout 5 http://<LAN_TARGET_IP>:<TARGET_PORT>
curl -I https://example.com
curl -I --insecure https://1.1.1.1
```

Windows (PowerShell):

```powershell
ping -n 3 <LAN_TARGET_IP>
Test-NetConnection -ComputerName <LAN_TARGET_IP> -Port <TARGET_PORT>
curl.exe -kI https://<LAN_TARGET_IP>:<TARGET_PORT>
curl.exe --connect-timeout 5 http://<LAN_TARGET_IP>:<TARGET_PORT>
curl.exe -I https://example.com
curl.exe -I --insecure https://1.1.1.1
```

## Containerized router checks

Applicable to Docker, Podman, or any OCI container runtime.

```bash
docker ps                                                          # or: podman ps
docker logs <ROUTER_CONTAINER> --tail 200
docker exec -it <ROUTER_CONTAINER> tailscale status
docker exec -it <ROUTER_CONTAINER> tailscale ip -4
docker exec -it <ROUTER_CONTAINER> ping -c 3 <LAN_TARGET_IP>
docker exec -it <ROUTER_CONTAINER> nc -vz <LAN_TARGET_IP> <TARGET_PORT>
docker exec -it <ROUTER_CONTAINER> sh -lc 'ip route'
docker exec -it <ROUTER_CONTAINER> sh -lc 'env | grep ^TS_'
docker exec -it <ROUTER_CONTAINER> sh -lc 'sysctl net.ipv4.ip_forward'
```

## Known working router pattern

The recommended containerized subnet router configuration:

- **Network mode:** host (`network_mode: host` / `--network=host`).
- **Identity:** distinct Tailscale node name and dedicated state volume per router instance.
- **Routes:** `TS_ROUTES=<SUBNET_CIDR>` or `tailscale set --advertise-routes=<SUBNET_CIDR>`.
- **SNAT:** enabled (`TS_SNAT_SUBNET_ROUTES=true` or default behavior).
- **Firewall backend:** explicitly set to match the host kernel (`TS_DEBUG_FIREWALL_MODE=nftables`).
- **IP forwarding:** enabled on the host (`sysctl -w net.ipv4.ip_forward=1`).

## Safety: do not run two host-network Tailscale router containers simultaneously on the same host

Risks:
- Competing packet-filter rule changes.
- Ambiguous route ownership.
- Conflicting route advertisements.
- Hard-to-diagnose intermittent failures.

**Rule:** only one host-network, kernel-mode Tailscale subnet router container should be active per host at any time.

## Safe replacement pattern

Use this cutover method when replacing a subnet router.

1. **Keep the existing router available but stopped.**
   Do not delete it until the replacement is proven stable.

2. **Prepare the replacement with a distinct hostname and separate state volume.**
   Never reuse the same state path while testing a separate node identity.

3. **Stop the existing router before starting the replacement** (if both use host networking on the same host).

4. **Start the replacement and verify route advertisement.**
   ```bash
   docker exec -it <ROUTER_CONTAINER> tailscale status
   ```
   Confirm `<SUBNET_CIDR>` is advertised.

5. **Approve the new node's route in the admin console.**
   This step is easy to miss and blocks real traffic.

6. **Verify from a client:**
   ```bash
   tailscale ping <ROUTER_NODE>
   tailscale status
   # Check route table (platform-specific, see above)
   ping -c 3 <LAN_TARGET_IP>
   nc -vz <LAN_TARGET_IP> <TARGET_PORT>
   ```

7. **If replacement fails, stop it and reactivate the previous router.**
   Keep rollback simple and fast.

## Recommended procedural flow for agents

When debugging any Tailscale subnet routing issue:

1. Gather environment specifics: subnet CIDR, LAN target IP/port, router node name, client OS.
2. Classify the symptom.
3. Determine whether the problem is exit-node-like or subnet-router-like.
4. Separate DNS failure from transport failure.
5. Confirm client route acceptance.
6. Confirm admin route approval (console or ACL autoApprovers).
7. Confirm router overlay reachability (`tailscale ping`).
8. Confirm router-to-LAN connectivity.
9. Check router mode, SNAT, IP forwarding, and firewall backend.
10. If replacing a router, follow the safe replacement pattern.
11. Record the client's DNS acceptance setting and whether it should be disabled for the client's network context.

## Fast diagnosis cheatsheet

| Symptom | Most likely fix |
|---|---|
| Internet dies on cellular/non-home network after Tailscale connects | `tailscale set --accept-routes=true --accept-dns=false` |
| Router online but LAN services unreachable | Check firewall backend: `TS_DEBUG_FIREWALL_MODE=nftables` |
| Replacement router not passing traffic | Approve the new route in admin console |
| Client reaches router but not LAN target | Test router-to-LAN reachability; verify SNAT and IP forwarding |
| Route table shows subnet but connections time out | Verify admin approval + firewall backend + SNAT |

## References

See supporting reference docs in `references/`:

- `troubleshooting-checklist.md` -- quick pre-flight checklist.
- `client-debug.md` -- multi-platform client diagnostic commands.
- `container-router-debug.md` -- containerized router inspection and known fixes.
- `lan-service-connectivity.md` -- end-to-end LAN service reachability tests.
