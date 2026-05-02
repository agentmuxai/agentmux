# Modal Cleanup — Migration Audit & Plan

**Date:** 2026-05-01
**Status:** Audit / Plan
**Repo state:** main @ `257bf0ff`, AgentMux v0.33.549
**Author:** AgentC
**Companion to:** `launch-modal-rearchitecture-2026-05-01.md`

---

## Goal

After landing `TabModalLayer` for the launch modal, walk every dialog/overlay in the app and decide whether it stays global, migrates to tab-scope, or stays as a pane-inline panel. This is the audit needed before deciding what else (if anything) gets migrated.

---

## Method

- Grepped all consumers of `Modal` from `@/element/modal-v2`.
- Grepped `Portal` from `solid-js/web` for ad-hoc overlays.
- Walked the legacy `ModalsRenderer` registry (`workspace.tsx:46`, `modals/modalregistry.tsx`).
- Searched filenames: `*-modal*`, `*-dialog*`, `*-overlay*`, `*-panel*`, `*-popover*`.
- Cross-checked every result against the four classifications.

---

## Classifications

- **(A)** Migrate to `TabModalLayer` — conceptually tab/pane-scoped; should follow the active tab; should not block the tab bar.
- **(B)** Stay global Portal — window-level (command palette, about, backend-driven prompts).
- **(C)** Already pane-scoped, leave alone — inline overlays inside their owning pane (e.g., `AgentFocusedPanel`).
- **(D)** Re-classify — currently mis-scoped; needs deliberate rework.
- **Out of scope** — popovers, tooltips, context menus, toasts, drag overlays.

---

## (A) Migrate to TabModalLayer

| Modal | File | Trigger | Current scoping | State location | Complexity |
|---|---|---|---|---|---|
| **AgentLaunchModal** | `frontend/app/view/agent/components/AgentLaunchModal.tsx:38` | Click agent card in picker | Portal-to-body via Modal v2 | `launchModalAgent` signal in `AgentPicker.tsx:70` | Small |
| **ImportPreviewModal** | `frontend/app/view/agent/components/ImportPreviewModal.tsx:15` | Drag/paste forge agents into pane | Portal-to-body via Modal v2 | Local signals in `AgentActionBar` | Small |
| **AgentPicker delete confirm** | `frontend/app/view/agent/components/AgentPicker.tsx:214-235` | Delete forge agent definition | Portal-to-body via Modal v2 | `deleteCandidate`, `deleteError` in `AgentPicker` | Small |
| **AgentView close confirm** | `frontend/app/view/agent/agent-view.tsx:~360` | Close pane with running processes | Portal-to-body via Modal v2 | `closeConfirm` signal in `AgentPresentationView` | Small–Medium |

**Why these.** Each is initiated by an action inside one pane and is only relevant to that pane. Today they cover the whole window via Portal-to-body, which means they incorrectly block the tab bar and follow the user across tabs. After migration, each renders inside its tab's `TabModalLayer` and disappears with the tab.

**Total estimated work:** ~1 engineer-day, after `TabModalLayer` infrastructure is in place.

---

## (B) Stay Global Portal

| Modal | File | Why global |
|---|---|---|
| **CommandPaletteModal** | `frontend/app/modals/command-palette.tsx:28` | App-wide hotkey; must overlay everything. |
| **AboutModal** | `frontend/app/modals/about.tsx:13` | App-level metadata; not tab-scoped. |
| **UserInputModal** | `frontend/app/modals/userinputmodal.tsx:14` | Backend RPC initiates; can pertain to any tab; safer to be window-level. |
| **MessageModal** | `frontend/app/modals/messagemodal.tsx:10` | Generic backend message; window-level scope is correct. |
| **TokenBreakdownPopover (+ nested reset confirm)** | `frontend/app/statusbar/TokenBreakdownPopover.tsx:51,165` | Anchored to the status bar (window chrome); the nested confirm has window-wide consequence. |

**Action:** No change. These continue to use `Modal v2` + Portal-to-body and the legacy `ModalsRenderer` registry as they do today.

---

## (C) Already Pane-Scoped — Leave Alone

These are inline overlays already mounted inside their pane's component tree. Not modals in the strict sense; they don't Portal anywhere.

| Component | File | Notes |
|---|---|---|
| **AgentFocusedPanel** | `frontend/app/view/agent/components/AgentFocusedPanel.tsx:26` | Half-pane settings overlay; precedent for the per-pane absolute pattern. `_focused-overlay.scss:8-18` shows the CSS approach we're echoing in `TabModalLayer`. |
| **AgentCardSettingsPanel** | `frontend/app/view/agent/components/AgentCardSettingsPanel.tsx:40` | Inline expansion under an agent card; not a modal. |
| **SlashHelpPanel** | `frontend/app/view/agent/components/SlashHelpPanel.tsx:57` | Inline `/help` panel inside agent pane. |
| **ActivityLogPanel** | `frontend/app/view/agent/components/ActivityLogPanel.tsx` | Collapsible section above composer. |
| **SlashCommandPicker** | `frontend/app/view/agent/components/SlashCommandPicker.tsx` | Floating-UI picker tied to slash command flow. |
| **AgentIdentityPanel** | `frontend/app/view/agent/components/AgentIdentityPanel.tsx:45` | Inline identity tab inside `AgentCardSettingsPanel`. |
| **DragOverlay** | `frontend/app/drag/DragOverlay.tsx:43` | Cross-window drop indicator; visual feedback only. |

**Action:** No change. Document these as the "inline overlay" tier.

---

## (D) Re-classify

None found. The four (A) candidates were initially considered for (D) reclassification (mis-scoped today), but they are coherently scoped if you redefine the modal layer — which is exactly what `TabModalLayer` does. So they go in (A).

---

## Out of Scope (Non-Modal Surfaces)

Listed for completeness. None of these need migration; none belong in `TabModalLayer`.

- **Popovers** — `element/popover.tsx`, `statusbar/HostPopover.tsx`, `notification/notificationpopover.tsx`. Floating-UI anchored, not modal.
- **Tooltips** — `element/tooltip.tsx`.
- **Context menus** — dispatched via `ContextMenuModel.showContextMenu()`.
- **TypeAheadModal (connection picker)** — `modals/typeaheadmodal.tsx:87`. Despite the name, this Portal-mounts to `props.blockRef.current` (block-scoped), not body. It's a searchable dropdown, not a modal dialog.
- **ChangeConnectionBlockModal** — `modals/conntypeahead.tsx:303`. Uses TypeAheadModal under the hood; same story.
- **Notifications & notification center** — `notification/notificationpopover.tsx`, `notification/notificationbubbles.tsx`.
- **Emoji palette** — `element/emojipalette.tsx`.
- **FlyoutMenu** — `element/flyoutmenu.tsx`. Portal-rendered dropdown menu.

---

## Open Questions Resolved During Audit

- **Identity dialog?** None as a standalone modal. `AgentIdentityPanel` is inline (C), `AccountForm` is inline. Nothing to migrate.
- **App-level Settings/Preferences?** None as a modal — settings open as a widget pane. Out of scope.
- **First-run / onboarding?** Removed (per `modals/modalregistry.tsx:10` comment).
- **Update available?** Goes through the notification system, not a modal.
- **Quit confirm?** Not in frontend; handled at the host/OS layer.
- **File pickers?** Backend/OS, not frontend.
- **DevTools/inspector?** CEF, not frontend.

---

## Shared Infrastructure Surfacing From the Audit

These should be designed once during Phase 1 and reused by every (A) migration.

1. **Z-index scale.** Add `--z-tab-modal` between pane content and `--z-modal` (the global Modal v2 token, used in `modal-v2.tsx:25`). Document the order: pane content → focused-overlay → tab-modal → global modal → context menu/tooltip.
2. **CEF pane airspace clipping.** Today `Modal v2` calls `usePaneOverlay()` (`modal-v2.tsx:42`) so native CEF panes don't paint over the modal. `TabModalLayer` overlays must do the same against the *tab-scoped* overlay rect, not the viewport. Reusable hook.
3. **Focus trap.** Modal v2 implements its own (`modal-v2.tsx:139-314`). Two options:
   - **Compose:** keep `<Modal>` *inside* `TabModalLayer`'s panel slot, but mount it through the layer rather than through Portal-to-body. This reuses the focus-trap logic for free.
   - **Extract:** pull focus-trap into a `useFocusTrap(ref)` hook and call from both layers.
   Recommended: **compose for v1, extract later if a third user appears.**
4. **Backdrop + ESC behavior.** Same composition argument — let Modal v2 handle backdrop click and ESC, but render the resulting Portal-less node into the tab layer.
5. **State store.** Today the legacy `ModalsModel` (`modalmodel.ts:8`) registers global modals. For (A) modals, state lives on a per-tab `TabModalLayer` context (one per `TabContent`). Distinct stores; do not merge. Each tab's layer is independent and self-cleaning.

---

## Migration Order

### Phase 0 — Land the launch modal first

Ship `launch-modal-rearchitecture-2026-05-01.md` end-to-end. It is the proof. Don't touch any other modal until it's working and shipped.

### Phase 1 — Infrastructure cleanup (after Phase 0)

Once `TabModalLayer` exists, generalize:

1. Move the launch-specific request shape into a discriminated union: `TabModalRequest = { kind: "launch-agent"; ... } | { kind: "confirm"; ... } | { kind: "import-preview"; ... }`.
2. Provide a generic `<TabConfirm>` component (title, body, cancel, confirm) so each (A) migration is a one-line `open({ kind: "confirm", ... })` call.
3. Add the `--z-tab-modal` token + the shared backdrop component.

No user-visible changes in this phase.

### Phase 2 — Mechanical migrations (low-risk)

In this order, smallest blast radius first:

1. **AgentPicker delete confirm** — smallest, isolated to picker. Good first migration to validate `<TabConfirm>`.
2. **ImportPreviewModal** — small, isolated to import flow.
3. **AgentLaunchModal already migrated in Phase 0.**

### Phase 3 — Higher coupling (Medium risk)

1. **AgentView close confirm** — touches pane lifecycle. Verify the close-confirm modal disappears cleanly when its tab is closed externally (e.g., parent tab kill) so the user can't see a stranded modal. Should fall out of `display:none` semantics, but worth a manual test.

### Phase 4 — Hold

Everything in (B) and (C) stays where it is. Do not migrate.

**Total estimated effort:** ~1 engineer-day for Phase 1 + 2 + 3 combined, on top of the launch-modal landing.

---

## Risks

- **R1 — Focus-trap composition.** If we keep `<Modal>` inside `TabModalLayer` for focus-trap, we must ensure the inner Modal does not Portal. Either pass a `disablePortal` flag (cleanest) or inline the rendering primitive. Decide before Phase 1.
- **R2 — Z-index regressions.** Adding `--z-tab-modal` must not interfere with `_focused-overlay.scss:8-18` (`z-index: 20`). Audit all `z-index` literals in the frontend during Phase 1 and consolidate into the token scale.
- **R3 — Per-tab state leak.** Each `TabModalLayer` instance must clean up its signal on tab close. Tabs are mounted-but-hidden today (`tabcontent.tsx:18-20`), so cleanup actually happens on workspace tear-down, not tab close. Acceptable, but document.
- **R4 — Backend-driven pane modals.** If a backend RPC ever wants to surface a confirm/input modal "in the active tab", we need a router that looks up the active tab's layer context. Out of scope for now — current backend RPCs target the global registry.

---

## Migration Complexity Summary

| Category | Count | Effort |
|---|---|---|
| (A) Migrate | 4 | ~1 engineer-day after infra |
| (B) Stay global | 5 | 0 |
| (C) Inline | 7 | 0 |
| (D) Reclassify | 0 | 0 |
| Out of scope | 8+ | 0 |

---

## Bottom Line

The cleanup is **bounded and small**. Four modals migrate, five stay global, seven stay inline. After `TabModalLayer` lands, the rest is mechanical: a generic `<TabConfirm>`, a discriminated request union, and three short PRs. No DB, RPC, or backend changes anywhere in the audit. The infrastructure work is the meaningful design effort; the per-modal migrations are find-and-replace once the API is right.
