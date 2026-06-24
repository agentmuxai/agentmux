# Spec: Floating Panes Section in Instance Panel

**Date:** 2026-06-24
**Status:** Draft
**Owner:** Oozp

---

## 1. Context

Clicking the version chip (`v0.X.Y (N)`) in the bottom-right of the status bar
opens the **InstancePanel** popover. It currently shows two things: About metadata
and a list of open windows in this process. The window list lives in a section
titled "This process — N windows".

Floating panes (`"floating-{uuid}"` label prefix) are **not** ordinary windows.
They are:

- Chromeless, borderless subordinate windows — one torn-off block per pane
- **Shared across all windows** — they belong to the process, not to a parent window
- Bound to a specific main window for cascade behaviour (minimize/restore/destroy),
  but visually they float freely above all windows
- Currently **invisible in the Instance Panel** — `isInstanceLabel()` in
  `launcher-event/types.ts` only passes `"window-"` and `"main"` labels, so
  floating panes never enter `openWindowEntriesAtom` and never appear in the list

The user wants floating panes to appear in a dedicated second section below the
windows section, since they are a process-level concept rather than per-window.

---

## 2. Goals

1. Surface all open floating panes in the Instance Panel
2. Keep them visually distinct from regular windows — different section, different
   row style, read-only (no rename)
3. Show enough context to identify each pane — block content label or workspace fallback
4. Allow clicking a floating pane row to focus/raise that pane
5. Show a "no floating panes" empty state, not an absent section, so users learn the
   feature exists

Non-goals (defer):
- Renaming floating panes (they have no persistent display name today)
- Opacity slider for floating panes (already controlled by system compositor)
- Closing a floating pane from this panel
- Showing which main window a pane is cascade-bound to (useful future polish)

---

## 3. Current Architecture (what to build on)

### 3.1 Data path for windows

```
listWindowInstances() ──► reconcileKnownEntriesFromSnapshot()
                                  │
                                  ▼
                        openWindowEntriesAtom   ← launcher events (WindowOpened/Closed)
                                  │
                                  ▼
                          InstancePanel (rendered)
```

`WindowEntry = { label: string; windowId: string | null }`

`isInstanceLabel(label)` gate (launcher-event/types.ts:144):
```ts
if (label === "main") return true;
if (label.startsWith("browser-pane-")) return false;
return label.startsWith("window-");
```
Floating panes (`"floating-*"`) are excluded here.

### 3.2 Floating pane label scheme

- Label: `"floating-{uuid}"` (e.g. `floating-a1b2c3d4-ef56-...`)
- Created by: `agentmux-cef/src/commands/floating_pane.rs` → `OpenFloatingPaneResponse`
- Tracked in host: `ACTIVE_FLOATER_HWNDS` registry (Windows),
  frameless-CEF-Views lifecycle (macOS/Linux)
- `listWindowInstances()` already returns them — the label is in the full snapshot

### 3.3 Label for display

Floating panes have no user-set display name. Resolution priority:

1. Block's agent name or tool title (requires `workspace → tab → block` traversal via WOS)
2. Workspace name (`ws.name`)
3. Positional fallback: "Pane 1", "Pane 2", …

The WOS traversal is the same pattern used by `resolveName()` for windows, but one
level deeper since a floating pane has exactly one block.

---

## 4. Proposed UI

### 4.1 Visual layout (annotated mockup)

```
┌────────────────────────────────────────┐
│ Version      v0.49.1                   │  ← about header (unchanged)
│ Build        9dd2d78                   │
│ Runtime      windows · x86_64          │
├────────────────────────────────────────┤
│ THIS PROCESS — 2 WINDOWS               │  ← windows section (unchanged)
│                                        │
│  ● Window 1                  [this]    │
│  ○ Window 2                            │
│    Opacity ──────────────●── 85%       │
├────────────────────────────────────────┤  ← new divider
│ FLOATING PANES — 3                     │  ← new section header
│                                        │
│  ◈ Claude agent · Workspace A          │  ← pane row (click → focus)
│  ◈ Terminal · Workspace A              │
│  ◈ Editor · Workspace B                │
│                                        │
│  (or, when none open:)                 │
│  No floating panes                     │  ← empty state
├────────────────────────────────────────┤
│  [+ Open another window]  [Close]      │  ← footer (unchanged)
└────────────────────────────────────────┘
```

### 4.2 Section header

```
FLOATING PANES — N
```

Same style as the existing `instance-panel-section-title`: 11px, uppercase,
secondary text colour, letter-spacing 0.04em.

Show count inline. Hide section entirely only if the API call to enumerate panes
fails (graceful degradation); always show the section (including empty state)
otherwise.

### 4.3 Pane row

```css
.instance-panel-pane-row
```

Matches `.instance-panel-window-row` dimensions (padding, font size, line height)
but with these differences:

| Property         | Window row          | Pane row                       |
|------------------|---------------------|--------------------------------|
| Leading icon     | `●` / `○` (accent)  | `◈` (secondary text colour)    |
| Hover bg         | `--hover-bg-color`  | Same                           |
| "this" badge     | Present             | Absent                         |
| Rename on dblClick | Yes               | No — no-op on dblClick         |
| Opacity slider   | Below row           | Absent                         |
| Cursor           | `default`           | `default`                      |

The diamond icon (`◈` U+25C8) is visually distinct from the window dot without
being alarming. It reads as "floating" (four-point, not filled-circle anchored).

Label line format: `{blockLabel} · {workspaceName}` — both truncated with ellipsis
if the row overflows. Falls back to `Pane N` if neither can be resolved.

### 4.4 Empty state

When no floating panes are open:

```
FLOATING PANES — 0

  No floating panes
```

The "No floating panes" line uses 11px secondary text colour, padded the same as
a regular row. This teaches users the feature exists rather than silently hiding
the section.

### 4.5 Focus action

Click → `getApi().focusWindow(entry.label)`.

Same handler as `handleFocusWindow`. Floating pane labels are valid window labels;
the host already handles them in `focus_window` (agentmux-cef/src/commands/window.rs:324).

No deferred double-click concern: floating panes are read-only rows so no rename
mode to disambiguate.

---

## 5. Data Model

### 5.1 New atom

```ts
// frontend/app/store/global.ts

export type FloatingPaneEntry = {
    label: string;        // "floating-{uuid}"
    windowId: string | null;
};

export const [openFloatingPaneEntriesAtom, setOpenFloatingPaneEntriesAtom] =
    createSignal<FloatingPaneEntry[]>([]);
```

`FloatingPaneEntry` is structurally identical to `WindowEntry` — a separate type
communicates intent and allows each to evolve independently.

### 5.2 Label classifier addition

Add alongside `isInstanceLabel` in `frontend/app/store/launcher-event/types.ts`:

```ts
export function isFloatingPaneLabel(label: string): boolean {
    if (label.startsWith("floating-pool-")) return false;
    return label.startsWith("floating-");
}
```

### 5.3 Launcher event reducer

In `launcher-event-reducer.ts`, the `WindowOpened` and `WindowClosed` branches
currently funnel through `isInstanceLabel`. Extend to also route
`isFloatingPaneLabel` entries into `openFloatingPaneEntriesAtom`:

```ts
// WindowOpened handler (pseudocode):
if (isInstanceLabel(ev.label)) {
    // existing path → openWindowEntriesAtom
} else if (isFloatingPaneLabel(ev.label)) {
    setOpenFloatingPaneEntriesAtom(prev =>
        prev.some(e => e.label === ev.label)
            ? prev
            : [...prev, { label: ev.label, windowId: ev.windowId ?? null }]
    );
}

// WindowClosed handler (pseudocode):
if (isFloatingPaneLabel(ev.label)) {
    setOpenFloatingPaneEntriesAtom(prev =>
        prev.filter(e => e.label !== ev.label)
    );
}
```

### 5.4 Boot-time snapshot reconciliation

`reconcileKnownEntriesFromSnapshot` currently populates only `openWindowEntriesAtom`.
Extend it to also seed `openFloatingPaneEntriesAtom` from the same `listWindowInstances()`
snapshot:

```ts
const floaters = snapshot.filter(e => isFloatingPaneLabel(e.label));
setOpenFloatingPaneEntriesAtom(floaters);
```

The same dev-mode guard that protects windows from stale reconcile applies here —
both atoms are refreshed together or not at all.

### 5.5 Display name resolution

New helper (can live in `util/window-title.ts` alongside `resolveWindowName`):

```ts
interface FloatingPaneNameOpts {
    workspaceName?: string;
    blockLabel?: string;       // agent name, file name, terminal title, etc.
    indexInOpenPanes: number;
}

export function resolveFloatingPaneName(opts: FloatingPaneNameOpts): string {
    if (opts.blockLabel) return opts.blockLabel;
    if (opts.workspaceName) return opts.workspaceName;
    return `Pane ${opts.indexInOpenPanes + 1}`;
}
```

`blockLabel` derivation inside InstancePanel (for a given `FloatingPaneEntry`):

```ts
const resolveFloatingName = (entry: FloatingPaneEntry, idx: number): string => {
    let blockLabel: string | undefined;
    let workspaceName: string | undefined;

    const win = entry.windowId
        ? getObjectValue<WaveWindow>(makeORef("window", entry.windowId))
        : null;

    if (win?.workspaceid) {
        const ws = getObjectValue<Workspace>(makeORef("workspace", win.workspaceid));
        workspaceName = ws?.name;

        // Floating panes have exactly one active tab and one block.
        const tab = ws?.activetabid
            ? getObjectValue<Tab>(makeORef("tab", ws.activetabid))
            : null;
        const blockId = tab?.blockids?.[0];
        if (blockId) {
            const block = getObjectValue<Block>(makeORef("block", blockId));
            // Use block meta title or view type as label.
            blockLabel = (block?.meta?.title as string | undefined)
                ?? (block?.meta?.view as string | undefined);
        }
    }

    return resolveFloatingPaneName({ workspaceName, blockLabel, indexInOpenPanes: idx });
};
```

If `blockLabel` resolves to a view type string (e.g. `"agent"`, `"term"`, `"browser"`),
title-case it for display: `"Agent"`, `"Terminal"`, `"Browser"`.

---

## 6. Component Changes

### 6.1 InstancePanel.tsx

1. Import `openFloatingPaneEntriesAtom`, `FloatingPaneEntry` from global store
2. Import `resolveFloatingPaneName` from util
3. Add `resolveFloatingName()` local helper (see §5.5)
4. Add `handleFocusPane(label)` — identical to `handleFocusWindow`; split for
   readability
5. Below the existing windows `<div class="instance-panel-section">`, add:

```tsx
<div class="instance-panel-divider" />
<div class="instance-panel-section">
    <div class="instance-panel-section-title">
        Floating panes — {floatingEntries().length}
    </div>
    <Show
        when={floatingEntries().length > 0}
        fallback={
            <div class="instance-panel-pane-empty">No floating panes</div>
        }
    >
        <For each={floatingEntries()}>
            {(entry, i) => (
                <div
                    class="instance-panel-pane-row"
                    onClick={() => handleFocusPane(entry.label)}
                    title="Click to focus"
                    role="button"
                    tabIndex={0}
                    onKeyDown={(e) => {
                        if (e.key === "Enter" || e.key === " ") {
                            e.preventDefault();
                            handleFocusPane(entry.label);
                        }
                    }}
                >
                    <span class="instance-panel-pane-icon">◈</span>
                    <span class="instance-panel-pane-name">
                        {resolveFloatingName(entry, i())}
                    </span>
                </div>
            )}
        </For>
    </Show>
</div>
```

### 6.2 _instance-panel.scss

Add after the existing window-row rules:

```scss
// Floating pane rows — same geometry as .instance-panel-window-row but
// icon uses secondary colour and there is no badge / opacity slider.
.instance-panel-pane-row {
    display: flex;
    align-items: center;
    gap: var(--space-1-5);
    width: 100%;
    background: transparent;
    border: 1px solid transparent;
    border-radius: 0;
    padding: var(--space-1) var(--space-1-5);
    color: var(--main-text-color);
    font-size: 12px;
    text-align: left;
    cursor: default;
    line-height: 1.3;

    &:hover {
        background: var(--hover-bg-color);
        border-color: var(--border-color);
    }

    .instance-panel-pane-icon {
        flex: 0 0 auto;
        color: var(--secondary-text-color);
        font-size: 10px;
        width: 10px;
        text-align: center;
    }

    .instance-panel-pane-name {
        flex: 1 1 auto;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }
}

.instance-panel-pane-empty {
    font-size: 11px;
    color: var(--secondary-text-color);
    padding: var(--space-1) var(--space-1-5);
    font-style: italic;
}
```

---

## 7. Files Touched

| File | Change |
|------|--------|
| `frontend/app/store/global.ts` | Add `FloatingPaneEntry` type, `openFloatingPaneEntriesAtom` signal |
| `frontend/app/store/launcher-event/types.ts` | Add `isFloatingPaneLabel()` |
| `frontend/app/store/launcher-event-reducer.ts` | Route floating pane open/close events; seed from snapshot |
| `frontend/app/statusbar/InstancePanel.tsx` | Import new atom; add floating pane section JSX |
| `frontend/app/statusbar/_instance-panel.scss` | Add `.instance-panel-pane-row`, `.instance-panel-pane-empty` |
| `frontend/util/window-title.ts` | Add `resolveFloatingPaneName()` helper |

No Rust changes required. `listWindowInstances()` and `focusWindow()` already handle
floating pane labels correctly.

---

## 8. Edge Cases

| Scenario | Behaviour |
|----------|-----------|
| Pane closes while panel is open | Launcher event fires → atom updates → `<For>` rerenders → row disappears |
| New pane opens while panel is open | Launcher event fires → atom updates → row appended |
| `windowId` is null (pre-registerBackendWindow race) | `resolveFloatingName` returns `Pane N`; updates reactively when `windowId` arrives |
| No floating panes | Show "No floating panes" empty state; count shows `— 0` |
| `focusWindow` fails on a pane that just closed | Log error, ignore — same as existing window focus path |
| Dev mode (no launcher events) | `reconcileKnownEntriesFromSnapshot` seeds both atoms from the full `listWindowInstances()` snapshot |
| macOS/Linux (Phase A) | Floating pane labels follow the same `"floating-*"` scheme; no platform branching needed in the frontend |

---

## 9. Open Questions

1. **Block label view-type mapping** — should we define a canonical map
   (`"agent" → "Agent"`, `"term" → "Terminal"`, `"browser" → "Browser"`, …)
   in a shared util, or is inline title-casing sufficient for now?
   _Recommendation: inline `toTitleCase(view)` for V1; extract to shared util
   when other surfaces need it._

2. **Pane count in version chip** — the status bar chip currently shows
   `v0.X.Y (N)` where N = window count. Should floating panes be included in N,
   shown separately (`v0.X.Y (2w · 3p)`), or left alone?
   _Recommendation: leave alone for V1; the panel itself surfaces the count._

3. **Section order** — Windows above, Floating panes below feels natural since
   windows are primary. No objection expected, but confirm with the team.

---

## 10. Implementation Order

1. `global.ts` — atom + type _(isolated, no deps)_
2. `launcher-event/types.ts` — `isFloatingPaneLabel` _(isolated)_
3. `launcher-event-reducer.ts` — route events + seed reconcile _(depends on 1–2)_
4. `util/window-title.ts` — `resolveFloatingPaneName` _(isolated)_
5. `InstancePanel.tsx` — import atoms + JSX _(depends on 1, 4)_
6. `_instance-panel.scss` — styles _(parallel with 5)_
