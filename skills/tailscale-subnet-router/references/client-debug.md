# Client Debug

Multi-platform diagnostic commands for Tailscale clients connecting through a subnet router.

## All platforms -- core status

```bash
tailscale status
tailscale netcheck
tailscale ping <ROUTER_NODE>
```

## Route acceptance

```bash
tailscale set --accept-routes=true
```

Verify the subnet route is installed in the system route table:

- **macOS:** `netstat -rn -f inet | grep <SUBNET_PREFIX>`
- **Linux:** `ip route show | grep <SUBNET_PREFIX>`
- **Windows (PowerShell):** `route print`

## DNS override setting

When Tailscale's DNS override causes connectivity issues (common on cellular or non-home networks):

```bash
tailscale set --accept-routes=true --accept-dns=false
```

This keeps subnet route acceptance enabled while preventing Tailscale from overriding the client's DNS configuration.

## DNS vs transport diagnosis

**DNS-dependent test:**
```bash
curl -I https://example.com
```

**DNS-independent test:**
```bash
curl -I https://1.1.1.1
```

**Platform-specific DNS inspection:**

macOS:
```bash
scutil --dns
```

Linux:
```bash
resolvectl status
# or
cat /etc/resolv.conf
```

Windows (PowerShell):
```powershell
Get-DnsClientServerAddress
Resolve-DnsName example.com
```

**Interpretation:**
- Hostname fails, IP works -> DNS problem. Consider `--accept-dns=false`.
- Both fail -> broader transport or policy issue.

## LAN target reachability (from client)

```bash
ping -c 3 <LAN_TARGET_IP>
nc -vz <LAN_TARGET_IP> <TARGET_PORT>
curl -kI https://<LAN_TARGET_IP>:<TARGET_PORT>
curl --connect-timeout 5 http://<LAN_TARGET_IP>
```

## Full diagnostic sequence

Run these in order for a complete client-side assessment:

1. `tailscale status` -- confirm connected, identify router node.
2. `tailscale ping <ROUTER_NODE>` -- confirm overlay reachability.
3. Check route table for `<SUBNET_PREFIX>` -- confirm route installed.
4. `ping -c 3 <LAN_TARGET_IP>` -- confirm LAN transport.
5. `nc -vz <LAN_TARGET_IP> <TARGET_PORT>` -- confirm service port.
6. `curl -I https://example.com` -- confirm DNS resolution works.
7. If DNS fails: `tailscale set --accept-routes=true --accept-dns=false`.
