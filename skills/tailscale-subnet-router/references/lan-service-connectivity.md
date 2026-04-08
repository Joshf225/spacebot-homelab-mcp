# LAN Service Connectivity

End-to-end reachability tests for LAN services accessed through a Tailscale subnet router.

## From the client

These tests verify the full path: client -> Tailscale overlay -> subnet router -> LAN -> target service.

```bash
ping -c 3 <LAN_TARGET_IP>
nc -vz <LAN_TARGET_IP> <TARGET_PORT>
curl -kI https://<LAN_TARGET_IP>:<TARGET_PORT>
curl --connect-timeout 5 http://<LAN_TARGET_IP>:<TARGET_PORT>
```

## From the router

These tests verify the router's direct LAN path to the target, bypassing the Tailscale overlay. Run inside the router environment (container exec, SSH, etc.).

```bash
ping -c 3 <LAN_TARGET_IP>
nc -vz <LAN_TARGET_IP> <TARGET_PORT>
curl -kI https://<LAN_TARGET_IP>:<TARGET_PORT>
```

For containerized routers:
```bash
docker exec -it <ROUTER_CONTAINER> ping -c 3 <LAN_TARGET_IP>
docker exec -it <ROUTER_CONTAINER> nc -vz <LAN_TARGET_IP> <TARGET_PORT>
```

## Interpretation matrix

| Client -> Router (tailscale ping) | Router -> LAN target | Client -> LAN target | Diagnosis |
|---|---|---|---|
| OK | OK | OK | Path is healthy. Issue is application-layer. |
| OK | OK | FAIL | SNAT/masquerade issue, or client route not installed. |
| OK | FAIL | FAIL | Router LAN path broken. Fix router networking first. |
| FAIL | -- | -- | Tailscale overlay issue. Check node status, keys, connectivity. |

## Common failure modes

### Client has route but connections time out

Check:
- Is the route **approved** in the admin console?
- Is the client **accepting routes**? (`tailscale set --accept-routes=true`)
- Is the route in the client's system route table?

### Router reaches target but client does not

Check:
- SNAT/masquerade enabled on the router?
- Firewall backend correct? (`TS_DEBUG_FIREWALL_MODE=nftables`)
- IP forwarding enabled on the router host?

### Ping works but service port does not

Check:
- Target host firewall rules (may allow ICMP but block TCP on `<TARGET_PORT>`).
- Service actually running and bound to the correct interface on the target.
- Network segmentation or VLAN rules between router and target.

## Testing multiple LAN targets

When a subnet router advertises a CIDR, all hosts in that range should be reachable. Test at least two distinct targets to confirm the routing path is healthy, not just a single host's reachability:

```bash
ping -c 2 <LAN_TARGET_1>
ping -c 2 <LAN_TARGET_2>
nc -vz <LAN_TARGET_1> <PORT_1>
nc -vz <LAN_TARGET_2> <PORT_2>
```

If one target works and another does not, the problem is target-specific (firewall, service binding) rather than a routing issue.
