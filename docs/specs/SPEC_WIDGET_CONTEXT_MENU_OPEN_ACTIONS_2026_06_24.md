# SPEC: Widget Context Menu — "Open in New Window" + "Open in Floating Pane"

**Date:** 2026-06-24
**Status:** Draft
**Scope:** Frontend + Rust host — no backend (srv) schema changes
**Files touched:**
- `frontend/app/window/action-widgets.tsx` — `buildItemMenuItems`, menu item handlers
- `agentmux-cef/src/commands/window/creation.rs` — extend `open_new_window` to accept optional initial view
- `agentmux-cef/src/ipc.rs` — wire new parameter through IPC dispatch

---

## 1. Problem

Right-clicking a widget in the bar shows a context menu with three entries:

```
New Window          ← opens a blank default window (not the widget)
─────────────
Pin to bar / Unpin from bar
```

Two issues:
- The label "New Window" is ambiguous — it doesn't say what will open.
- The action opens a **blank** new window rather than the clicked widget in that window.
- There is no way to pop a widget into a floating pane from the bar.

---

## 2. Desired Menu

After this change the right-click context menu has exactly **three entries**:

```
Open in New Window
Open in Floating Pane
─────────────────────
Pin to bar / Unpin from bar
```

Both new entries open the **specific widget** that was right-clicked, not a generic default layout.

---

## 3. Behaviour

### 3.1 "Open in New Window"

Opens a fresh top-level OS window and immediately places the widget's view as its
first (and only) pane.

- The new window is a full AgentMux instance window (same as the current "New Window"
  path — uses the pool-first / cold-path machinery in `open_new_window`).
- The window opens with a **single pane** showing the widget's view type
  (e.g. right-clicking Terminal → new window contains a Terminal pane).
- If the widget has `magnified: true` in its config the new pane is opened magnified.

#### Implementation — host side

Extend `open_new_window` in `agentmux-cef/src/commands/window/creation.rs` to accept an
optional `initial_view: Option<String>` parameter. After the window is promoted from
the pool (or spun up cold), the host fires a `window:open-view` directive to that
window's frontend with the view name so the frontend can call `pane.open`.

```
// IPC call from frontend:
invokeCommand("open_new_window", { initial_view: "term" })
```

The host's pool-promote or cold-path `open_window_with_kind` already returns the
window label. After promotion, the host sends a one-shot `window:open-view` WPS event
scoped to that label's workspace so the new window's app-init listener can open the
requested pane on first render.

**Platform notes:**
- Windows / macOS / Linux: all use the same `open_new_window` → pool or cold path.
  The pool window on all platforms starts with the default workspace; the directive
  replaces its layout identically on all three platforms.
- The `H.7` mid-close gate (`any_browser_pane_closing`) applies to all platforms
  identically — no platform-specific branch needed.

#### Fallback

If `initial_view` is absent or unrecognised, the window opens with the default layout
(current behaviour). This preserves the existing "+" new-window trigger path.

### 3.2 "Open in Floating Pane"

Opens the widget's view as a floating pane in the **current window** using the
existing `pane.open` RPC with `floating: true`.

```ts
// frontend call:
await TabRpcClient.rpcCall("pane.open", {
    view: resolveWidgetView(widgetKey),   // e.g. "term" for defwidget@terminal
    floating: true,
});
```

This reuses the existing `open_pane_floating` code path in `agentmux-srv` — no new
backend work required.

**Platform notes:**
- Windows: `open_floating_pane_window` opens a chromeless OS window via the
  tear-off-block saga. Fully supported.
- macOS: same code path, chromeless window created via `open_floating_pane_window`.
  Fully supported.
- Linux: same path. Supported since the Linux floating-pane work.

**Container agents:** `pane.open` with `floating: true` opens a pane in the
current instance's floating workspace. Container agent panes behave identically to
host agent panes for this action — the floating pane gets a fresh block and the
container lifecycle is unaffected.

---

## 4. View Resolution

The widget key in `buildItemMenuItems` is the `shortName`
(e.g. `"terminal"` from `defwidget@terminal`), but `pane.open` and
`window:open-view` need the canonical view string (e.g. `"term"`).

Resolution: read `wmap()["defwidget@" + shortName]?.blockdef?.meta?.view` which is
already available in the `MoreDropdown` closure via the `wmap` accessor. Pass this
resolved view string (not the shortName) to both actions.

```ts
const resolveView = (shortName: string): string | null =>
    wmap()[`defwidget@${shortName}`]?.blockdef?.meta?.["view"] ?? null;
```

If the widget has no resolvable view (defensive: shouldn't happen for built-in widgets),
both new menu items are omitted for that widget.

---

## 5. Menu Structure Changes

### Current (`buildItemMenuItems`)

```ts
[
    { label: "New Window", click: () => getApi().openNewWindow() },
    { type: "separator" },
    { label: "Pin to bar" / "Unpin from bar", ... },
]
```

### After

```ts
[
    {
        label: "Open in New Window",
        click: () => {
            closeMore();
            const view = resolveView(shortName);
            if (view) fireAndForget(() => getApi().openNewWindowWithView(view));
            else fireAndForget(() => getApi().openNewWindow());
        },
    },
    {
        label: "Open in Floating Pane",
        click: () => {
            closeMore();
            const view = resolveView(shortName);
            if (!view) return;
            fireAndForget(() =>
                TabRpcClient.rpcCall("pane.open", { view, floating: true })
            );
        },
    },
    { type: "separator" },
    { label: "Pin to bar" / "Unpin from bar", ... },
]
```

`getApi().openNewWindowWithView(view)` is a new typed wrapper over
`invokeCommand("open_new_window", { initial_view: view })`.

---

## 6. Pinned Bar vs More Dropdown

The context menu is built by `buildItemMenuItems` and used in two call sites:
1. **Pinned bar** (`WidgetBar`): right-click on pinned widget button
2. **More dropdown** (`MoreDropdown`): right-click on item in the overflow list

Both call sites pass `shortName` + `wmap` and render the same menu. No divergence
needed — both get the two new items identically.

---

## 7. Not in Scope

- "Open in New Tab" (separate future feature — tabs are within a window, not a new UX
  action from the widget bar).
- Any change to the "Pin"/"Unpin" entry — it remains unchanged.
- Any change to the default click action (left-click still opens the widget docked in
  the current window).
- Opening a widget in another running instance's window (cross-instance pane.open is
  a separate larger feature).
