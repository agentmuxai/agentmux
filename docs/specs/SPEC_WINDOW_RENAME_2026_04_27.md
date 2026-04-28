# Window Rename + Click Behaviors in InstancePanel

**Date**: 2026-04-27
**Author**: AgentA-asaf
**Status**: Draft (proposed; not yet implemented)
**Scope**: `frontend/app/statusbar/InstancePanel.tsx`, plus light backend support for persisting the user's chosen window name across restarts.

---

## 0. Quality bar

- Single source of truth for the name (no display-vs-stored drift).
- Rename UX matches the platform conventions users already see in this app (inline edit, Enter/Escape, blur-to-commit).
- Rename survives full app restart and survives tear-off / merge / cancel-back.
- Default name when unset is computed (not blank), so the panel is never empty rows.
- Empty / whitespace-only names are rejected silently — the row reverts to the previous name.

## 1. Goals

The InstancePanel rows currently display either `Window 1` (main) or `Window N` (others), with no way to distinguish between two same-shaped instances at a glance. This spec covers:

1. **Single click on a row** → focus that window (bring to foreground, switch to it).
2. **Double click on a row** → enter rename mode for that row's display name.
3. **Persistence** so renamed windows keep their names across:
   - Full app restart
   - Tear-off / cancel-back round-trips
   - Multi-instance launches (each instance preserves its own names)

## 2. UX behavior

### 2.1 Single click → focus

- Hover state: row gets the existing `instance-panel-row-hover` style (already present).
- Click anywhere on the row (label area, padding) → call `getApi().focusWindow(label)`.
- Already focused row (label === own window's label): no-op (current behavior preserved).
- After focus, the panel closes (current behavior).

This is **already implemented** in `InstancePanel.tsx::handleFocusWindow` — no spec changes needed beyond keeping current behavior.

### 2.2 Double click → enter rename mode

- Double click on a row → row's label converts to an inline `<input type="text">`:
  - Pre-filled with the current name
  - All text selected (so typing replaces it immediately)
  - Receives focus
- The row's icon / hover affordances (focus indicator) remain visible during rename.
- The window-focus side-effect of single click is suppressed during the rename. Specifically: after the user starts a double-click (mousedown count = 2), do NOT fire `focusWindow`.

#### 2.2.1 Commit / cancel

- **Enter** key → commit. Validate (see §2.3); if valid, persist and exit edit mode.
- **Escape** key → cancel. Discard input value, revert display to pre-edit name.
- **Blur** (click outside the input, or the panel closes) → commit (treat as Enter).
- The input's keyboard events are stopped from propagating to the global key handler — Enter must not trigger app-wide actions while in rename mode.

#### 2.2.2 Validation

- Trim leading/trailing whitespace before commit.
- Reject:
  - Empty string after trim → revert to previous name (no error toast — silent revert is fine for this UX).
  - Length > 64 characters → truncate to 64.
- Accept any other Unicode (no character whitelist — let the user use emoji, non-ASCII, etc.).

### 2.3 Default name when unset

When the user has not assigned a name, derive a default in this priority:

1. **Workspace name** — `workspace.name` if non-empty.
2. **Index-based fallback** — `Window 1`, `Window 2`, ... by row position (current behavior).

The user's renamed value, if any, takes precedence over both.

The default is computed at render time only — it is NOT persisted as the user's chosen name. This way, renaming the underlying workspace is reflected in unset windows (because they fall through to step 1), without overwriting any user-set names.

## 3. Data model

### 3.1 Where the name lives

Two storage candidates with their tradeoffs:

| Candidate | Pros | Cons |
|---|---|---|
| **Window record (backend `obj.rs::Window`)** | Persisted across restarts, follows the window's `oid`, single writer | Requires a backend object change + RPC; subwindow / pool windows wouldn't have a backend Window record yet |
| **WindowMeta (host `state::WindowMeta`)** | Process-local, fast | Lost on restart; user expectations on "rename a window" imply persistence |

**Decision: store on the backend `Window` record** as a new optional field `display_name: Option<String>`. Justification: rename is a deliberate user action; the user expects it to survive restart. Process-local would force them to re-rename every launch.

Plumbing already exists: each frontend window has a `windowId` (`WaveWindow.oid`, distinct from the host label). The frontend resolves backend Window via existing channels. The renamed value goes through `ObjectService.UpdateObjectMeta` — same path used for workspace name/icon/color.

### 3.2 Schema change

In `agentmux-srv/src/backend/obj.rs`, the `Window` struct gains:

```rust
pub struct Window {
    pub oid: String,
    pub version: i64,
    pub workspaceid: String,
    // ... existing fields ...
    /// User-assigned display name shown in the InstancePanel and any
    /// other window-list UI. None → fall back to workspace name, then
    /// to the index-based "Window N" label. Persisted across restarts.
    /// Trimmed and length-capped at 64 chars on write.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}
```

`#[serde(default)]` ensures forward/backward compat with existing on-disk Windows.

### 3.3 RPC

Use `ObjectService.UpdateObjectMeta` (already exists) with key `"window:displayname"` rather than adding a dedicated RPC. The meta map is the standard place for user-tunable per-object fields and avoids growing the service surface.

Storing in meta vs the typed field is a coin-flip; meta is preferred here because:
- No backend schema bump needed
- Existing code paths handle meta updates uniformly
- Other tunables (workspace color, tab color) follow the same pattern

The typed field option from §3.2 is the alternative if we ever need stronger typing for tools that introspect Window records. For now: meta.

## 4. Frontend changes

### 4.1 Component shape

Change `InstancePanel.tsx`'s row rendering:

```tsx
// Pseudo-code shape, not final source
const [editingLabel, setEditingLabel] = createSignal<string | null>(null);
const [editValue, setEditValue] = createSignal("");

const handleRowClick = (label: string) => {
    if (editingLabel() === label) return; // ignore inside-input clicks
    handleFocusWindow(label);
};

const handleRowDblClick = (label: string, currentName: string) => {
    setEditingLabel(label);
    setEditValue(currentName);
    // focus + select on next tick
};

const commitRename = async (label: string) => {
    const trimmed = editValue().trim().slice(0, 64);
    setEditingLabel(null);
    if (!trimmed) return; // silent revert on empty
    const windowId = await getWindowIdForLabel(label); // see §4.2
    if (!windowId) return;
    await ObjectService.UpdateObjectMeta(
        makeORef("window", windowId),
        { "window:displayname": trimmed },
    );
};
```

Row JSX:

```tsx
<div onClick={[handleRowClick, label]} onDblClick={[handleRowDblClick, label, currentName]}>
    {editingLabel() === label
        ? <input
            value={editValue()}
            onInput={e => setEditValue(e.currentTarget.value)}
            onKeyDown={e => {
                if (e.key === "Enter") commitRename(label);
                else if (e.key === "Escape") setEditingLabel(null);
                e.stopPropagation();
            }}
            onBlur={() => commitRename(label)}
            ref={el => el && (el.focus(), el.select())}
          />
        : <span>{currentName}</span>}
</div>
```

### 4.2 Resolving label → backend windowId

The host already maps host-label → backend windowId via `state.window_id_map` (see `client.rs`). Frontend reads it via the existing `registerBackendWindow` round-trip; the InstancePanel will need access. Two options:

1. **Add a host RPC** `getBackendWindowIdForLabel(label) → Option<string>`.
2. **Pre-load** the mapping into `openWindowLabelsAtom` as `Array<{ label: string, windowId: string }>` instead of `Array<string>`.

**Decision: (2)** — fewer round-trips, the data is already at hand, and the InstancePanel needs both pieces (label for focus, windowId for rename).

### 4.3 Reactive name resolution

The displayed name for each row is computed from a chain of fallbacks:

```ts
function resolveName(entry: { label: string, windowId: string }, workspaces: WorkspaceMap): string {
    const win = getObjectValue<Window>(makeORef("window", entry.windowId));
    const userName = win?.meta?.["window:displayname"];
    if (typeof userName === "string" && userName.trim()) return userName.trim();
    const ws = workspaces[win?.workspaceid ?? ""];
    if (ws?.name) return ws.name;
    return null; // signal "use index fallback"
}
```

The row uses `resolveName(...) ?? \`Window ${idx + 1}\``. Updates to `window:displayname` flow through Wave's existing object subscription (the panel already subscribes to `WaveWindow` objects).

## 5. Edge cases

- **Two windows with the same name**: allowed. Disambiguation falls to the user; no auto-suffix.
- **Renaming the main window** (label `"main"`, windowId is the primary client's): allowed. Same path as any other window.
- **Tear-off mid-rename**: if the user is editing a row when a tear-off completes (new row appears), the edit state should NOT be disrupted — `editingLabel()` is keyed by label, so the new row renders normally.
- **Window closed mid-rename**: the row disappears from `openWindowLabelsAtom`. The edit input unmounts; no commit fires (no `onBlur` on an unmounted element). User's typed value is lost — acceptable, the window is gone.
- **Pool window in the list**: should not happen post-PR-#568 (pool windows are filtered from `list_windows`), so no rename UX needed for pool windows.
- **Keyboard chord mode**: while in rename, prevent global chord shortcuts (Ctrl+T, Ctrl+W, etc.) from firing. The `e.stopPropagation()` in §4.1 handles this for keys; verify no document-level capture-phase listener bypasses it.

## 6. Implementation phases

| Phase | Scope | LOC est. |
|---|---|---|
| 1 | Backend meta key contract — write/read `window:displayname` via existing UpdateObjectMeta. No schema change. | 0 (uses existing) |
| 2 | Frontend `openWindowLabelsAtom` shape change → `Array<{ label, windowId }>`. Add host RPC if needed. | ~30 |
| 3 | InstancePanel row state machine (focus / dblclick / rename / commit / cancel). | ~80 |
| 4 | Default-name fallback chain (user → workspace → index). | ~20 |
| 5 | Tests for: empty-string revert, 64-char truncation, blur-commit, Escape-cancel. | ~50 |

## 7. Out of scope

- Renaming workspaces from this panel (workspaces have their own UI).
- Custom icons / colors per window (workspace color already serves this).
- Drag-to-reorder rows (rows are sorted "main first, then alphabetical by label" today).
- Right-click context menu on rows (separate ask if needed).
- Renaming via keyboard shortcut from outside the panel (open panel + dblclick is the entry point).

## 8. Open questions

- Should the row's hover affordance change during rename mode (e.g. dim other rows to indicate focus is captured)? Default: no, keep hover behavior consistent.
- Should there be an explicit "rename" button on each row in addition to dblclick? Discoverability is a concern — dblclick is invisible to a first-time user. Consider adding a small pencil icon on hover, but that's an optional follow-up.
- Should rename announce the new name via accessibility live region? Defer to A11y review.

## 9. Acceptance tests

- [ ] Single click on a non-focused row → that window comes to the foreground; panel closes.
- [ ] Single click on already-focused row → no-op; panel closes.
- [ ] Double click on a row → input appears with current name selected; subsequent typing replaces.
- [ ] Type "My Window" + Enter → row displays "My Window".
- [ ] Restart the app → "My Window" still displays.
- [ ] Type "  " (spaces) + Enter → row reverts to previous name (no commit).
- [ ] Type 100 chars + Enter → row displays the first 64 chars.
- [ ] Press Escape during edit → row reverts to pre-edit name.
- [ ] Click outside the input → commits as if Enter was pressed.
- [ ] Tear off a tab from a renamed window → original window keeps its name; tear-off window gets the index-based default.
- [ ] Rename a window, close it (last visible) → app exits cleanly; on next launch the window's name was attached to that *windowId*, not the label, so a re-spawn under a different label correctly shows the index default (the previously-renamed window record may be GC'd by `close_window` — verify cleanup path).
