# API Troubleshooting

Proxmox VE API authentication, permissions, and error diagnosis reference for Spacebot agents.

## API connectivity basics

- **Default port:** 8006 (HTTPS)
- **Base URL:** `https://<host>:8006/api2/json`
- **Auth header:** `Authorization: PVEAPIToken=USER@REALM!TOKENID=UUID`
- **POST body format:** `application/x-www-form-urlencoded` (not JSON)
- **Response envelope:** `{"data": ...}` wraps all responses

## HTTP status codes

| Code | Meaning | Common cause |
|------|---------|-------------|
| 200 | Success | Normal operation |
| 400 | Bad request | Invalid parameters (wrong VMID format, missing required field) |
| 401 | Unauthorized | Bad token, expired token, wrong token format |
| 403 | Forbidden | Token lacks permissions for this operation/path |
| 404 | Not found | Wrong node name, VMID doesn't exist, wrong API path |
| 500 | Internal server error | Proxmox bug or backend failure |
| 595 | Connection error | Host unreachable, port blocked, TLS handshake failure |

## Authentication issues

### HTTP 401: Unauthorized

**Check token format:**
- Token ID must be `USER@REALM!TOKENID` (e.g., `root@pam!spacebot`).
- Note the `!` separator between user and token name.
- The realm is usually `pam` (Linux PAM) or `pve` (Proxmox VE).

**Check token secret:**
- Must be the UUID returned when the token was created.
- Format: `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`.
- Check for leading/trailing whitespace.

**Check token existence:**
```bash
# On the Proxmox host:
pveum user token list root@pam
```

**Check token expiration:**
```bash
pveum user token list root@pam --output-format json | jq '.[] | select(.tokenid=="spacebot")'
```

### Creating a new API token

```bash
# Full-privilege token (same as user):
pveum user token add root@pam spacebot --privsep 0

# Limited-privilege token (needs separate ACLs):
pveum user token add root@pam spacebot --privsep 1
pveum aclmod / -token 'root@pam!spacebot' -role PVEVMAdmin
```

`--privsep 0` gives the token the user's permissions. `--privsep 1` (default) requires you to assign roles to the token separately.

## Permission issues

### HTTP 403: Forbidden

**Check user/token roles:**
```bash
pveum acl list
pveum user list --output-format json
```

**Common roles:**

| Role | Permissions |
|------|-------------|
| `PVEAdmin` | Full admin (everything except user management) |
| `PVEVMAdmin` | VM/CT lifecycle: create, start, stop, delete, snapshots |
| `PVEVMUser` | VM/CT use: start, stop, console, but not create/delete |
| `PVEAuditor` | Read-only: list, status, but no mutations |
| `PVEDatastoreAdmin` | Storage management |
| `Administrator` | Full access including user management |

**ACL paths:**

ACLs can be scoped to specific paths:
- `/` -- everything
- `/vms/<VMID>` -- specific VM
- `/storage/<STORAGE>` -- specific storage pool
- `/nodes/<NODE>` -- specific node

If a token has `PVEVMAdmin` on `/vms/100` but not on `/vms/200`, it can manage VM 100 but not VM 200.

**Fix: grant broader permissions:**
```bash
# Grant PVEVMAdmin on all VMs:
pveum aclmod /vms -token 'root@pam!spacebot' -role PVEVMAdmin

# Or on everything:
pveum aclmod / -token 'root@pam!spacebot' -role PVEAdmin
```

## TLS/connection issues

### Self-signed certificate errors

Proxmox uses a self-signed cert by default. Spacebot handles this with `verify_tls = false` in config.

If connections still fail:
- Check that port 8006 is reachable: `nc -vz <PVE_HOST_IP> 8006`
- Check the Proxmox proxy service: `systemctl status pveproxy`
- Check for firewall rules: `iptables -L -n` or `pve-firewall status`

### Connection timeouts

- Verify network path: `ping <PVE_HOST_IP>`.
- Check if the `pveproxy` service is running: `systemctl status pveproxy`.
- Check if another service is competing for port 8006.
- Check DNS resolution if using hostname instead of IP.

## Async task handling

### UPID format

Mutating operations return a UPID (Unique Process ID):
```
UPID:pve1:001A2B3C:04D5E6F7:6789ABCD:qmstart:100:root@pam:
```

Format: `UPID:{node}:{pid}:{pstart}:{starttime}:{type}:{id}:{user}:`

### Task status polling

Spacebot tools automatically poll task status. If a tool reports a timeout:

1. The task may still be running. Check via SSH:
   ```bash
   # List recent tasks:
   pvesh get /nodes/<NODE>/tasks --limit 5
   
   # Check specific task:
   pvesh get /nodes/<NODE>/tasks/<UPID>/status
   
   # View task log:
   pvesh get /nodes/<NODE>/tasks/<UPID>/log
   ```

2. Check the Proxmox web UI: Datacenter > Task History.

### Common task failures

| Task type | Common failure | Fix |
|-----------|---------------|-----|
| `qmstart` | "not enough memory" | Free RAM on the node or reduce VM memory |
| `qmstart` | "storage not available" | Check storage pool status |
| `qmclone` | timeout | Large disk + slow storage = long clone time; use linked clone |
| `vzdestroy` | "VM is running" | Stop the VM first |
| `qmsnapshot` | "not enough space" | Clean up old snapshots, free storage |

## Rate limiting

Proxmox doesn't enforce API rate limits, but rapid API calls can overload the `pveproxy` service.

Spacebot supports rate limiting in config:
```toml
[tools.rate_limits]
"proxmox.*" = { per_minute = 15 }
```

Recommended: keep read-only calls at 15-30/minute, mutating calls at 5-10/minute.

## Debugging API responses

To see raw API responses for debugging, use SSH:
```bash
# Direct API call from the Proxmox host:
pvesh get /nodes/pve1/qemu/100/status/current --output-format json-pretty

# Or using curl:
curl -k -H "Authorization: PVEAPIToken=root@pam!spacebot=<UUID>" \
  https://localhost:8006/api2/json/nodes/pve1/qemu/100/status/current | jq .
```
