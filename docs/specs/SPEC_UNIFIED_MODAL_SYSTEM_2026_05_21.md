# SPEC — Unified modal system (scope-based)

**Status:** Draft / for review
**Date:** 2026-05-21
**Author:** AgentA
**Area:** `frontend/app/element/modal-v2.{tsx,scss}`, `frontend/app/tab/TabModalLayer.tsx`
+ `tab-modal.*`, `frontend/app/modals/*`
**Supersedes / completes:** `SPEC_ROBUST_MODAL_SYSTEM_2026_04_23` (its v1
retirement, never finished), `launch-modal-rearchitecture-2026-05-01.md`
(TabModalLayer), and folds in `SPEC_AGENT_LAUNCH_AND_MODAL_DISMISSAL_2026_05_21.md`
Features B/C.

---

## 1. Summary

AgentMux has **three** modal systems. This spec replaces them with **one**,
organised around a single first-class axis: **what the modal locks** —
a **pane**, a **tab**, or the **window**.

One primitive: `<Modal scope="pane" | "tab" | "window" …>`. Everything else
(mount point, backdrop extent, `inert` boundary, scroll lock, pane-overlay
clip) is a *consequence* of the scope. "Modal types" collapse to that axis;
content shapes (confirm / form / info) are presets layered on top.

---

## 2. Why — current state

| System | Role | Status |
|---|---|---|
| `element/modal-v2.tsx` (`Modal`, `ConfirmModal`) | window-level dialogs | modern, accessible — 8 importers |
| `tab/TabModalLayer.tsx` + `tab-modal.*` | tab-scoped agent modals (launch/install/identity/memory/prereq/auth) | deliberate parallel layer |
| `modals/` registry (`modalregistry` + `modalsrenderer` + `modalmodel`, `pushModal`) | legacy registry: `MessageModal`, `UserInputModal`, `TypeaheadModal`, … | **stranded legacy** — `<ModalsRenderer/>` still mounted in `workspace.tsx` |

Two distinct reasons for the split:

- **modal-v2 ↔ v1 registry — an unfinished migration.** modal-v2's header
  says it outright: it "retires `element/modal.tsx` and the `modals/` wrapper
  once callers migrate (`SPEC_ROBUST_MODAL_SYSTEM_2026_04_23` §6 PRs 3–5)."
  Those PRs were never completed. Pure cruft. (Half-migrated artifact:
  `AboutModal` is *registered* in the v1 registry yet *renders* a modal-v2
  `<Modal>` internally.)
- **TabModalLayer — deliberate, not cruft.** Agent modals must render *inside*
  the tab's DOM (sibling of `<TileLayout>` in `TabContent`) so they hide with
  the tab (`display:none`) and clip against `TabContent` for the native
  pane-overlay system. modal-v2 portals to the window body — the wrong mount
  for a tab-scoped modal.

The fix is not "rename v2" — it is to make **scope** first-class so all three
become one parameterised system.

---

## 3. The scope model

A modal **locks a region**; everything outside that region stays interactive.

| `scope` | Locks | Mount node | Backdrop covers | Inert boundary | Replaces |
|---|---|---|---|---|---|
| `window` | the whole window | portal → window `document.body` | full window | document (minus modal root) | modal-v2 |
| `tab` | one tab's content | that tab's content root | tab content rect | tab content (panes); tab bar + other tabs stay live | TabModalLayer |
| `pane` | one pane / tile | that pane's root element | pane rect | that pane only; rest of tab + window live | *(new capability)* |

`pane` scope has **no callers today** — the capability is built, and the first
feature that needs an in-pane dialog (per-pane settings, in-pane confirm)
adopts it. Window + tab are migrations of existing behaviour.

---

## 4. Unified API

```tsx
<Modal
    open={…}
    scope="window" | "tab" | "pane"     // default "window"
    target={…}                           // tab/pane: which one — see §7
    onClose={() => …}
    closeOnBackdropClick={false}          // §9
    closeOnEscape={true}
    size="sm" | "md" | "lg"
    …a11y props (ariaLabel, initialFocus, …)
>
    <ModalHeader title="…" />
    <ModalBody>…</ModalBody>
    <ModalFooter>…</ModalFooter>
</Modal>
```

- **Declarative is canonical.** `<Modal>` driven by an `open` signal.
- **Imperative sugar** — a single `openModal({ scope, target, render })` helper
  may wrap it for call-sites that genuinely need fire-and-forget (replacing
  `pushModal`). The legacy registry/`displayName` indirection is dropped.
- `ConfirmModal` and other presets (§10) compose `<Modal>` unchanged.

---

## 5. Lock & inert semantics

Each open modal has a **lock region** (the element resolved per §3). The system:

1. Renders a **backdrop** sized to the lock region.
2. Applies `inert` to the lock region's content *siblings* of the modal — so
   keyboard/SR focus cannot escape into the locked area.
3. **Scroll lock** applies to the lock region only (window: body scroll lock,
   as today; tab/pane: the region's own scroll container).

This is the core redesign vs modal-v2, which inerts document-wide
unconditionally. The inert boundary must be **scope-relative**.

---

## 6. Stacking & coordination

A single global **modal stack**; each entry records `{ id, scope, lockEl }`.

- A modal blocks everything **within its lock region**, including
  lower-stacked modals whose lock region is contained in it (a `window` modal
  covers a `tab`/`pane` modal beneath it).
- Modals with **non-overlapping** lock regions coexist independently — e.g.
  two `pane` modals in different panes, or a `pane` modal and a `tab` modal in
  a different tab.
- **ESC / backdrop** act on the *reachable topmost* — the highest modal not
  contained within a higher modal's lock region.
- This is the second core redesign: modal-v2's stack is a flat z-order; the
  unified stack must reason about scope containment.

---

## 7. Mount & target resolution

- `window` — `Portal` into the originating window's `document.body`
  (multi-window aware, as modal-v2 already does via `resolveMountDocument()`).
- `tab` — mounts into the tab content root. A `TabModalScope` context
  (the slimmed-down successor to `TabModalLayer`) supplies the mount node;
  a `<Modal scope="tab">` opened from within a tab resolves its tab from context.
- `pane` — mounts into the pane root; resolved from the existing pane/block
  context.

Each scope registers its backdrop rect with the **pane-overlay clip** system
(native CEF browser panes must cut a transparent hole) — modal-v2's
`ModalPaneOverlayClip` and TabModalLayer's `PaneOverlayClip` converge into one
scope-aware helper.

---

## 8. Ported vs rebuilt

**Ported as-is** (correct, scope-agnostic — do *not* re-derive):
- Focus trap via sentinel spans; focus save on open / restore on close.
- ARIA: `role="dialog"`, `aria-modal`, generated `aria-labelledby`.
- `prefers-reduced-motion` handling; the transition model
  (`SPEC_MODAL_TRANSITIONS_2026_05_18`).
- `ModalHeader` / `ModalBody` / `ModalFooter` and the `modal-panel-*` classes.

**Redesigned around `scope`:**
- Mount target resolution (§7) — was always `Portal(body)`.
- `inert` + scroll-lock boundary (§5) — was document-wide.
- Backdrop sizing — was full-window.
- The modal stack (§6) — was flat z-order.

---

## 9. Dismissal (folds in `SPEC_AGENT_LAUNCH_AND_MODAL_DISMISSAL` B/C)

- `closeOnBackdropClick` (default `true`). **Important modals pass `false`** —
  a backdrop click then does not dismiss (no data-loss footgun).
- **Cancel nudge:** when a backdrop click is rejected, the panel's
  `[data-modal-dismiss]` control briefly nudges (subtle scale-up pulse;
  reduced-motion → a static highlight tick). No-op if the panel has no
  dismiss control.
- **ESC** closes the reachable-topmost modal regardless of
  `closeOnBackdropClick` — a deliberate keypress, not a stray click.
- The in-flight `agenta/modal-no-backdrop-dismiss` branch (B/C against
  `TabModalLayer`) is **superseded** — the behaviour lands here as part of the
  unified primitive instead of being bolted onto the soon-to-be-retired layer.

---

## 10. Content presets

Architecture is `scope`; content shape is a preset composing `<Modal>`:
- `ConfirmModal` — title + body + Cancel/Confirm (exists; rehome onto unified `<Modal>`).
- `MessageModal`, `UserInputModal`, `TypeaheadModal` — reimplement as presets.
- Command palette, About — `scope="window"` presets.

No preset introduces new architecture.

---

## 11. Migration plan

1. **Build the unified primitive** at `@/element/modal` (drop the `-v2`
   suffix) — the `scope` model, porting §8's a11y core. `pane` implemented.
2. **Window callers** — the 8 `@/element/modal-v2` importers → `@/element/modal`,
   `scope="window"` (default — mostly an import swap).
3. **Tab callers** — agent modals → `<Modal scope="tab">`; `TabModalLayer`
   shrinks to a context that supplies the tab mount node (or is absorbed
   entirely). Delete `tab-modal.*` once empty.
4. **v1 registry** — `MessageModal` / `UserInputModal` / `TypeaheadModal` /
   command-palette → unified presets; delete `modalregistry`, `modalsrenderer`,
   `modalmodel`, drop `<ModalsRenderer/>` from `workspace.tsx`, remove `pushModal`.
5. **Delete** `modal-v2.{tsx,scss}` (content moved to `modal`). One system.

Each stage is its own PR; the app keeps building between stages (the unified
primitive co-exists with the old ones until the last caller moves).

---

## 12. Open decisions

1. **Imperative API** — ship `openModal({scope,…})` sugar, or convert every
   `pushModal` call-site to a declarative `open` signal? (Lean: thin sugar —
   some call-sites are deep in non-component code.)
2. **`pane` scope now or design-only** — build the implementation in stage 1
   with no callers, or stub it and implement when the first feature lands?
   (Lean: build it — the inert/stack design must account for it regardless,
   and an untested code path rots.)
3. **Does a `tab` modal block the tab bar?** Spec assumes **no** (tab bar
   stays live — matches TabModalLayer today). Confirm.
4. **Cross-scope stacking** — exact rule when a `window` modal opens over a
   `pane` modal: the pane modal is inert'd but stays mounted (spec's
   assumption) vs forced-closed.
5. **Naming** — `@/element/modal` for the primitive; what becomes of the
   `app/modals/` directory (home for presets, or also retired)?

---

## 13. References

- `docs/specs/SPEC_ROBUST_MODAL_SYSTEM_2026_04_23.md` — modal-v2's origin; its
  unfinished §6 v1-retirement is completed here.
- `docs/specs/launch-modal-rearchitecture-2026-05-01.md` — TabModalLayer.
- `docs/specs/SPEC_MODAL_TRANSITIONS_2026_05_18.md` — the animation model to port.
- `docs/specs/SPEC_AGENT_LAUNCH_AND_MODAL_DISMISSAL_2026_05_21.md` — Features
  B/C fold into §9; Feature A (launch-modal Continue-default) is independent
  content work, unaffected.
