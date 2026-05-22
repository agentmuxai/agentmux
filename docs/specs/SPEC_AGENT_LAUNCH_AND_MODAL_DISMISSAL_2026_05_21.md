# SPEC — Agent-launch default view + modal-dismissal discipline

**Status:** Draft / for review
**Date:** 2026-05-21
**Author:** AgentA
**Area:** `frontend/app/view/agent/components/AgentLaunchModal.tsx`,
`frontend/app/element/modal-v2.tsx` (+ `.scss`)

Three related UX changes around the agent launch / edit modals. They are
independent and can ship as separate PRs, but share one spec because they all
concern the same surfaces.

---

## Feature A — Launch defaults to "Continue using existing agent"

### Problem

When the user launches an agent **definition** that already has one or more
existing agent **instances**, the launch modal opens in *New agent* mode.
Continuing an existing agent is reachable only by selecting it from the
"Continue agent" dropdown — an extra, easily-missed step. The common case
(re-open the agent you already set up) should be the default.

### Current state

`AgentLaunchModal.tsx` already has the machinery:

- `namedAgents()` — a server-side list of instances **of the current
  definition** (`agent.tracked` query, `definition_id: props.agent.id`).
  Empty ⇒ this definition has never been launched.
- `continueOfId()` / `continuedRow()` / `isContinue()` — the currently
  selected continuation, mirrored from `flow.state.form.continueOfId`.
- `flow.dispatch({ type: "ContinueOfChanged", continueOfId, carry })`.

What's missing is a **top-level view mode** and the right default.

### Behavior

- The modal has an explicit view mode: **Continue** or **New**.
- **Default:** `namedAgents()` non-empty ⇒ open in **Continue** mode, with the
  most-recent instance preselected (recency order already exists — see the
  `agent_def_list` last-used ordering). Empty ⇒ open in **New** mode.
- A clearly-visible control toggles modes:
  - In Continue mode: a **"Start a new agent instead"** button/link.
  - In New mode (when `namedAgents()` is non-empty): a **"Continue an existing
    agent"** button/link back.
  - When `namedAgents()` is empty, no toggle is shown — New is the only option.
- Switching New → Continue preselects the most-recent instance; Continue → New
  clears `continueOfId` (dispatch `ContinueOfChanged` with empty id).
- Identity/memory lock rules (`continueLocksIdentity` / `continueLocksMemory`)
  are unchanged — they already key off `isContinue()`.

### Notes

This is a presentation-layer default over existing flow state — no new RPC, no
reducer command beyond the existing `ContinueOfChanged`. The view mode is local
modal state derived from `namedAgents()` at open time.

---

## Feature B — Important modals don't dismiss on backdrop click

### Problem

Clicking the dimmed area outside a modal closes it. For a launch or edit modal
this discards in-progress input (instance name, identity/memory selections,
field edits) on a single stray click — a real data-loss footgun.

### Current state

`modal-v2.tsx` `Modal` already supports `closeOnBackdropClick` (default
`true`). `handleBackdropClick` no-ops when it is `false`.

### Behavior

- **Important modals must pass `closeOnBackdropClick={false}`.** A backdrop
  click never disposes them; only an explicit **Cancel** / **Close** /
  submit action does.
- "Important" = any modal that holds unsaved user input or gates a
  consequential action. The audit set (initial — confirm in review):
  - `AgentLaunchModal` (launch)
  - agent **edit** surfaces (`AgentCardSettingsPanel` and the Identity / Memory
    edit modals)
  - `AgentNewMemoryModal`, agent install/import modals with field input
  - **Not** purely-informational modals (AboutModal, error displays) — those
    keep backdrop-dismiss; their only disposition is "dismiss".
- **ESC:** still closes important modals. ESC is a deliberate keypress, not an
  accidental pointer slip — it is not the footgun this addresses. (Open
  decision §Open-1 if review disagrees.)
- Every important modal must therefore render an explicit **Cancel/Close**
  control — see Feature C for how it is made discoverable.

---

## Feature C — Cancel-button nudge on a rejected backdrop click

### Problem

With Feature B, a backdrop click on an important modal does *nothing*. A user
who clicks outside expecting it to close gets silence and may think the app is
stuck. They should be guided to the explicit dismiss control.

### Behavior

- When a backdrop click is **rejected** (`closeOnBackdropClick === false`), the
  modal's primary dismiss control briefly **nudges** — a subtle, fast blink /
  slight scale-up — to draw the eye. No close, no other side effect.
- The nudge is brief (~`--motion-base`), subtle (small scale or
  opacity/glow pulse, not a jarating bounce), and self-clearing.
- Honors `prefers-reduced-motion` — under reduced motion the nudge is a single
  static highlight tick, not an animation (consistent with modal-v2's existing
  reduced-motion handling).

### Implementation sketch

- **Identifying the dismiss control.** Establish a `data-modal-dismiss`
  attribute convention: the Cancel/Close button of every modal carries it.
  `ConfirmModal`'s cancel button and `Modal`'s `showCloseButton` get it
  automatically; custom modals (AgentLaunchModal etc.) tag their cancel button.
- **modal-v2 `handleBackdropClick`:** when `closeOnBackdropClick === false`,
  instead of a bare `return`, set a short-lived `nudge` signal. An effect adds
  a `modal-dismiss--nudge` class to the `[data-modal-dismiss]` element inside
  the panel, removed on `animationend` (or after a fixed timeout under reduced
  motion).
- **CSS (`modal-v2.scss`):** a `@keyframes modal-dismiss-nudge` (blink / slight
  scale), gated by `@media (prefers-reduced-motion: reduce)` for the static
  fallback.
- If a modal has no `[data-modal-dismiss]` element the nudge is a no-op
  (safe degradation).

---

## Open decisions

1. **ESC on important modals** — keep ESC closing (this spec's assumption), or
   route ESC through the same no-close + nudge path as the backdrop?
2. **Nudge animation** — pick one: quick double-blink, ~6% scale-up pulse, or a
   focus-ring glow. (Spec recommends the scale-up pulse — least likely to read
   as an error.)
3. **"Important" audit set** — confirm the §B list; in particular whether
   `AgentCardSettingsPanel` counts (it may auto-save rather than hold a
   dirty buffer).
4. **New-agent toggle when no existing instances** — hidden entirely (this
   spec), or shown disabled with a tooltip.

---

## Rollout

Three independent PRs:

1. **B + C together** in `modal-v2` — the backdrop guard and the nudge are one
   coherent change to the primitive; ship with the audit set wired up.
2. **A** — `AgentLaunchModal` view-mode default + toggle.

B/C first: it is the primitive-level change and unblocks marking A's modal
non-dismissible.
