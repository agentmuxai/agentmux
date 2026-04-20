# App API — `pane.open` Spec

Status: Proposed
Date: 2026-04-20
Depends on: `app-api-extension.md`, `app-api-status.md`

## Motivation

Agents running inside an agent pane can already call the App API over
WebSocket RPC to open/send/stop other agents (`agent.open`,
`agent.send`, etc.). What they **can't** do today is ask the host to
open an arbitrary view — for example, to show a spec file they just
wrote in a preview pane next to their own pane.

Today this requires the user to manually open the pane. If the agent
could request it, typical "write a doc, then show it" and "open this
log while I work" flows become one-shot.

## Goals

1. An agent can request a new pane containing a specific view (preview,
   codeeditor, term, web, sysinfo, help).
2. For file-backed views (preview, codeeditor), the agent supplies a
   path and the pane opens with that file loaded.
3. Placement is controllable: same tab (split beside current pane), new
   tab, or a named tab.
4. The call is idempotent where sensible — opening the same file
   focuses the existing pane instead of creating a duplicate.
5. The call honors AgentMux's existing block permissions and path
   sandboxing (no exposing files outside the user's allowed roots).

## Non-goals

- Creating new view types.
- Controlling pane geometry precisely (width/height in pixels).
- Window-level operations (new window, move to window). Covered
  elsewhere.
- Closing or re-arranging existing panes beyond focusing an
  already-open match.

## Design principles (carried from app-api-extension.md)

- High-level intent, not low-level mechanics. Agent says "show me this
  file in a preview pane," not "CreateBlock with view=preview, SetMeta
  file=…, …".
- Stable contract over internal metadata keys.
- Idempotent for file-backed opens.
- Transport-agnostic: same command over CEF IPC, WebSocket RPC, HTTP
  REST.

## Command: `pane.open`

### Request

```typescript
{
  // What to show
  view: "preview" | "codeeditor" | "term" | "web" | "sysinfo" | "help";

  // View-specific arguments (mutually exclusive by view)
  file?: string;      // absolute or workspace-relative path; required for preview/codeeditor
  url?: string;       // required for web
  cwd?: string;       // optional for term (defaults to current pane's cwd)

  // Placement
  placement?: {
    tab?: "current" | "new" | string;   // default "current"; string = tab name
    split?: "right" | "below" | "tab";  // default "right" when tab=current
    focus?: boolean;                    // default true
  };

  // Behavior
  idempotent?: boolean;  // default true for file/url views, false for term
  title?: string;        // optional explicit pane title
}
```

### Response

```typescript
{
  blockId: string;       // created or reused pane
  tabId: string;
  created: boolean;      // false if we focused an existing match
}
```

### Errors

- `INVALID_VIEW` — unknown view.
- `MISSING_ARG` — view requires `file`/`url`/etc. and it wasn't
  provided.
- `PATH_DENIED` — path falls outside allowed roots.
- `FILE_NOT_FOUND` — file doesn't exist when view=preview/codeeditor
  and `allowMissing` is not set.
- `TAB_NOT_FOUND` — `placement.tab` was a name that doesn't exist and
  `createTab` wasn't requested.

### Idempotency rules

When `idempotent` is true (default for file/url views):
- For `preview` / `codeeditor`: if a pane in the target tab already
  has the same view and the same resolved absolute file path, focus
  it and return `created: false`.
- For `web`: same-origin + same path match → focus.
- `term` / `sysinfo` / `help`: no dedup; always create.

### Path resolution

- Absolute paths are used as-is, then validated against allowed roots.
- Relative paths resolve against the calling pane's `cmd:cwd` (for
  agent panes) or the workspace root.
- Paths containing `..` that escape allowed roots are rejected with
  `PATH_DENIED`.

### Sandboxing

The existing filesystem access controls that apply to the `preview`
and `codeeditor` views (user-allowed roots) apply here. This command
does not widen access — it only surfaces a file the user could open
manually.

## Tier assignment

Tier 2 in the existing app-api-extension.md taxonomy (UI
orchestration, high-value but not on the critical path of agent
lifecycle).

## Implementation sketch

1. Add `PaneOpenRequest` / `PaneOpenResponse` to
   `agentmux-srv/src/backend/rpc_types.rs`.
2. Add a handler in `agentmux-srv/src/server/app_api.rs` that:
   - validates the request (view + required fields),
   - resolves the path / URL,
   - searches the current tab's blocks for an existing match when
     idempotent,
   - otherwise orchestrates `CreateBlock` → `SetMeta` with the view's
     expected metadata keys,
   - applies placement via the existing split/tab layout APIs.
3. Expose the same handler over CEF IPC for in-process callers, and
   (Tier 3) over the HTTP REST surface when it lands.
4. Update `docs/specs/app-api-status.md` with implementation status.

## Discoverability for agents

Update the agent startup/system prompt generation to mention
`pane.open` alongside `agent.send` / `agent.open`, with a one-line
example. Without discoverability, no agent will know to call it.

## Open questions

- Should we ship a thin shell wrapper (`agentmux pane open <file>`)
  so shell-based agents can call this without writing a WebSocket
  client? Aligns with the "CLI tool" gap noted in `app-api-status.md`.
- Placement grammar: do we need `split: "left" | "above"` for
  completeness, or is "right / below / new tab" sufficient for agent
  use cases?
- Should `pane.open` accept multiple files and open them as a group
  (common when writing a spec + opening the affected source files)?
- Permission model: do we want a per-agent capability check before
  honoring `pane.open`, or is the existing App API authkey sufficient?

## Affected files

- `agentmux-srv/src/server/app_api.rs`
- `agentmux-srv/src/backend/rpc_types.rs`
- `agentmux-srv/src/backend/blocks.rs` (or wherever block creation
  lives) — confirm CreateBlock + layout helpers are callable from the
  App API handler
- `frontend/app/store/wshrpc` CEF IPC surface (if we also expose this
  in-process)
- `docs/specs/app-api-extension.md` — cross-reference
- `docs/specs/app-api-status.md` — add status row
