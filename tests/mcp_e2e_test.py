#!/usr/bin/env python3
"""
End-to-end MCP stdio test harness for spacebot-homelab-mcp.

Starts the server as a subprocess, sends JSON-RPC requests over stdin,
reads responses from stdout, and validates each phase.
"""

import json
import subprocess
import sys
import os
import time
import select

BINARY = os.path.expanduser(
    "~/Developer/spacebot-homelab-mcp/target/release/spacebot-homelab-mcp"
)
CONFIG = os.path.expanduser("~/.spacebot-homelab/config.toml")
HOST = "proxmox_dev"
TEST_CONTAINER = "mcp-e2e-test-nginx"

PASS = 0
FAIL = 0
RESULTS = []


def result(name, passed, detail=""):
    global PASS, FAIL
    tag = "\033[32mPASS\033[0m" if passed else "\033[31mFAIL\033[0m"
    if passed:
        PASS += 1
    else:
        FAIL += 1
    line = f"  [{tag}] {name}"
    if detail and not passed:
        line += f"  -- {detail}"
    print(line)
    RESULTS.append((name, passed, detail))


class McpClient:
    def __init__(self):
        env = os.environ.copy()
        self.proc = subprocess.Popen(
            [BINARY, "server", "--config", CONFIG],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=env,
        )
        self._id = 0

    def next_id(self):
        self._id += 1
        return self._id

    def send(self, method, params=None):
        req_id = self.next_id()
        msg = {"jsonrpc": "2.0", "id": req_id, "method": method}
        if params is not None:
            msg["params"] = params
        line = json.dumps(msg) + "\n"
        self.proc.stdin.write(line.encode())
        self.proc.stdin.flush()
        return req_id

    def send_notification(self, method, params=None):
        msg = {"jsonrpc": "2.0", "method": method}
        if params is not None:
            msg["params"] = params
        line = json.dumps(msg) + "\n"
        self.proc.stdin.write(line.encode())
        self.proc.stdin.flush()

    def recv(self, timeout=30):
        """Read one JSON-RPC response (skip notifications)."""
        deadline = time.time() + timeout
        buf = b""
        while time.time() < deadline:
            remaining = deadline - time.time()
            if remaining <= 0:
                break
            ready, _, _ = select.select([self.proc.stdout], [], [], min(remaining, 0.1))
            if ready:
                chunk = os.read(self.proc.stdout.fileno(), 8192)
                if not chunk:
                    break
                buf += chunk
                while b"\n" in buf:
                    line, buf = buf.split(b"\n", 1)
                    line = line.strip()
                    if not line:
                        continue
                    try:
                        msg = json.loads(line)
                    except json.JSONDecodeError:
                        continue
                    if "id" not in msg:
                        continue
                    return msg
        return None

    def call(self, method, params=None, timeout=30):
        """Send a request and wait for the matching response."""
        req_id = self.send(method, params)
        deadline = time.time() + timeout
        while time.time() < deadline:
            resp = self.recv(timeout=max(deadline - time.time(), 1))
            if resp is None:
                return None
            if resp.get("id") == req_id:
                return resp
        return None

    def close(self):
        try:
            self.proc.stdin.close()
        except Exception:
            pass
        try:
            self.proc.terminate()
            self.proc.wait(timeout=5)
        except Exception:
            self.proc.kill()


def parse_content(resp):
    """Extract the inner content JSON from an MCP tool result.

    MCP returns: { result: { content: [{ type: "text", text: "<json>" }] } }
    The text is our execution-proof envelope JSON which has its own
    "content" field containing the actual payload (also JSON-encoded).
    """
    try:
        r = resp["result"]
        if "content" in r:
            for item in r["content"]:
                if item.get("type") == "text":
                    return json.loads(item["text"])
        return r
    except (KeyError, json.JSONDecodeError, TypeError):
        return resp


def parse_inner(content):
    """From our execution-proof envelope, get the inner 'content' payload."""
    if content is None:
        return None
    inner = content.get("content")
    if isinstance(inner, str):
        try:
            return json.loads(inner)
        except (json.JSONDecodeError, TypeError):
            return content
    return inner if inner is not None else content


def raw_text(resp):
    """Get raw text from MCP result (for error messages that aren't JSON)."""
    try:
        for item in resp["result"]["content"]:
            if item.get("type") == "text":
                return item["text"]
    except (KeyError, TypeError):
        pass
    return str(resp)


def main():
    print("=" * 60)
    print("spacebot-homelab-mcp  E2E Test Suite")
    print("=" * 60)

    client = McpClient()
    time.sleep(1)

    # ─── Phase 0: Initialize ───
    print("\n--- Phase 0: MCP Handshake ---")
    resp = client.call("initialize", {
        "protocolVersion": "2024-11-05",
        "capabilities": {},
        "clientInfo": {"name": "e2e-test", "version": "1.0.0"},
    })
    ok = resp is not None and "result" in resp
    result("initialize returns result", ok, str(resp)[:200] if not ok else "")
    if ok:
        si = resp["result"].get("serverInfo", {})
        result("server name", si.get("name") == "spacebot-homelab-mcp",
               f"got: {si.get('name')}")
        result("server version is 0.2.0", si.get("version") == "0.2.0",
               f"got: {si.get('version')}")
        result("protocol version 2024-11-05",
               resp["result"].get("protocolVersion") == "2024-11-05")

    client.send_notification("notifications/initialized")
    time.sleep(0.5)

    # ─── Phase 1: tools/list ───
    print("\n--- Phase 1: Tool Discovery ---")
    resp = client.call("tools/list")
    ok = resp is not None and "result" in resp
    result("tools/list returns result", ok)
    if ok:
        tools = resp["result"].get("tools", [])
        tool_names = sorted([t["name"] for t in tools])
        result(f"tool count is 18", len(tools) == 18, f"got {len(tools)}: {tool_names}")

        expected = [
            "docker.container.list", "docker.container.start",
            "docker.container.stop", "docker.container.logs",
            "docker.container.inspect", "docker.container.delete",
            "docker.container.create",
            "docker.image.list", "docker.image.pull",
            "docker.image.inspect", "docker.image.delete", "docker.image.prune",
            "ssh.exec", "ssh.upload", "ssh.download",
            "confirm_operation",
            "audit.verify_operation", "audit.verify_container_state",
        ]
        for t in expected:
            result(f"  tool '{t}' registered", t in tool_names)

        tool_map = {t["name"]: t for t in tools}
        def annot(name, key):
            return tool_map.get(name, {}).get("annotations", {}).get(key)

        result("docker.container.list readOnlyHint=true",
               annot("docker.container.list", "readOnlyHint") is True)
        result("docker.container.delete destructiveHint=true",
               annot("docker.container.delete", "destructiveHint") is True)
        result("audit.verify_operation readOnlyHint=true",
               annot("audit.verify_operation", "readOnlyHint") is True)
        result("ssh.exec openWorldHint=true",
               annot("ssh.exec", "openWorldHint") is True)

    # ─── Phase 2: Non-Destructive Operations ───
    print("\n--- Phase 2: Non-Destructive Operations ---")

    def check_envelope(content, label):
        """Validate execution proof envelope fields."""
        # Actual envelope keys: server_nonce, server_version, executed_at, source, type, content, data_classification
        has = all(k in content for k in ["server_nonce", "server_version", "executed_at"])
        result(f"  {label}: execution proof envelope (server_nonce, server_version, executed_at)", has,
               f"keys: {list(content.keys())}")
        if has:
            result(f"  {label}: server_version=0.2.0",
                   content.get("server_version") == "0.2.0",
                   f"got: {content.get('server_version')}")

    # docker.container.list
    resp = client.call("tools/call", {
        "name": "docker.container.list",
        "arguments": {"host": HOST, "all": True},
    })
    content = parse_content(resp) if resp else None
    result("docker.container.list succeeds", content is not None)
    if content:
        check_envelope(content, "container.list")

    # docker.image.list
    resp = client.call("tools/call", {
        "name": "docker.image.list",
        "arguments": {"host": HOST},
    })
    content = parse_content(resp) if resp else None
    result("docker.image.list succeeds", content is not None)
    if content:
        check_envelope(content, "image.list")

    # ssh.exec uptime
    resp = client.call("tools/call", {
        "name": "ssh.exec",
        "arguments": {"host": HOST, "command": "uptime"},
    })
    content = parse_content(resp) if resp else None
    result("ssh.exec 'uptime' succeeds", content is not None)
    if content:
        check_envelope(content, "ssh.exec")
        full = str(content)
        has_uptime = "load average" in full.lower() or "up" in full.lower()
        result("  ssh.exec: output contains uptime info", has_uptime,
               f"content: {full[:150]}")

    # ─── Phase 3: Create Throwaway Container ───
    print("\n--- Phase 3: Create Throwaway Test Container ---")

    resp = client.call("tools/call", {
        "name": "docker.image.pull",
        "arguments": {"host": HOST, "image": "nginx:alpine"},
    }, timeout=60)
    content = parse_content(resp) if resp else None
    result("docker.image.pull nginx:alpine", content is not None)

    resp = client.call("tools/call", {
        "name": "docker.container.create",
        "arguments": {"host": HOST, "image": "nginx:alpine", "name": TEST_CONTAINER},
    }, timeout=30)
    content = parse_content(resp) if resp else None
    result("docker.container.create succeeds", content is not None)
    if content:
        check_envelope(content, "container.create")

    resp = client.call("tools/call", {
        "name": "docker.container.start",
        "arguments": {"host": HOST, "container": TEST_CONTAINER},
    })
    content = parse_content(resp) if resp else None
    result("docker.container.start succeeds", content is not None)

    # Verify it's running via inspect
    resp = client.call("tools/call", {
        "name": "docker.container.inspect",
        "arguments": {"host": HOST, "container": TEST_CONTAINER},
    })
    content = parse_content(resp) if resp else None
    found = content is not None and TEST_CONTAINER in str(content)
    result("  container visible in inspect", found,
           f"content: {str(content)[:200]}" if not found else "")

    # ─── Phase 4: Anti-Hallucination Verification ───
    print("\n--- Phase 4: Anti-Hallucination Verification ---")

    # audit.verify_operation — check container.create was logged
    resp = client.call("tools/call", {
        "name": "audit.verify_operation",
        "arguments": {"tool_name": "docker.container.create", "contains": TEST_CONTAINER, "last_minutes": 5},
    })
    content = parse_content(resp) if resp else None
    result("audit.verify_operation (container.create)", content is not None)
    if content:
        inner = parse_inner(content)
        result("  verified=true", inner.get("verified") is True,
               f"verified={inner.get('verified')}, keys={list(inner.keys())}")

    # audit.verify_container_state — running container
    resp = client.call("tools/call", {
        "name": "audit.verify_container_state",
        "arguments": {"host": HOST, "container": TEST_CONTAINER, "expected_state": "running"},
    })
    content = parse_content(resp) if resp else None
    result("verify_container_state (expect running)", content is not None)
    if content:
        inner = parse_inner(content)
        result("  verified=true", inner.get("verified") is True,
               f"verified={inner.get('verified')}, actual={inner.get('actual_state')}")

    # audit.verify_container_state — nonexistent → absent (404 fix test)
    resp = client.call("tools/call", {
        "name": "audit.verify_container_state",
        "arguments": {"host": HOST, "container": "nonexistent-xyz-99999", "expected_state": "absent"},
    })
    content = parse_content(resp) if resp else None
    result("verify_container_state (expect absent, 404 fix)", content is not None)
    if content:
        inner = parse_inner(content)
        result("  verified=true (Docker 404 confirms absent)", inner.get("verified") is True,
               f"verified={inner.get('verified')}, actual={inner.get('actual_state')}, detail={inner.get('detail', '')[:120]}")

    # ─── Phase 5: Confirmation Flow ───
    # Test both docker.container.stop AND docker.container.delete confirmation flows.
    # Both are configured as confirm="always" in config.toml.
    print("\n--- Phase 5: Confirmation Flow (stop + delete container) ---")

    # ── 5a: Stop via confirmation flow ──
    resp = client.call("tools/call", {
        "name": "docker.container.stop",
        "arguments": {"host": HOST, "container": TEST_CONTAINER},
    })
    content = parse_content(resp) if resp else None
    result("docker.container.stop returns response", content is not None)

    stop_token = None
    if content:
        inner = parse_inner(content)
        status = inner.get("status", "")
        stop_token = inner.get("token", "")
        result("  stop: status=confirmation_required", status == "confirmation_required",
               f"got status='{status}', inner keys={list(inner.keys())}")
        result("  stop: confirmation token present", bool(stop_token),
               f"token={stop_token[:30]}..." if stop_token else "no token found")

    if stop_token:
        # Confirm the stop
        resp = client.call("tools/call", {
            "name": "confirm_operation",
            "arguments": {"token": stop_token, "tool_name": "docker.container.stop"},
        })
        content = parse_content(resp) if resp else None
        result("confirm_operation executes stop", content is not None)
        if content:
            check_envelope(content, "confirmed stop")

        time.sleep(1)

        # Verify container is now exited
        resp = client.call("tools/call", {
            "name": "audit.verify_container_state",
            "arguments": {"host": HOST, "container": TEST_CONTAINER, "expected_state": "exited"},
        })
        content = parse_content(resp) if resp else None
        if content:
            inner = parse_inner(content)
            result("  verify_container_state confirms exited after confirmed stop",
                   inner.get("verified") is True,
                   f"verified={inner.get('verified')}, actual={inner.get('actual_state')}")
    else:
        result("  (skipping stop confirm — no token)", False, "stop confirmation token was not returned")

    time.sleep(1)

    # ── 5b: Delete via confirmation flow ──
    resp = client.call("tools/call", {
        "name": "docker.container.delete",
        "arguments": {"host": HOST, "container": TEST_CONTAINER},
    })
    content = parse_content(resp) if resp else None
    result("docker.container.delete returns response", content is not None)

    token = None
    if content:
        inner = parse_inner(content)
        status = inner.get("status", "")
        token = inner.get("token", "")
        result("  delete: status=confirmation_required", status == "confirmation_required",
               f"got status='{status}', inner keys={list(inner.keys())}")
        result("  delete: confirmation token present", bool(token),
               f"token={token[:30]}..." if token else "no token found")

    if token:
        # Step 2: Confirm the delete
        resp = client.call("tools/call", {
            "name": "confirm_operation",
            "arguments": {"token": token, "tool_name": "docker.container.delete"},
        })
        content = parse_content(resp) if resp else None
        result("confirm_operation executes delete", content is not None)
        if content:
            check_envelope(content, "confirmed delete")

        time.sleep(1)

        # Verify the delete was audit-logged
        resp = client.call("tools/call", {
            "name": "audit.verify_operation",
            "arguments": {"tool_name": "docker.container.delete", "contains": TEST_CONTAINER, "last_minutes": 5},
        })
        content = parse_content(resp) if resp else None
        if content:
            inner = parse_inner(content)
            result("  audit.verify_operation confirms delete logged",
                   inner.get("verified") is True,
                   f"verified={inner.get('verified')}")

        # Verify container state is absent (full loop: confirm → execute → verify)
        resp = client.call("tools/call", {
            "name": "audit.verify_container_state",
            "arguments": {"host": HOST, "container": TEST_CONTAINER, "expected_state": "absent"},
        })
        content = parse_content(resp) if resp else None
        if content:
            inner = parse_inner(content)
            result("  verify_container_state confirms absent after confirmed delete",
                   inner.get("verified") is True,
                   f"verified={inner.get('verified')}, actual={inner.get('actual_state')}")

        # Mark container as already deleted so cleanup phase can skip
        TEST_CONTAINER_DELETED = True
    else:
        TEST_CONTAINER_DELETED = False
        result("  (skipping confirm + verify — no token)", False, "confirmation token was not returned")

    # ─── Phase 6: Security Guardrails ───
    print("\n--- Phase 6: Security Guardrails ---")

    # Blocked SSH command — "rm -rf" is in blocked_patterns
    resp = client.call("tools/call", {
        "name": "ssh.exec",
        "arguments": {"host": HOST, "command": "rm -rf /tmp/test"},
    })
    text = raw_text(resp)
    rejected = "does not match" in text.lower() or "blocked" in text.lower() or "not allowed" in text.lower()
    result("ssh.exec 'rm -rf' rejected", rejected,
           f"response text: {text[:150]}" if not rejected else "")

    # Blocked SSH — command injection via semicolon
    resp = client.call("tools/call", {
        "name": "ssh.exec",
        "arguments": {"host": HOST, "command": "uptime; cat /etc/passwd"},
    })
    text = raw_text(resp)
    rejected = "does not match" in text.lower() or "blocked" in text.lower() or "not allowed" in text.lower()
    result("ssh.exec injection via ';' rejected", rejected,
           f"response text: {text[:150]}" if not rejected else "")

    # Rate limiting (docker.container.list limit is 5/min)
    # Previous calls in this session have already consumed some quota.
    # Use docker.container.logs on the stopped container as a fresh endpoint to test.
    print("\n  Rate limit test (docker.image.list: 5/min, fresh quota)...")
    rate_limited = False
    for i in range(8):
        resp = client.call("tools/call", {
            "name": "docker.image.list",
            "arguments": {"host": HOST},
        }, timeout=10)
        text = raw_text(resp)
        if "rate" in text.lower() and "limit" in text.lower():
            rate_limited = True
            result(f"  rate limited on request #{i+1}", True)
            break
    if not rate_limited:
        result("  rate limiting triggered within 8 requests", False,
               "no rate limit response observed")

    # ─── Phase 7: Cleanup ───
    print("\n--- Phase 7: Cleanup ---")

    if TEST_CONTAINER_DELETED:
        print("  Container already deleted in Phase 5 — verifying absent state only.")
        resp = client.call("tools/call", {
            "name": "audit.verify_container_state",
            "arguments": {"host": HOST, "container": TEST_CONTAINER, "expected_state": "absent"},
        })
        content = parse_content(resp) if resp else None
        if content:
            inner = parse_inner(content)
            result("cleanup: verified absent (already deleted in Phase 5)",
                   inner.get("verified") is True,
                   f"verified={inner.get('verified')}, actual={inner.get('actual_state')}")
        else:
            result("cleanup: verify_container_state response", False, "no response")
    else:
        # Delete the test container (requires confirmation since it's configured as "always")
        resp = client.call("tools/call", {
            "name": "docker.container.delete",
            "arguments": {"host": HOST, "container": TEST_CONTAINER, "force": True},
        })
        content = parse_content(resp) if resp else None
        token = None
        if content:
            inner = parse_inner(content)
            token = inner.get("token", "")

        if token:
            resp = client.call("tools/call", {
                "name": "confirm_operation",
                "arguments": {"token": token, "tool_name": "docker.container.delete"},
            })
            content = parse_content(resp) if resp else None
            result("cleanup: container deleted", content is not None)

            time.sleep(1)
            resp = client.call("tools/call", {
                "name": "audit.verify_container_state",
                "arguments": {"host": HOST, "container": TEST_CONTAINER, "expected_state": "absent"},
            })
            content = parse_content(resp) if resp else None
            if content:
                inner = parse_inner(content)
                result("cleanup: verified absent",
                       inner.get("verified") is True,
                       f"verified={inner.get('verified')}, actual={inner.get('actual_state')}")
        else:
            result("cleanup: delete (no token returned)", False,
                   f"content: {str(content)[:200]}")

    # ─── Summary ───
    print("\n" + "=" * 60)
    total = PASS + FAIL
    print(f"Results:  \033[32m{PASS} passed\033[0m,  \033[31m{FAIL} failed\033[0m  (total: {total})")
    print("=" * 60)

    client.close()
    return 0 if FAIL == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
