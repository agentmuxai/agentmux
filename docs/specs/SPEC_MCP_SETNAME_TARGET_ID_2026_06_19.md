# SPEC: `SetName` explicit target-id parameter

**Date:** 2026-06-19
**Status:** Implementing
**Author:** smike
**Related:** `agentmux-mcp/src/main.rs` (SetName tool), `agentmux-common::api_types` (WindowNameRequest etc.)

---

## 1. Problem

`SetName` currently resolves the target element from the calling agent's own context
(its `AGENTMUX_BLOCKID`). This means an agent can only rename *its own* window, tab,
workspace, or pane — it cannot target another element even when it knows the ID.

Combined with `Layout` (which already returns `window_id`, `tab_id`, `workspace_id`
for every element in the instance), the missing piece is a way to pass an explicit ID
through to the rename endpoint.

**Use case (motivating example):**

```
// Discover all windows
Layout(query: "windows")
→ [{ window_id: "88b2e33c-...", name: "" }, ...]

// Name a specific one
SetName(target: "window", name: "Dev", target_id: "88b2e33c-...")
```

---

## 2. Design

Add a single optional `target_id` string parameter to `SetName`. When present it is
forwarded as the explicit id field in the matching request struct; when absent the
existing own-context resolution (from `block_id`) is used unchanged.

| `target`    | `target_id` maps to          | Endpoint |
|-------------|------------------------------|----------|
| `window`    | `WindowNameRequest.window_id` | `POST /api/v1/window/name` |
| `tab`       | `TabNameRequest.tab_id`       | `POST /api/v1/tab/name` |
| `workspace` | `WorkspaceNameRequest.workspace_id` | `POST /api/v1/workspace/name` |
| `pane`      | `PaneTitleRequest.block_id`   | `POST /api/v1/pane/title` |

The srv-side request structs (`WindowNameRequest`, `TabNameRequest`, etc.) already
have these fields — all defined in `agentmux-common::api_types` as of A2 (#1575).
No srv changes needed.

**Auth guard change:** when `target_id` is provided the agent's own `block_id` is no
longer required (we know the target directly). The check is relaxed to only require
`block_id` when `target_id` is absent.

---

## 3. Schema change (mcp SETNAME_TOOL)

```json
{
  "name": "SetName",
  "inputSchema": {
    "type": "object",
    "properties": {
      "target":    { "type": "string", "enum": ["window","tab","pane","workspace"] },
      "name":      { "type": "string" },
      "target_id": {
        "type": "string",
        "description": "Explicit id of the element to rename (window_id / tab_id / workspace_id / block_id). Omit to default to your own."
      }
    },
    "required": ["target", "name"]
  }
}
```

---

## 4. Acceptance criteria

- `SetName(target:"window", name:"X")` — still renames own window (no regression).
- `SetName(target:"window", name:"X", target_id:"<window_id>")` — renames that specific window.
- Same for `tab`, `workspace`, `pane`.
- `cargo check -p agentmux-mcp` green; existing 4 mcp tests pass.
- Works end-to-end: `Layout → window_id → SetName(target_id)` renames the right window.

---

## 5. Files changed

| File | Change |
|------|--------|
| `agentmux-mcp/src/main.rs` | Add `target_id` to `SETNAME_TOOL` schema; extract in handler; pass through |
| `docs/specs/SPEC_MCP_SETNAME_TARGET_ID_2026_06_19.md` | This file |
