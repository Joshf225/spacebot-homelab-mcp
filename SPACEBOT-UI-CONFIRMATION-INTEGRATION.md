# Spacebot Web UI Confirmation Integration

## Purpose

This note documents the MCP confirmation-flow work that was needed in the main `spacebot` app so destructive `spacebot-homelab-mcp` tools are usable from the Spacebot web UI.

This file is intended for upstream review of `spacebot-homelab-mcp` so it is clear that:

1. The MCP server-side confirmation flow already existed in `spacebot-homelab-mcp`.
2. The missing piece was client support in the main `spacebot` app.
3. Older Spacebot builds may need `[confirm]` rules left disabled until their client/UI supports `confirm_operation`.

## Summary

`spacebot-homelab-mcp` destructive tools such as:

- `docker.container.stop`
- `docker.container.delete`
- `docker.image.delete`
- `docker.image.prune`
- `proxmox.vm.stop`
- `proxmox.vm.create`
- `proxmox.vm.delete`
- `proxmox.vm.snapshot.rollback`

can require confirmation through the MCP tool `confirm_operation`.

The MCP server returns a structured `confirmation_required` response with a token. A compatible client must then call `confirm_operation` with that token and tool name.

Before the Spacebot web UI changes, the user-visible failure mode was:

1. The agent asked for plain-text confirmations like `confirm stop VM 100`.
2. The UI did not expose the pending confirmation token.
3. The agent could get stuck in a text-only confirmation loop.
4. In practice, users often had to disable `[confirm]` rules for destructive homelab commands.

After the initial web UI implementation, retesting surfaced two more client-side issues:

1. The web UI could render the confirmation card before the assistant's pre-confirmation explanation text arrived.
2. Workers could still describe a destructive MCP action with past-tense success wording even when the MCP tool had only returned `confirmation_required`.

These were follow-up `spacebot` issues, not `spacebot-homelab-mcp` protocol issues, and were fixed in later iterations.

## What Already Existed In `spacebot-homelab-mcp`

The MCP server already had the correct server-side behavior:

- destructive tool handlers call the confirmation manager
- `confirmation_required` responses include a token
- `confirm_operation` executes the second step
- audit logging records the destructive operation lifecycle

No protocol change was needed in the MCP server for the web UI flow itself.

## What Had To Change In `spacebot`

The main Spacebot app needed both backend and frontend work.

### Backend / API

- detect MCP `confirmation_required` tool results
- store pending confirmations per portal/web chat session
- expose API endpoints to list pending confirmations
- expose API endpoints to confirm or dismiss a pending confirmation
- call the MCP server's `confirm_operation` tool from the backend
- publish the confirmed result back into the portal conversation transcript
- emit live events so the web UI updates without refresh

### Frontend / Web UI

- fetch pending confirmations for the active portal conversation
- render a visible confirmation card in the chat UI
- provide `Confirm action` and `Dismiss` controls
- remove the card when resolved
- show the confirmed result back in the chat transcript
- delay rendering the confirmation card until the assistant's pre-confirmation message arrives, with a short fallback timeout if the message is delayed

### Prompting / Agent Behavior

The Spacebot channel prompt also needed updates so the agent would:

- stop inventing plain-text confirmation protocols
- stop saying things like `Stopping it now` without actually calling tools
- use a worker for MCP-backed infrastructure actions
- call the destructive MCP tool first, then let the UI handle confirmation
- treat `confirmation_required` as a terminal pre-confirmation state, not as a successful execution result
- keep pre-confirmation and post-confirmation messaging separate

## Files Changed In `spacebot`

### Web UI / API flow

- `src/api/state.rs`
  - Added pending MCP confirmation storage.
  - Added parsing of `confirmation_required` tool results.
  - Added `McpConfirmationPending` and `McpConfirmationResolved` events.

- `src/api/portal.rs`
  - Added portal endpoints to list, confirm, and dismiss pending confirmations.
  - Added portal-side publishing of confirmed results back into chat.

- `src/api/server.rs`
  - Registered the new portal confirmation routes.

- `src/api/system.rs`
  - Added SSE/event-stream support for the new confirmation events.

- `src/mcp.rs`
  - Added backend execution path for `confirm_operation` against the correct MCP server.
  - Added helpers for MCP tool namespacing and tool-result text extraction.

- `src/tools/mcp.rs`
  - Reused the shared MCP namespacing helper so confirmation routing is stable.

- `interface/src/api/client.ts`
  - Added frontend API methods and types for pending confirmations.

- `interface/src/api/schema.d.ts`
  - Regenerated schema/types for the new portal confirmation endpoints.

- `interface/src/components/WebChatPanel.tsx`
  - Added the visible confirmation card UI.
  - Added confirm and dismiss actions.
  - Added live updates for pending/resolved confirmations.
  - Added delayed confirmation-card reveal so the explanatory assistant message can land before the user sees the confirm controls.

- `interface/src/hooks/useLiveContext.tsx`
  - Bridged backend confirmation events into browser events for the portal chat UI.

### Prompting / agent behavior

- `prompts/en/adapters/portal.md.j2`
  - New portal-specific guidance for MCP destructive actions and UI confirmations.
  - Clarified that pre-confirmation messaging must describe pending confirmation only, and post-confirmation messaging should be a separate executed result.

- `prompts/en/channel.md.j2`
  - Added explicit instruction that MCP-backed infrastructure actions are worker tasks.

- `prompts/en/worker.md.j2`
  - Added explicit worker guidance that `confirmation_required` means the destructive action has NOT executed yet.
  - Prevents worker summaries from using past-tense success wording before the user confirms in the web UI.

- `src/prompts/engine.rs`
  - Registered the `portal` adapter prompt and added prompt tests.
  - Added worker-prompt coverage to verify `confirmation_required` guidance stays in the worker system prompt.

- `src/prompts/text.rs`
  - Registered the new `adapters/portal` prompt text entry.

## Files Changed In `spacebot-homelab-mcp`

These were compatibility/documentation changes made in this repo so users understand the client dependency.

- `README.md`
  - Added a compatibility note explaining that `[confirm]` rules require MCP client support for `confirm_operation`.

- `example.config.toml`
  - Added the same warning above the commented `[confirm]` examples.

## Compatibility Guidance

If a user's Spacebot build or UI does not surface pending MCP confirmations yet:

1. Leave the `[confirm]` section disabled for homelab MCP tools.
2. Use `dry_run=true` before destructive operations.
3. Re-enable `[confirm]` once the client supports the MCP confirmation flow.

This is especially important for:

- `proxmox.vm.stop`
- `proxmox.vm.create`
- `proxmox.vm.delete`
- `proxmox.vm.snapshot.rollback`
- Docker destructive operations with `always` confirmation enabled

## Expected Web UI Flow

For a destructive request like `stop VM 100 on proxmox-homelab`:

1. The channel agent spawns a worker.
2. The worker calls `proxmox.vm.stop`.
3. `spacebot-homelab-mcp` returns `confirmation_required` with a token.
4. Spacebot stores the pending confirmation for the portal conversation.
5. Spacebot sends a pre-confirmation assistant message explaining that the action is pending confirmation.
6. The web UI shows a confirmation card after that explanatory message arrives, with a short fallback delay if needed.
7. The user clicks `Confirm action`.
8. Spacebot backend calls `confirm_operation`.
9. The final tool result is posted back into the chat transcript.

## Validation Notes

The web UI integration was validated by:

- enabling Proxmox confirmation rules in the active homelab MCP config
- confirming pending confirmations appeared in the portal UI
- confirming the second-step execution succeeded through `confirm_operation`
- confirming that destructive MCP replies no longer rely on plain-text `confirm ...` loops
- confirming that pre-confirmation text and the confirmation card appear in the intended order
- confirming that worker summaries treat `confirmation_required` as pending, not executed

## Upstream Takeaway

The confirmation system in `spacebot-homelab-mcp` is correct and useful, but it depends on client support.

When users report that destructive homelab MCP commands appear to stall or require awkward plain-text confirmation loops, the likely issue is not the MCP server itself. The likely issue is that the client has not yet implemented the MCP `confirm_operation` UX.
