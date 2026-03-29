# Spec: Sysinfo Body Context Menu — Metric Selection

**Date:** 2026-03-28
**Status:** Ready to implement

---

## Problem

Right-clicking the **body** of a sysinfo pane shows only the generic pane menu
(Split, Close, Magnify, Replace With…). There is no way to switch metrics from
a body right-click.

The **header** right-click already calls `viewModel?.getSettingsMenuItems?.()` and
injects a "Plot Type" submenu — but the body context menu in `blockframe.tsx`
(`onBodyContextMenu`, line 675) only calls `buildPaneContextMenu()` and does not
inject any view-specific items.

---

## Goal

Right-clicking a sysinfo pane body should show direct metric-selection radio items
at the **top level** of the menu — no submenu nesting — so the user can switch in
one click:

```
● CPU
○ Memory
○ CPU + Memory
○ Network
○ Network (Sent/Recv)
○ CPU + Memory + Network
○ Disk I/O
○ Disk I/O (R/W)
○ All CPU Cores
────────────────────
Split Right
Split Down
…
Close
```

---

## Current State

### Body context menu — `blockframe.tsx` line 674–685

```typescript
const onBodyContextMenu = (e: MouseEvent) => {
    if (!blockData() || props.preview) return;
    e.preventDefault();
    e.stopPropagation();
    const menu = buildPaneContextMenu(blockData(), {
        magnified: isMagnified(),
        onMagnifyToggle: nodeModel.toggleMagnify,
        onClose: nodeModel.onClose,
    }, props.viewModel);
    ContextMenuModel.showContextMenu(menu, e);   // ← no view-specific items
};
```

### Header context menu — `blockframe.tsx` line 58–60

```typescript
const extraItems = viewModel?.getSettingsMenuItems?.();
if (extraItems && extraItems.length > 0) menu.push({ type: "separator" }, ...extraItems);
```

### Existing `getSettingsMenuItems()` in `sysinfo-model.ts` lines 193–228

Returns `[{ label: "Plot Type", submenu: [...radio items] }, { type: "separator" }]`.
The nesting is acceptable for the header (less frequently used) but too many clicks
for the body (primary interaction surface).

---

## Design Decisions

### Decision 1: Body menu vs. header menu

The body menu is the **primary** interaction point (user right-clicks the chart
itself). Metric selection should be one click from there. The header "Plot Type"
submenu can stay as-is — it is hidden under the header icon and mostly used for
keyboard/accessibility paths.

### Decision 2: Flat items vs. submenu in body

Flat radio items at top level — no "Plot Type" parent item. Nine items is
acceptable; they map directly to the nine `PlotTypes` keys and the user sees
all options at once. A submenu would require an extra hover/click.

### Decision 3: Separate method vs. reusing `getSettingsMenuItems()`

Add a **new optional method** `getBodyContextMenuItems?(): ContextMenuItem[]` to
the `ViewModel` interface. This keeps the header and body menus independently
configurable. Other views (term, agent, etc.) get nothing from body right-click
unless they opt in.

Alternatively, modify `onBodyContextMenu` to call `getSettingsMenuItems()` and
unwrap any single top-level submenu into flat items — but that is fragile coupling
to the menu structure.

**Chosen: new `getBodyContextMenuItems()` method.**

### Decision 4: Separator placement

Put metric items **before** pane actions, separated by a line. This mirrors the
common pattern (app-specific actions first, destructive/structural actions last).

```
[metric radio items]
────────────────────
[pane actions: split, magnify, close…]
```

---

## Implementation Plan

### Step 1 — Add method to ViewModel interface

**File:** `frontend/types/custom.d.ts`

Add to the `ViewModel` interface:

```typescript
getBodyContextMenuItems?: () => ContextMenuItem[];
```

### Step 2 — Implement in `SysinfoViewModel`

**File:** `frontend/app/view/sysinfo/sysinfo-model.ts`

Add new method (alongside existing `getSettingsMenuItems`):

```typescript
getBodyContextMenuItems(): ContextMenuItem[] {
    const plotData = this.dataAtom();
    if (plotData.length === 0) return [];

    const currentlySelected = this.plotTypeSelectedAtom();
    return Object.keys(PlotTypes).map((plotType): ContextMenuItem => ({
        label: plotType,
        type: "radio",
        checked: currentlySelected === plotType,
        click: async () => {
            const dataTypes = PlotTypes[plotType](plotData[plotData.length - 1]);
            await RpcApi.SetMetaCommand(TabRpcClient, {
                oref: WOS.makeORef("block", this.blockId),
                meta: { "graph:metrics": dataTypes, "sysinfo:type": plotType },
            });
        },
    }));
}
```

### Step 3 — Wire into body context menu

**File:** `frontend/app/block/blockframe.tsx`

Modify `onBodyContextMenu` (line 675) to inject view-specific items before pane
actions:

```typescript
const onBodyContextMenu = (e: MouseEvent) => {
    if (!blockData() || props.preview) return;
    e.preventDefault();
    e.stopPropagation();

    const menu: ContextMenuItem[] = [];

    // View-specific body items (e.g. sysinfo metric selection)
    const bodyItems = props.viewModel?.getBodyContextMenuItems?.();
    if (bodyItems && bodyItems.length > 0) {
        menu.push(...bodyItems, { type: "separator" });
    }

    // Shared pane actions (split, magnify, replace, close)
    menu.push(...buildPaneContextMenu(blockData(), {
        magnified: isMagnified(),
        onMagnifyToggle: nodeModel.toggleMagnify,
        onClose: nodeModel.onClose,
    }, props.viewModel));

    ContextMenuModel.showContextMenu(menu, e);
};
```

Note: `buildPaneContextMenu` currently returns `ContextMenuItem[]` — confirm the
return type or spread accordingly. If it currently mutates and returns the array,
spreading it here is safe.

---

## Files to Change

| File | Change |
|------|--------|
| `frontend/types/custom.d.ts` | Add `getBodyContextMenuItems?: () => ContextMenuItem[]` to `ViewModel` |
| `frontend/app/view/sysinfo/sysinfo-model.ts` | Add `getBodyContextMenuItems()` method |
| `frontend/app/block/blockframe.tsx` | Modify `onBodyContextMenu` to inject body items |

**No backend changes needed** — metric selection writes to block meta via existing
`SetMetaCommand`, same as the header "Plot Type" submenu.

---

## Menu Item Labels

Taken directly from `PlotTypes` keys in `sysinfo-types.ts`:

| Label | Metrics shown |
|-------|---------------|
| CPU | `cpu` |
| Mem | `mem:used` |
| CPU + Mem | `cpu`, `mem:used` |
| Net | `net:bytestotal` |
| Net (Sent/Recv) | `net:bytessent`, `net:bytesrecv` |
| CPU + Mem + Net | `cpu`, `mem:used`, `net:bytestotal` |
| Disk I/O | `disk:total` |
| Disk I/O (R/W) | `disk:read`, `disk:write` |
| All CPU | `cpu:0`…`cpu:N` (dynamic) |

Labels are stable — they are also the `sysinfo:type` meta value stored on the block.

---

## Non-Goals

- No new metrics beyond what the backend already collects.
- No per-interface network breakdown (e.g. eth0 vs. wifi) — backend aggregates.
- No interval/numpoints adjustment from body menu — that stays in header settings.
- No changes to the header menu structure.

---

## Testing

1. Right-click sysinfo chart body → metric radio items appear at top, pane actions below separator
2. Click a metric → chart switches immediately (reactive via `sysinfo:type` meta)
3. Checked item reflects current selection
4. Right-click sysinfo **header** → unchanged (still shows "Plot Type" submenu)
5. Right-click a **terminal** pane body → no metric items (method not present on TermViewModel)
6. Right-click sysinfo when no data loaded yet → no metric items, only pane actions
