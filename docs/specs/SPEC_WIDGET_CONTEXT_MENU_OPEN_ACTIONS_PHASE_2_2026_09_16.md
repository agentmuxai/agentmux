# SPEC: Widget Context Menu — Pinned-Bar Parity + "Open in New Tab" (Phase 2)

**Date:** 2026-09-16
**Status:** Draft
**Companion:** `SPEC_WIDGET_CONTEXT_MENU_OPEN_ACTIONS_2026_06_24.md` (Phase 1 — this doc
picks up exactly what Phase 1 left incomplete or explicitly deferred, see §1)
**Scope:** Frontend only — no backend (srv) or host (agentmux-cef) changes. Every
primitive this spec needs already exists and is already cross-platform (see §7).
**Files touched:**
- `frontend/app/window/action-widgets.tsx` — `handlePinnedContextMenu`,
  `buildItemMenuItems`; both replaced by a single shared builder
- `frontend/app/window/action-widgets-menu.ts` *(new)* — the shared
  `buildWidgetOpenActions()` helper, consumed by both menu-rendering paths
- `frontend/app/tab/tab-presets.ts` — export a reusable
  `openViewInNewTab(view, meta)` helper (generalizes the pattern already used by
  `EditorViewModel.openInNewTab`)

---

## 1. Problem

Phase 1 (`SPEC_WIDGET_CONTEXT_MENU_OPEN_ACTIONS_2026_06_24.md`) added "Open in New
Window" and "Open in Floating Pane" to the widget right-click menu — but only landed
correctly in **one** of the two places that menu is built.

**Confirmed in the shipped code, not just theory:** `action-widgets.tsx` has two
separate, divergent menu builders for what is conceptually one action:

- `buildItemMenuItems` (More-dropdown / pinned-parent-flyout right-click) — correctly
  resolves the widget's `view`/`meta` and has both Phase 1 items.
- `handlePinnedContextMenu` (right-click directly on a **pinned bar icon** — the
  primary, most-used surface) — still shows a single generic **"New Window"** item
  wired to `getApi().openNewWindow()` with no view/meta at all. This opens a blank
  default-layout window, indistinguishable from right-clicking empty title-bar space
  and picking "New Window" there — even though `handlePinnedContextMenu` already has
  the clicked widget's definition in scope and simply doesn't use it.

Phase 1 §6 ("Pinned Bar vs More Dropdown") stated *"Both call sites pass shortName +
wmap and render the same menu. No divergence needed — both get the two new items
identically."* That claim did not hold in the shipped code — this spec's first job is
closing that gap.

Separately, Phase 1 §7 ("Not in Scope") explicitly deferred a third action:

> "Open in New Tab" (separate future feature — tabs are within a window, not a new
> UX action from the widget bar).

That feature is now requested. This spec adds it to both call sites at the same time
as fixing the pinned-bar parity gap, so the two menus cannot diverge again by
construction (see §4).

---

## 2. Desired Menu

Both call sites converge on the same three actions, in the same order, for any leaf
widget (agent, browser, terminal, editor, sysinfo, etc. — see the widget table in
`CLAUDE.md` / `agentmux-srv/src/config/widgets.json`):

```
Open in New Window
Open in Floating Pane
Open in New Tab
─────────────────────
Pin to bar / Unpin from bar
```

All three actions open the **specific widget that was right-clicked** — never a
blank/default layout. Parent/group widgets (e.g. a "Messengers" group) are unaffected
by this spec: they keep their existing, separate group-scoped menu (currently just
"Unpin group from bar") — there is no single pane a group represents, so none of the
three Open-in-* actions apply to them. This exclusion is pre-existing (Phase 1 made
the same call) and is not being revisited here.

---

## 3. Behaviour

### 3.1 "Open in New Window" — no new work, reuse as-is

Already fully implemented by Phase 1 and already used correctly by
`buildItemMenuItems`: `getApi().openNewWindowWithView(view, blockMeta)` →
`invokeCommand("open_new_window", { initial_view, initial_meta })` → the host's
pool-promote or cold-path `open_window_with_kind` threads both through so the new
window's frontend calls `pane.open` with the full resolved meta on first render. This
spec's only change here is making `handlePinnedContextMenu` call the same thing
instead of the bare `getApi().openNewWindow()` it calls today.

### 3.2 "Open in Floating Pane" — no new work, reuse as-is

Also already fully implemented by Phase 1 and already used correctly by
`buildItemMenuItems`:

```ts
TabRpcClient.rpcCall("pane.open", { view, meta: blockMeta, floating: true }, {});
```

Note on naming: "Floating Pane" is this app's established term for a pane torn into
its own **chromeless** OS-level window via the existing `tear_off_block` saga
(`open_pane_floating` in `agentmux-srv/src/server/app_api/pane.rs`) — distinct from
"New Window," which opens a full-chrome AgentMux window. The label is "Open in
Floating Pane," not "Open in Floating Window" — the underlying window is chromeless
implementation detail; the user-facing concept is a floating pane, matching the term
used everywhere else in the app (float/redock, `SPEC_FLOATING_PANE_*` doc cluster).
This spec's only change here is making `handlePinnedContextMenu` call the same RPC
instead of having no floating-pane option at all.

### 3.3 "Open in New Tab" — new

Opens a **brand new tab** in the current window, containing only the clicked widget's
view as its sole pane, and switches to it.

**Must not** be implemented as `pane.open` against a freshly created `tab_id`. This is
a documented, confirmed footgun (`frontend/app/tab/tab-presets.ts` and
`frontend/app/view/editor/editor-model.ts`, both around the `openInNewTab`-style call
sites): the RPC succeeds server-side (block created, layout updated, no error) but
never renders client-side, because the new tab's `layoutModel` is not yet subscribed
to the backend's `layout:update` broadcast at the moment `pane.open` would be called.

Instead, follow the same client-side sequence `EditorViewModel.openInNewTab` already
uses successfully for exactly this "new tab, one pre-seeded pane" case, generalized
into a reusable helper so this isn't a third copy of the same logic:

```ts
// frontend/app/tab/tab-presets.ts
export async function openViewInNewTab(view: string, meta?: Record<string, unknown>): Promise<void> {
    const ws = workspace();
    if (!ws) return;
    const tabId = await WorkspaceService.CreateTab(ws.oid, "", true, false);
    const layoutModel = await waitForLayoutModel(tabId);
    if (!layoutModel) return;
    await createBlockOnModel(tabId, layoutModel, { meta: { view, ...meta } }, null, null);
    await setActiveTab(tabId);
}
```

`EditorViewModel.openInNewTab` can be left as-is (it already works) or refactored to
call this shared helper — refactoring it is not required for this feature to ship,
but is the natural follow-up cleanup since it would otherwise be a third
near-identical implementation of the same sequence.

**Platform notes:** `WorkspaceService.CreateTab`, `waitForLayoutModel`, and
`createBlockOnModel` are pure frontend/srv-RPC calls with no OS branching anywhere in
their call chain — confirmed no `target_os`/platform conditionals in
`tab-presets.ts` or the services layer. Identical behavior on Windows, macOS, and
Linux by construction.

---

## 4. Shared Builder — fixing the root cause, not just the symptom

Phase 1's actual failure mode was duplicating the same menu-item list into two
functions and trusting them to stay in sync ("no divergence needed" — they diverged
immediately). This spec removes that duplication instead of adding a third copy.

New helper, `frontend/app/window/action-widgets-menu.ts`:

```ts
export interface WidgetOpenAction {
    label: string;
    run: () => void;
}

export function buildWidgetOpenActions(shortName: string, wmap: () => WidgetMap): WidgetOpenAction[] {
    const widgetDef = wmap()[`defwidget@${shortName}`];
    const blockMeta = widgetDef?.blockdef?.meta as Record<string, unknown> | undefined;
    const view = (blockMeta?.["view"] as string) ?? null;
    if (!view) return [];
    return [
        {
            label: "Open in New Window",
            run: () => fireAndForget(async () => getApi().openNewWindowWithView(view, blockMeta)),
        },
        {
            label: "Open in Floating Pane",
            run: () => fireAndForget(async () =>
                TabRpcClient.rpcCall("pane.open", { view, meta: blockMeta, floating: true }, {})
            ),
        },
        {
            label: "Open in New Tab",
            run: () => fireAndForget(async () => openViewInNewTab(view, blockMeta)),
        },
    ];
}
```

Both call sites map this same array into their own menu-item shape and append their
own separator + Pin/Unpin entry, rather than each hand-building the three actions:

- `buildItemMenuItems` maps `WidgetOpenAction[]` → `PopoverMenuItem[]` (`{ label, click: run }`) for the DOM `PopoverMenu` path (More dropdown / pinned-parent flyout).
- `handlePinnedContextMenu` maps the same array → `ContextMenuItem[]` (`{ label, click: run }`) for the native `ContextMenuModel.showContextMenu` path (direct pinned-bar-icon right-click).

Both `PopoverMenuItem` (`popover-menu.tsx`) and `ContextMenuItem`
(`frontend/types/custom.d.ts`) already use the same `{ label; click }` / `{ type:
"separator" }` shape, so the mapping is a one-line `.map()` in each call site — there
is no remaining place for the two menus to silently diverge again, since both now
read from the same array.

---

## 5. Menu Structure Changes

### `handlePinnedContextMenu` — current

```ts
ContextMenuModel.showContextMenu(
    [
        { label: "New Window", click: () => fireAndForget(async () => getApi().openNewWindow()) },
        { type: "separator" },
        { label: "Unpin from bar", click: () => unpinWidget(shortName, settings(), wmap()) },
    ],
    e
);
```

### `handlePinnedContextMenu` — after

```ts
const actions = buildWidgetOpenActions(shortName, wmap);
ContextMenuModel.showContextMenu(
    [
        ...actions.map((a) => ({ label: a.label, click: a.run })),
        { type: "separator" },
        { label: "Unpin from bar", click: () => unpinWidget(shortName, settings(), wmap()) },
    ],
    e
);
```

### `buildItemMenuItems` — current

```ts
const items: PopoverMenuItem[] = [
    { label: "Open in New Window", click: () => { ... } },
    { label: "Open in Floating Pane", click: () => { ... } },
];
items.push({ type: "separator" });
items.push(/* Pin/Unpin */);
```

### `buildItemMenuItems` — after

```ts
const actions = buildWidgetOpenActions(shortName, wmap);
const items: PopoverMenuItem[] = actions.map((a) => ({
    label: a.label,
    click: () => { closeMore(); a.run(); },
}));
items.push({ type: "separator" });
items.push(/* Pin/Unpin — unchanged */);
```

---

## 6. Call Sites

All three call sites that reach either menu builder get the fix/addition
identically, since both builders now consume `buildWidgetOpenActions`:

1. **Pinned bar** — right-click directly on a pinned `ActionWidget` icon
   (`handlePinnedContextMenu`, native OS menu). This is the primary gap this spec
   closes.
2. **More dropdown** — right-click a widget row inside the overflow "More" list
   (`buildItemMenuItems` via `MoreDropdown`'s `onItemContextMenu`). Already had
   items 1–2; gains item 3.
3. **Pinned-parent flyout** — right-click a child widget inside a pinned group's
   flyout, e.g. Messengers → Discord (`buildItemMenuItems` via
   `PinnedWidgetFlyout`'s `onItemContextMenu`). Same as #2.

Parent/group widgets themselves (right-clicking the group icon, not a child inside
its flyout) are out of scope — see §2.

---

## 7. Cross-Platform

No platform-conditional code is required anywhere in this feature. Every primitive
it depends on is already fully cross-platform:

- **`open_new_window` view/meta plumbing** (`agentmux-cef/src/commands/window/creation.rs`):
  has Windows-only `#[cfg(target_os = "windows")]` blocks, but all of them are scoped
  to window positioning/DPI physical-pixel math for the pool path — the
  `initial_view`/`initial_meta` threading itself has no OS branch and runs
  identically on Windows, macOS, and Linux.
- **`pane.open { floating: true }`** (`agentmux-srv/src/server/app_api/pane.rs`,
  `open_pane_floating`): zero `target_os` conditionals. The floating-pane window's
  own materialization (`open_floating_pane_window`) already has three confirming
  specs (`SPEC_FLOATING_PANE_TEAROFF_CROSS_PLATFORM_2026-05-26.md`,
  `SPEC_MACOS_FLOATING_PANE_TEAROFF_2026_05_29.md`,
  `SPEC_LINUX_FLOATING_PANE_TEAROFF_2026_05_30.md`) stating it is fully supported on
  all three platforms.
- **`WorkspaceService.CreateTab` / `createBlockOnModel` / `setActiveTab`**
  (`frontend/app/tab/tab-presets.ts`, `frontend/app/store/services.ts`): pure
  frontend + srv-RPC calls, no platform conditionals anywhere in the chain.

`CLAUDE.md`'s launcher-level Windows multi-instance isolation invariants (I1–I6 —
Job Object, named-pipe, process handles) are not implicated: this feature only opens
a new window/pane/tab **within the already-running instance** via the existing
pool/cold-path `open_new_window` machinery, a different layer entirely from launcher
process/pipe lifecycle.

---

## 8. Not in Scope

- Opening a widget in another running instance's window (cross-instance `pane.open`
  — a separate, larger feature; Phase 1 excluded this too).
- Any change to the "Pin to bar" / "Unpin from bar" entry — unchanged.
- Any change to the default left-click action on a widget (still opens docked in the
  current window/tab).
- Parent/group widget right-click menus (still their own separate, group-scoped
  menu — see §2).
- Refactoring `EditorViewModel.openInNewTab` to call the new shared
  `openViewInNewTab` helper — recommended follow-up cleanup, not required for this
  feature.
