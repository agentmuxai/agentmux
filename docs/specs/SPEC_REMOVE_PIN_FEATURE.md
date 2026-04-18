# SPEC: Remove Tab Pinning, Uniform Inter-Tab Separator

**Status:** Spec — not implemented.
**Date:** 2026-04-18
**Owner:** AgentA

---

## 1. Goal

Two coupled changes to the tab bar:

1. **Remove tab pinning entirely.** Pin/unpin affordance in the context menu, the separate `.pinned-tab-spacer` divider, all `pinnedtabids` handling in drag-and-drop, workspace model, command palette, and keybindings.
2. **Apply the visual weight of the old pinned/regular divider (the short vertical bar) to every gap between tabs.** Replaces the existing thin `.tab::after` separator so every tab boundary has the same clear "pinned-group-separator" look.

After the change, pinning is no longer a concept. Every tab is a regular tab; every adjacent pair of tabs is separated by the same vertical bar. This simplifies the data model, drag-drop logic, and mental model, and matches how Edge / VS Code Explorer tabs render.

---

## 2. Current state

### Data model

- Workspace type in `agentmux-srv/src/backend/obj.rs:358` has `pub pinnedtabids: Vec<String>`. `tabids` holds the regular tabs. The effective order is `[...pinnedtabids, ...tabids]` (pinned first).
- `UpdateTabIds(workspaceId, tabIds, pinnedTabIds)` RPC at `frontend/app/store/services.ts:177` — frontend writes both lists.
- Backend DnD (`agentmux-srv/src/backend/wcore/dnd.rs`) treats the two lists separately when reordering, cross-window tear-off, and validation.

### UI

- `tabbar.tsx:147-169` renders pinned tabs via `<For each={pinnedTabIds()}>`, then a single `{pinnedTabIds().length > 0 && <div class="pinned-tab-spacer" />}` divider, then regular tabs.
- `.pinned-tab-spacer` (`tabbar.scss:42-49`): 1 px × 18 px bar, `var(--border-color)`, 3 px horizontal margin, bottom-aligned.
- `tab.tsx:97` context menu item toggles pin (`📌 Pin` / `📌 Unpin`).
- `tab.tsx:226` special-cases "active pinned tab on trigger > 0" for some focus behaviour.
- `tab.tsx:287` `<Show when={props.isPinned}>` renders a different close-button treatment for pinned tabs.
- Existing per-tab separator: `.tab::after` at `tab.scss:19-26` — 1 px × 14 px thinner bar, drawn on the tab's left; disabled on first child and when the tab is active (`&:first-child::after { content: none }` and the active-tab block at `tab.scss:65-68`).

### Other call sites touching pinned

| File | Lines | Purpose |
|---|---|---|
| `frontend/app/store/command-registry.ts` | 81 | Pin-aware tab list for commands |
| `frontend/app/store/global.ts` | 74, 830 | `activetabid` fallback + order construction |
| `frontend/app/store/keymodel.ts` | 158, 242 | `isPinned()` check for some shortcuts + ordered id list |
| `frontend/app/tab/droppable-tab.tsx` | 31-127 | Pin flag + DnD payload |
| `frontend/app/tab/tabbar.tsx` | 38-186 | Render, reorder, auto-colour-first-pin |
| `agentmux-srv/src/backend/wcore/dnd.rs` | 61, 118, 130, 158, 166, 171, 267, 296, 304, 309 | Pin-aware DnD accounting |
| `agentmux-srv/src/backend/obj.rs` | 358, 557, 566 | Schema + tests |
| `agentmux-srv/src/backend/wcore/mod.rs` | 190 | Tests |

---

## 3. Changes

### 3.1 Frontend — remove pin

- **`tabbar.tsx`**
  - Delete `pinnedTabIds()` / `allTabIds()` helpers. Use `ws.tabids` directly (plus a one-time migration read — see §4).
  - Delete the `togglePinTab` callback.
  - Collapse the two `<For each={pinnedTabIds()}>` + `<For each={regularTabIds()}>` blocks into a single `<For each={ws.tabids}>`.
  - Remove the `{pinnedTabIds().length > 0 && <div class="pinned-tab-spacer" />}` line.
  - Remove the `isBeforeActive`, `isFirst`, `tabIndex`, `pinnedTabIds` props wherever they were pin-derived; compute them from `ws.tabids` alone.
- **`tab.tsx`** — delete the `isPinned: boolean` prop, the `📌 Pin`/`📌 Unpin` context-menu item, the `props.active && props.isPinned` branch at line 226, and the `<Show when={props.isPinned}>` block at line 287. Drop the `isPinned` field from any payload.
- **`droppable-tab.tsx`** — delete `isPinned` prop and drag-payload field.
- **`keymodel.ts`** — delete `isPinned()` helper (line 158). Ordered id list (line 242) becomes `[...(ws.tabids ?? [])]`.
- **`command-registry.ts`** — line 81 becomes `[...(ws.tabids ?? [])]`.
- **`global.ts`** — line 74 fallback simplifies to `ws.activetabid || ws.tabids?.[0] || tabId`. Line 830 cross-workspace id list drops the concat.
- **Command palette** — remove "Pin Tab" / "Unpin Tab" entries if present in the command registry.

### 3.2 Frontend — uniform inter-tab separator

Replace the current `.tab::after` rule (`tab.scss:19-26`) with the visual of `.pinned-tab-spacer` applied to every tab boundary.

```scss
// tab.scss
.tab {
    &::after {
        content: "";
        position: absolute;
        // Place the bar on the LEFT edge of each tab so adjacent tabs share
        // a single visual separator. First tab hides its own via :first-child.
        left: 0;
        bottom: 4px;
        width: 1px;
        height: 18px;                 // was 14
        background: var(--border-color);
        pointer-events: none;
    }

    &:first-child::after {
        content: none;
    }

    &.active {
        // Drop the separator on the active tab AND on the next-sibling tab,
        // so the active tab reads as a connected block.
        & + .tab::after,
        &::after {
            content: none;
        }
    }
}
```

Delete `.pinned-tab-spacer` from `tabbar.scss:42-49` and the `<div class="pinned-tab-spacer" />` from `tabbar.tsx`.

The 18 px height + `var(--border-color)` match the weight of the old pinned-group divider so the new look is "that bar, everywhere".

### 3.3 Backend

Do **not** break the wire format immediately — other AgentMux instances may still send `pinnedtabids`. Strategy:

1. **Keep the field in `obj.rs::Workspace`** with `#[serde(default)]` so missing is OK.
2. **On every `Workspace` read** (the single bottleneck is whichever RPC returns it — typically `GetWorkspace` in `wcore/mod.rs`), drain any non-empty `pinnedtabids` into the head of `tabids` and clear `pinnedtabids`. This is a lazy migration — first load drops pinning permanently.
3. **`UpdateTabIds`** drops its `pinned_tab_ids` parameter or ignores it. The frontend call site (`services.ts:177`) shrinks to `UpdateTabIds(workspaceId, tabIds)`.
4. **DnD code (`wcore/dnd.rs`)** removes the dual-list accounting — everything becomes `tabids`. The `len() + len()` totals collapse, the `retain` calls drop the `pinnedtabids` branches, and the first-fallback selector `.or(source_ws.pinnedtabids.first())` becomes unnecessary.
5. **Tests (`obj.rs:557`, `wcore/mod.rs:190`)** — delete pin-specific fixtures or rewrite to assert drain-on-read behaviour.

### 3.4 Data / settings

- If any `settings.json` or `defaultconfig` key references a pin keybinding / command, delete it.
- `widgets.json` — not relevant (pin is a tab feature, not a widget).

---

## 4. Migration strategy

Users with pinned tabs must not lose them. The backend drain (§3.3 step 2) handles the workspace-state case. There's one edge:

- **In-flight drag of a pinned tab during upgrade** — impossible in practice (state changes on load, before DnD can fire), but worth asserting via a test.

No separate migration script is required — the lazy drain is sufficient because AgentMux re-reads workspace state on every session.

---

## 5. Tests

- Unit: `obj.rs` test for `Workspace::from_json` now asserts that a JSON blob containing a legacy `pinnedtabids` is drained into `tabids` on deserialization (or on the first touching mutation — pick one path).
- Unit: `wcore/mod.rs` drops `pinnedtabids.len() == 1` assertions; replaces with `tabids` coverage.
- Frontend Vitest: `tabbar.test.tsx` (create if missing) asserts the rendered tab count equals `ws.tabids.length` and there is no `.pinned-tab-spacer` element.
- Visual: snapshot or manual — 3+ tabs render with a 1 × 18 px separator on every non-first-non-active boundary, matching the former pinned divider weight.

---

## 6. Out of scope / follow-ups

- Drop-zone animation when re-ordering already uses `.tile-drop-hover`; no change needed after this spec.
- If we later want tab colours, pinning, or another grouping affordance back, reintroduce as a new concept rather than reviving `pinnedtabids`.
- Theme tuning of the separator colour (`var(--border-color)` is fine for now; revisit if contrast looks off against a coloured tab's background).

---

## 7. File-change checklist

Frontend:

- [ ] `frontend/app/tab/tabbar.tsx` — remove pin render, togglePin, two-pass For, divider.
- [ ] `frontend/app/tab/tab.tsx` — remove isPinned prop, pin context-menu item, pinned branches.
- [ ] `frontend/app/tab/droppable-tab.tsx` — remove isPinned prop/payload.
- [ ] `frontend/app/tab/tab.scss` — new inter-tab separator using old pinned-spacer look.
- [ ] `frontend/app/tab/tabbar.scss` — delete `.pinned-tab-spacer`.
- [ ] `frontend/app/store/global.ts` — drop pinned in active/order fallbacks.
- [ ] `frontend/app/store/command-registry.ts` — drop pinned concat.
- [ ] `frontend/app/store/keymodel.ts` — delete `isPinned()`, drop pinned concat.
- [ ] `frontend/app/store/services.ts` — shrink `UpdateTabIds` signature.

Backend:

- [ ] `agentmux-srv/src/backend/obj.rs` — keep field with `#[serde(default)]`, add drain helper.
- [ ] `agentmux-srv/src/backend/wcore/mod.rs` — call drain on the workspace read path, update test.
- [ ] `agentmux-srv/src/backend/wcore/dnd.rs` — collapse dual-list accounting, update cross-window tear-off.
- [ ] Ancillary tests.

Nothing in this spec is load-bearing enough to need a PR-series split — can ship in one PR with a clear title like `refactor(tab): remove pin feature, uniform inter-tab separator`.
