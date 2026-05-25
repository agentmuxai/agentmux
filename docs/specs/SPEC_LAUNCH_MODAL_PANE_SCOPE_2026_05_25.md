# SPEC: Move launch modal from tab-scope to pane-scope lock

**Date:** 2026-05-25
**Author:** AgentA (Claude Opus 4.7)
**Builds on:** [`SPEC_UNIFIED_MODAL_SYSTEM_2026_05_21.md`](./SPEC_UNIFIED_MODAL_SYSTEM_2026_05_21.md), [`launch-modal-rearchitecture-2026-05-01.md`](./launch-modal-rearchitecture-2026-05-01.md)

---

## TL;DR

The agent launch modal (and the sibling install / create-from-template / authentication modals plumbed through `useTabModal`) currently uses `scope="tab"`. When it opens, the backdrop covers the whole tab — every other pane in the tile layout goes `inert`, can't be clicked, can't be scrolled. That's overreach: the modal is configuring **one agent pane**; the user shouldn't lose access to a browser pane / terminal / second agent pane in the same tab.

Switch it to `scope="pane"`. The infrastructure already exists (`PaneModalScope` context in `frontend/app/element/modal.tsx`; built per `SPEC_UNIFIED_MODAL_SYSTEM_2026_05_21.md` §12.2 but unused). The user-visible change: the modal backdrop is clipped to the agent pane's bounds, and `inert` only covers the pane's own content — every other pane in the tab stays interactive.

---

## 1. Current state

Stack as wired today:

1. `AgentPicker.tsx` (rendered inside an agent pane's content) calls `useTabModal()` and dispatches a `launch` request.
2. `TabModalLayer.tsx` (wrapping the entire `<TabContent>`'s tile layout) holds the request signal, dispatches on `kind`, renders the panel inside a `<Modal scope="tab">`.
3. The unified `<Modal>` from `frontend/app/element/modal.tsx` (`scope="tab"` branch) reads `TabModalScope` for the mount node — set by `TabModalLayer` to its own wrapper — and applies scope-relative `inert` + scroll lock to the tab's content root.

Net effect: backdrop spans the whole tab area; every pane in the tile layout (including non-agent panes — browser, terminal, sysinfo) goes `inert` for the lifetime of the modal.

## 2. Problem

The user is configuring **one agent pane**. The modal's authority shouldn't extend past that pane's bounds. Concrete pain:

- A multi-pane layout (agent pane + browser pane side-by-side) loses the browser entirely while the launch modal is open. The user can't switch tabs in the browser, scroll a doc, or copy a URL into the modal's "working directory" field.
- A two-agent tab (e.g., comparing two providers) loses the *other* agent pane completely.
- The backdrop visual implies the lock is broader than the user's intent. Reads as "I committed to the launch flow" rather than "I'm configuring this agent".

## 3. Target design

Switch to `scope="pane"`. The unified modal system already supports this:

- `PaneModalScope` context in `frontend/app/element/modal.tsx:96` — symmetric with `TabModalScope`.
- `<Modal scope="pane">` resolves the mount node from `useContext(PaneModalScope)`, applies inert to the pane root, backdrop is clipped to the pane via `position: absolute` inside the mount node.
- Modal stack (§6 of the unified spec) already accounts for pane-scoped lock regions — multiple pane modals in different panes coexist; ESC/backdrop "reachable topmost" calculation does the right thing.

**Visual:**

```
┌─────────────────────────────────┐
│ Tab bar              [- □ ✕]   │ ← stays live
├─────────────────────────────────┤
│ ┌──────────────┬──────────────┐ │
│ │ AGENT PANE   │  BROWSER     │ │
│ │ ┌──────────┐ │              │ │
│ │ │ Launch   │ │   (stays     │ │
│ │ │ modal    │ │    live —    │ │ ← was inert before, live now
│ │ │ (here)   │ │    scrollable│ │
│ │ └──────────┘ │    clickable)│ │
│ │ backdrop ⬛  │              │ │
│ └──────────────┴──────────────┘ │ ← only the left pane's content is inert
└─────────────────────────────────┘
```

## 4. Implementation

### 4.1 Wire `PaneModalScope` at the pane root

Every block / pane that hosts an agent view needs to render `<PaneModalScope.Provider value={mountAccessor}>` around its content, where `mountAccessor` is an `Accessor<HTMLElement | null>` resolving to the pane's root.

The natural mount point is the pane's outermost `position: relative` root. `Block.tsx` (or whichever component renders the pane content frame) is the place. The provider pattern mirrors `TabModalScope` in `TabModalLayer.tsx:78-79` — same shape, smaller scope.

Open question: should EVERY pane provide a `PaneModalScope`, or only agent panes? Recommend every pane — the cost is one context value per pane, and it future-proofs other pane types (browser-search-replace, terminal-find, etc.) that might want their own pane-scoped modals later. The agent pane is just the first caller.

### 4.2 Switch `AgentPicker`'s launch flow

`useTabModal()` is the wrong abstraction for this — it's a tab-wide signal. Two paths forward:

**Option A — direct `<Modal scope="pane">` in the agent view:**
The pane renders the `<Modal>` itself, gated on a local `bool` signal that `AgentPicker` flips. Cuts out `TabModalLayer` for the launch flow entirely. Cleaner separation, no shared signal infrastructure between panes.

**Option B — generalize the layer to a `PaneModalLayer`:**
Mirror the existing `TabModalLayer` but at pane scope. Keep the existing request-shape (`launch | install | createFromTemplate | upsertMemory | preLaunchAuth`) and dispatch table — just narrow the audience. Lets the existing call sites swap `useTabModal()` for `usePaneModal()` with minimal churn.

**Recommend Option A.** Reasons:
- No new layer abstraction (one fewer indirection to reason about).
- The launch flow is already mostly self-contained — `AgentLaunchModalPanel` already takes a callback-based API; doesn't need a global dispatch.
- `TabModalLayer`'s remaining surface (UpsertMemory etc.) can stay tab-scoped where appropriate, or migrate individually.
- The unified `<Modal>` is small; rendering it inline from the agent view costs ~10 lines.

Risk for Option A: the launch flow has a "+ New" → create-from-template → back-to-launch state machine that currently rides on `TabModalLayer`'s dispatch (see `frontend/app/store/launch-flow-state/`). Moving it under the agent view means the state machine moves too — bigger refactor than the cursor-fix-sized PR I want this to be. **Mitigation:** do a thin Option B first (introduce `PaneModalLayer`, swap the call site to `usePaneModal()`), keep the dispatch table identical; revisit Option A in a follow-up if the indirection feels heavy.

**Revised recommendation:** ship Option B in PR 1, defer Option A's flattening.

### 4.3 Backdrop + clip

The unified `<Modal>`'s scope-relative styling already handles this — backdrop is `position: absolute` inside the mount node when scope is `pane` or `tab`. No new CSS work.

Pane-overlay clip (`SPEC_MODAL_PANE_CLIP_2026_04_24`): the registration of the modal's rect with the backend to subtract from Win32 pane regions stays unchanged — the modal still wants its own paint area to composite cleanly with neighboring native HWNDs.

### 4.4 Pane lifecycle edges

| Event | Behavior |
|---|---|
| Pane resized while modal open | Backdrop + modal panel reposition with the pane (already handled by `position: absolute` in the mount node). |
| Pane closed (X clicked, layout split, etc.) while modal open | Modal unmounts with the pane. No "modal leak" — the entire mount subtree is gone. State (`launch-flow-state`) needs to clear if it's keyed to the pane; check that the slice's `Disposed` action is wired (cf. browser-pane-state-store's discipline). |
| User switches to another tab and back | Tab `display: none` is unaffected; modal stays open in the original pane. Returning to the tab shows it where they left it. |
| Two agent panes in the same tab, both with launch modals open | Independent — each pane's `PaneModalScope` resolves to a different mount node, the modal stack reasons about both as distinct lock regions (per unified spec §6). |

## 5. Migration & non-launch modals

This spec only covers the launch modal flow. The other surfaces currently using `TabModalLayer` (UpsertMemory, PreLaunchAuth dialogs, etc.) stay tab-scoped — they may legitimately want tab-wide authority, or they may not, but that's a separate evaluation. Keep `TabModalLayer` and `useTabModal()` around for them; the new `PaneModalLayer` / `usePaneModal()` lives alongside.

Out of scope:
- Window-scope review for non-modal "tooltip / popover / hover-anchor" surfaces — those have their own positioning model and aren't part of the unified modal system.
- Visual redesign of the modal panel (sizing, padding, chrome). Same panel; only the lock authority changes.
- Mobile / touch interaction. AgentMux is desktop-only today.

## 6. Test plan

| # | Scenario | Pass |
|---|---|---|
| 1 | Single-pane agent tab, click "+ New" or pick a definition | Backdrop covers only the agent pane; tab bar + header stay live |
| 2 | Two-pane tab (agent + browser), open launch modal | Browser pane stays clickable + scrollable; agent pane is inert behind backdrop |
| 3 | Two-agent tab, open launch modal in pane 1 | Pane 2's agent UI stays fully interactive |
| 4 | Two-agent tab, open launch modal in pane 1 AND pane 2 simultaneously | Both modals open independently; ESC closes the focused one, backdrop click on each closes that one |
| 5 | Resize the agent pane (drag divider) while modal open | Backdrop + panel reflow with the pane bounds |
| 6 | Close the agent pane (X) while modal open | Modal disappears; no orphan backdrop or stuck state |
| 7 | Switch tab while modal open, switch back | Modal still open in the same state |
| 8 | Multi-window (split into two AgentMux windows) | Each window's launch modal is independent (unchanged from today) |

Integration: `frontend/app/view/agent/components/AgentLaunchModal.integration.test.tsx` exists for the launch flow; extend with a regression that asserts the rendered `<Modal>` has `scope="pane"` and `PaneModalScope` resolves to a pane-local mount.

## 7. Delivery plan

Single PR, scoped tight:

1. Add `PaneModalLayer` mirroring `TabModalLayer` (request shape + dispatch identical for the launch family of requests; UpsertMemory etc. stay routed through the existing tab layer).
2. Add `<PaneModalScope.Provider>` at the pane root component (the `Block` / pane frame).
3. Switch `AgentPicker.tsx`'s `useTabModal()` to `usePaneModal()`.
4. Update `AgentLaunchModal.integration.test.tsx` to render under a `PaneModalLayer` + assert pane-scope mount.
5. Update the launch-modal-rearchitecture spec to point at this one as its successor for the scope axis.

Reagent + codex review. No portable smoke required — visual, no schema change.

## 8. Related

- [`SPEC_UNIFIED_MODAL_SYSTEM_2026_05_21.md`](./SPEC_UNIFIED_MODAL_SYSTEM_2026_05_21.md) — the `scope="pane"` capability comes from here (§3, §6, §12.2)
- [`launch-modal-rearchitecture-2026-05-01.md`](./launch-modal-rearchitecture-2026-05-01.md) — the previous launch-modal redesign; this spec narrows its scope
- [`SPEC_MODAL_PANE_CLIP_2026_04_24.md`](./SPEC_MODAL_PANE_CLIP_2026_04_24.md) — Win32 HWND clip, unchanged
- [`frontend/app/element/modal.tsx`](../../frontend/app/element/modal.tsx) — `PaneModalScope` context (already built, awaiting first caller)
- [`frontend/app/tab/TabModalLayer.tsx`](../../frontend/app/tab/TabModalLayer.tsx) — pattern to mirror at pane scope
