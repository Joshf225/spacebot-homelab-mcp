# Troubleshooting Checklist

Quick pre-flight checklist for Tailscale subnet router issues. Resolve environment placeholders before use.

## Symptom classification

- [ ] Internet dies when Tailscale connects
- [ ] `tailscale ping` works but LAN services time out
- [ ] `curl` shows DNS resolution errors
- [ ] Subnet route appears installed but services remain unreachable

## Decision points

- [ ] Is this exit-node-like or subnet-router-like?
- [ ] Is the failure DNS or transport?
- [ ] Has the client accepted routes? (`tailscale set --accept-routes=true`)
- [ ] Has the admin approved the advertised subnet route?
- [ ] Is the router itself reachable over Tailscale? (`tailscale ping <ROUTER_NODE>`)
- [ ] Can the router reach the LAN target directly?
- [ ] Is there a firewall backend mismatch (iptables vs nftables)?

## Environment facts to gather

Before debugging, collect these from the user's environment:

| Item | Value |
|---|---|
| Subnet CIDR | `<SUBNET_CIDR>` |
| LAN target IP | `<LAN_TARGET_IP>` |
| Target service port | `<TARGET_PORT>` |
| Router node name | `<ROUTER_NODE>` |
| Router LAN IP | `<ROUTER_LAN_IP>` |
| Client OS | macOS / Linux / Windows |
| Router deployment | Container / Bare metal / VM |

## Common root causes

| Root cause | Fix |
|---|---|
| Client DNS override breaks internet on non-home networks | `tailscale set --accept-routes=true --accept-dns=false` |
| Firewall backend mismatch in containerized router | `TS_DEBUG_FIREWALL_MODE=nftables` (or `iptables`, matching host) |
| Route not approved in admin console | Approve in admin console or add ACL `autoApprovers` entry |
| IP forwarding disabled on router host | `sysctl -w net.ipv4.ip_forward=1` |

## Replacement safety

- [ ] Do not run two host-network Tailscale router containers simultaneously on the same host
- [ ] Keep existing router stopped but available for rollback
- [ ] Use a distinct hostname and separate state volume for the replacement
- [ ] Stop existing router before starting replacement (if same host, host networking)
- [ ] Approve the new node's route in admin console after it advertises
