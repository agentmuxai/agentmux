# Spec: Sysinfo Pane Cleanup

**Date:** 2026-03-18
**Status:** Ready to implement

---

## Changes

### 1. Remove the cog (settings gear) button

The sysinfo pane currently shows a cog icon in the header that opens a settings menu with "Plot Type" submenu. Remove it — all sysinfo-specific options move to the right-click context menu.

**How:** Return empty array from `getSettingsMenuItems()` in `sysinfo-model.ts`. The block frame renders the cog only when this returns items.

### 2. Remove the remote/connection button

Already done — `manageConnection` set to `false`.

### 3. Simplify context menu to 3 views

Right-click on the sysinfo pane should show:

```
📊 CPU
💾 Memory
💿 Disk
───────────
(standard pane context menu items)
```

Just three views. Remove all the other plot types (Net, CPU + Mem, CPU + Mem + Net, All CPU, etc.). Users who want network or combined views can configure via settings.json.

### Current Plot Types (remove most)

| Type | Keep? | Reason |
|------|-------|--------|
| CPU | ✓ | Core metric |
| Mem | ✓ | Core metric |
| CPU + Mem | ✗ | Combined — power user, not needed in menu |
| Net | ✗ | Not requested |
| Net (Sent/Recv) | ✗ | Not requested |
| CPU + Mem + Net | ✗ | Not requested |
| Disk I/O | ✓ | Core metric |
| Disk I/O (R/W) | ✗ | Detailed — power user |
| All CPU | ✗ | Per-core — power user |

### Context Menu Structure

The sysinfo-specific items appear ABOVE the standard pane context menu (which includes close, split, magnify, etc.):

```
📊 CPU              (radio, checked if active)
💾 Memory           (radio, checked if active)
💿 Disk             (radio, checked if active)
───────────
(standard pane context: Split, Close, etc.)
```

## Implementation

### `sysinfo-model.ts`

```typescript
getSettingsMenuItems(): ContextMenuItem[] {
    // Return empty — no cog menu
    return [];
}
```

The right-click context menu is built by the block frame, which calls `getSettingsMenuItems()` and prepends them. So we need a different approach — override the context menu on the sysinfo view itself.

### Approach: Add items to pane context menu

The block frame's context menu (right-click on header) already calls `model.getSettingsMenuItems()` and prepends the result. So keep using `getSettingsMenuItems()` but return only the 3 core views:

```typescript
getSettingsMenuItems(): ContextMenuItem[] {
    const plotData = this.dataAtom();
    if (plotData.length === 0) return [];

    const currentType = this.plotTypeSelectedAtom();
    const coreTypes = ["CPU", "Mem", "Disk I/O"];

    return coreTypes.map((plotType) => ({
        label: plotType === "Mem" ? "💾 Memory" : plotType === "CPU" ? "📊 CPU" : "💿 Disk",
        type: "radio" as const,
        checked: currentType === plotType,
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

This removes the cog (no wrapping in "Plot Type" submenu — items are top-level) and shows only 3 options.

## Files

| File | Change |
|------|--------|
| `frontend/app/view/sysinfo/sysinfo-model.ts` | Simplify `getSettingsMenuItems()` to 3 core views, no submenu |
