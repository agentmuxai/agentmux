# App API — `pane.open` Spec

Status: Partially Implemented (MVP shipped; idempotency, `is_new`, and `mode` pending)
Date: 2026-04-20
Updated: 2026-06-14
Depends on: `app-api-extension.md`, `app-api-status.md`

## Implementation Status (as of 2026-06-14)

| Feature | Status |
|---------|--------|
| Basic `pane.open` handler in `app_api.rs` | ✅ Shipped (skeleton) |
| `view`, `file`, `url`, `cwd`, `title`, `tab_id`, `split_direction`, `split_reference_block_id`, `focus` | ✅ Shipped in `CommandPaneOpenData` |
| Idempotency (focus existing pane on same file) | ❌ Not implemented |
| Path sandboxing / allowed-roots validation | ❌ Not implemented |
| `is_new` (scratch/untitled file creation) | ❌ Not implemented — see `SPEC_EDITOR_WIDGET_DEFAULT_UX_2026_06_14.md` |
| `mode: "preview" \| "pinned"` | ❌ Not implemented |
| `language` hint | ❌ Not implemented |
| `amux open <path>` CLI bridge | ❌ Not implemented — see `ANALYSIS_AGENT_APP_API_OPEN_IN_EDITOR_2026_05_30.md` |

> **Note:** The view name in the live RPC is `"editor"` (not `"codeeditor"` as written below in the original design). All future code should use `"editor"`. The spec body below is being kept historically accurate and updated with corrections inline.

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
  // NOTE: live RPC uses "editor" not "codeeditor" — codeeditor is deprecated
  view: "preview" | "editor" | "term" | "web" | "sysinfo" | "help";

  // View-specific arguments (mutually exclusive by view)
  file?: string;      // absolute or workspace-relative path; required for editor/preview unless is_new=true
  is_new?: boolean;   // PENDING: if true, open a scratch/untitled buffer (no file path required)
  language?: string;  // PENDING: hint for syntax highlighting ("markdown", "typescript", etc.)
  url?: string;       // required for web
  cwd?: string;       // optional for term (defaults to current pane's cwd)

  // Placement
  placement?: {
    tab?: "current" | "new" | string;                       // default "current"; string = tab name
    split?: "right" | "below" | "left" | "above" | "tab";  // default "right" when tab=current
    focus?: boolean;                                         // default true
  };

  // Behavior
  idempotent?: boolean;  // PENDING: default true for file/url views, false for term
  mode?: "preview" | "pinned";  // PENDING: default "pinned"; "preview" = single-click VS Code style tab
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

## Open questions (resolved 2026-06-14)

- **Shell wrapper:** Yes — `amux open <file>` is Phase 1 of the agent CLI bridge.
  Design finalized in `ANALYSIS_AGENT_APP_API_OPEN_IN_EDITOR_2026_05_30.md`.
  Pending spec approval before implementation.
- **Placement grammar:** Extended to include `"left"` and `"above"` (added above).
  Full four-direction split is necessary for agent-driven layouts.
- **Multiple files:** Yes — a future `files: string[]` variant should open them as
  a tab group. Not in scope for Phase 1 (single-file only).
- **Permission model:** Existing App API authkey is sufficient for Phase 1. Per-agent
  capability flags are a Phase 3+ concern alongside the MCP façade.
- **`is_new` flow:** Scratch file semantics are specified in
  `SPEC_EDITOR_WIDGET_DEFAULT_UX_2026_06_14.md`. The `pane.open` protocol simply
  accepts `is_new: true` (no `file` field) and delegates scratch file creation to
  the `ScratchFileService`.

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
