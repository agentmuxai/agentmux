# SPEC: Floating-pane redock — hover-intent dwell and a neutral parking zone

**Status:** active
**Date:** 2026-09-09
**Implemented:** §5.1 (P1 heartbeat), §5.2 (P2 threshold) and §5.4 (P4
extraction) shipped together — see §8 for why they could not be split. §5.3
(P3, the neutral parking zone) is still open and is deliberately a separate
change; the sections below are written as they were proposed, with §5.4's
ordering caveat noted in §8.
**Area:** floating panes / drag-and-dock
**Scope:** The dwell/velocity gate gating floating-pane redock, on all three
platforms. Tear-off (docked to floating) is out of scope; so is cross-tab pane
drag, which shares `RedockFloatingPane` but not this gate.
**Relates to:** [SPEC_FLOATING_PANE_REDOCK_2026-05-27](SPEC_FLOATING_PANE_REDOCK_2026-05-27.md),
[SPEC_PANE_DRAG_TO_TAB_2026_07_10](SPEC_PANE_DRAG_TO_TAB_2026_07_10.md),
[SPEC_REDOCK_FRAMEWORK_HARDENING_2026_07_27](SPEC_REDOCK_FRAMEWORK_HARDENING_2026_07_27.md)

---

## 1. Problem

Dragging a floating pane over the main window shows the redock ghost and re-docks
the pane the moment the mouse is released. A user who simply wants the floating
pane to *sit above* AgentMux — as an overlay, not as a docked tile — cannot
express that intent: every drag that ends over the main window docks.

**This is a recurrence, not a new report.** The identical complaint was filed
2026-06-03 and quoted verbatim in PR #1249's body:

> "Sometimes when dragging the pane over a main window, I don't want it to dock,
> but it immediately does."

That report was fixed by adding a dwell + velocity gate. Three months later the
same behavior is being reported again. The gate did ship and does work as
written — but its threshold was tuned too low, and the mechanism it depends on
is structurally unable to measure dwell while the cursor is stationary, so it
falls back to inferences that re-open the very hole it was built to close.

## 2. History

| Date | PR / commit | What landed |
|---|---|---|
| 2026-06-08 | [#1249](https://github.com/agentmuxai/agentmux/pull/1249) `3aa3cdd0` | Original fix. Introduced `REDOCK_DWELL_MS = 180` + `REDOCK_VELOCITY_PX_PER_S = 400` in the floater drag handler. Two-stage "armed then committed" model. |
| 2026-06-18 | [#1559](https://github.com/agentmuxai/agentmux/pull/1559) `ddf3177f` | Extended the same 180 ms gate to the *ghost* listener (`app-init.ts`), which had none; extracted both constants into `floating-pane-constants.ts` as a shared source of truth. |

Two facts from that history matter for this spec:

1. **PR #1249 pre-registered this escalation.** Its "Approach" section surveyed
   dwell-time delay, velocity gating, and explicit compass widgets (VS Code,
   Infragistics, Telerik, Dockview), chose dwell + velocity because it
   "composes naturally, no new UI affordances," and stated plainly:
   *"Compass widget is the natural next step if this proves insufficient."*
   It has proven insufficient. Section 5.3 is that next step.

2. **The spec #1249 promised was never written.** Its body cites a
   `SPEC_FLOATING_REDOCK_DWELL_2026-06-03.md` under the specs directory for the
   "full research summary, design rationale, behavior matrix, known limitations,
   and future work." That file has never existed in git history. (Its full path
   is deliberately not written out here: `scripts/check-spec-citations.sh` would
   read it as a live citation, and a pointer to nothing is worse than none.) The industry research and
   the tuning rationale therefore lived only in a PR description, where nobody
   revisiting the constant would find them. `scripts/check-spec-citations.sh`
   did not catch it because it only validates citations in *changed* files.
   This document replaces that missing spec.

## 3. Root causes

Three independent causes. **Fixing only the constant (3.1) will not fix the
reported behavior**, because 3.2 defeats any threshold value.

### 3.1 The threshold is below the human intent boundary

`frontend/app/workspace/floating-pane-constants.ts:12` — `REDOCK_DWELL_MS = 180`.

PR #1249's tuning note justifies it:

> `REDOCK_DWELL_MS = 180` — below Windows' 400 ms hover default because
> drag-aiming feels more deliberate than passive hover.

The premise is sound (aiming *is* more deliberate than passive hover) but the
conclusion is inverted: deliberateness of the *gesture* is not what the delay
gates. The delay gates the **consequence**, and the consequence here is
destructive and non-obvious to undo — the floating window is destroyed, the
pane is grafted into the layout tree, and recovering the previous state means
manually tearing off again and re-positioning the window.

This codebase already reached the opposite conclusion for a strictly *smaller*
action. `frontend/app/tab/tabbar-dnd.ts:62` sets `SPRING_SWITCH_MS = 500`, and
`SPEC_PANE_DRAG_TO_TAB_2026_07_10.md:31-33` justifies it as:

> deliberately longer than the redock ghost's 180 ms since switching the visible
> tab is a bigger action.

Switching which tab is visible is reversible with one click. A redock is not.
The precedent is anchored to the wrong reference point.

### 3.2 Dwell is inferred from the *absence* of events — the load-bearing bug

The Win32 drag loop emits hover state only from inside its `WM_MOUSEMOVE`
handler, throttled to 50 ms (`agentmux-cef/src/ui_tasks/drag.rs:598-616`).
**When the cursor stops moving, `WM_MOUSEMOVE` stops arriving, so hover events
stop entirely.** A dwell clock driven by those events cannot advance during the
exact condition it is meant to measure — holding still.

To compensate, the renderer arms redock through a disjunction of fallbacks that
substitute *elapsed time since the last event* for genuine dwell
(`frontend/app/workspace/floating-pane-workspace.tsx`):

| Line | Fallback | Why it re-opens the hole |
|---|---|---|
| `:533` | `hoverArmed` (real dwell, two confirmed events) | Correct — this is the intended path |
| `:540` | `now - dwellHoverTargetFirstSeenAt >= REDOCK_DWELL_MS` at mouseup | Wall-clock recheck; acceptable |
| `:545` | `dwellHoverTargetFirstSeenAt === null && now - dwellWinLastSampleAt >= REDOCK_DWELL_MS` | **Arms after the velocity gate explicitly rejected the entry**, purely because events went quiet |
| `:652`, `:657`, `:659` | same three, re-evaluated in `window_drag_ended` | Duplicated for the Windows mouseup / drag-ended ordering race |

The `:545` branch is the decisive one. Its own comment describes the intent —
"Hold-still after fast entry: velocity gate preserved the target but cleared the
timer; cursor then stopped so no slow event restarted it" — but the effect is
that **any pause of at least `REDOCK_DWELL_MS` immediately before release arms
the redock, including one the velocity gate just disqualified.**

Every careful repositioning of a floating window ends in exactly that pause: the
user stops moving, confirms the position, then releases. So the gate is bypassed
on precisely the gesture it exists to protect. Raising `REDOCK_DWELL_MS` to 500
in isolation would only change the required pause from 180 ms to 500 ms — still
shorter than any deliberate "place it here and let go."

### 3.3 There is no neutral surface to park over

`frontend/app-init.ts:180-182` maps `DropDirection::Center` (dir `8`) to the
**full leaf rect**. Combined with Top/Right/Bottom/Left (half-leaf) and the
`Outer*` bands (1/5 edge), every pixel of every pane in the main window resolves
to a valid drop zone. There is no region a user can hover over that means
"nothing — I am just passing through, or parking here."

A dwell-only model cannot express parking intent even in principle: parking is
communicated by hovering *longer*, which is the same signal that arms the dock.
The two intents are read from one channel, and dwell resolves the ambiguity in
the wrong direction.

## 4. Best practice

| Product / guideline | Delay before a drag-hover action commits |
|---|---|
| Windows shell hover default | 400 ms |
| macOS spring-loaded folders | ~500-750 ms (user-adjustable) |
| Windows `MenuShowDelay` (submenu open) | 400 ms default |
| AgentMux spring-loaded tabs (`SPRING_SWITCH_MS`) | 500 ms |
| AgentMux redock (today) | **180 ms** |

The consensus band for "hover became intent" is **400-700 ms**. AgentMux's
redock is the outlier by a factor of roughly 2.5.

The second, stronger convention: **mature docking UIs do not infer dock intent
from window-relative position at all.** Visual Studio, JetBrains, Qt, and
Dockview all require the drop to land on an explicit dock guide / compass
target, leaving the rest of the surface neutral so a floating tool window can be
parked anywhere. This is the affordance 3.3 is missing, and the one #1249
deferred.

**Chosen value: `REDOCK_DWELL_MS = 500`** — adopts the repo's own
`SPRING_SWITCH_MS`, sits inside the consensus band, and is above the Windows
400 ms hover default rather than deliberately below it. Choosing the existing
in-repo constant rather than a newly invented number also removes the "two
subsystems, two arbitrary thresholds" divergence noted in 3.1.

## 5. Design

Four phases. **P1 is a prerequisite for P2 being meaningful** — shipping the
constant bump alone would be a cosmetic change to a defeated gate.

### 5.1 P1 — Make dwell measurable while stationary (correctness)

`agentmux-cef/src/ui_tasks/drag.rs:463-464` already installs a 100 ms wake
timer for the entire drag:

```rust
const DRAG_TICK_ID: usize = 0xD9A6;
SetTimer(h, DRAG_TICK_ID, 100, None);
```

Its handler (`:666-672`) currently does nothing but consume the message. Emit
the redock-hover update from that tick as well as from `WM_MOUSEMOVE`, reusing
the existing `last_hover_emit` throttle so a moving cursor does not double-emit:

- Cursor moving: `WM_MOUSEMOVE` drives emission at 50 ms (unchanged).
- Cursor stationary: the 100 ms tick keeps emitting with the last known cursor
  position, so `dwellHoverTargetFirstSeenAt` advances truthfully.

With a genuine heartbeat, the inference fallbacks become dead weight and must be
**deleted, not retuned**: `:545` and its `window_drag_ended` twin `:659` are
removed outright, leaving arming to `hoverArmed` plus the wall-clock recheck
against `dwellHoverTargetFirstSeenAt`. The velocity gate then actually holds —
a rejected fast entry stays rejected until the cursor genuinely slows and dwells.

The macOS/Linux JS-driven path already receives continuous `mousemove` and needs
no heartbeat; it needs only the same fallback deletion
(`:560`, `:562` — the `dwellSlowSince` / `dwellCurrentConfirmedAt` proxies).

### 5.2 P2 — Raise the threshold

`REDOCK_DWELL_MS: 180 -> 500` in `floating-pane-constants.ts:12`.

`REDOCK_VELOCITY_PX_PER_S` stays at 400 — it is a rate, not a duration, and
#1249's derivation (relaxed aim is about 100 px / 250 ms; transit drags hit
1500+ px/s) is still sound.

One coupling to honour: `floating-pane-workspace.tsx:830` fires the hover IPC at
`REDOCK_DWELL_MS - HOVER_THROTTLE_MS` (`HOVER_THROTTLE_MS = 50`, `:780`) so the
ghost is painted before `mouseup` can commit. That relationship is preserved
automatically by the subtraction, but the ghost's first paint moves from ~130 ms
to ~450 ms. That is intended: **the ghost appearing is the user-visible signal
that release will dock**, so it must not appear before the gesture has been
recognised as a dock attempt. It should remain the case that ghost-visible and
armed are the same instant, never ghost-visible-but-not-armed.

### 5.3 P3 — A neutral parking zone (the compass escalation)

Restrict redock arming to explicit drop-guide regions rather than the whole leaf:

- Keep the `Outer*` edge bands (1/5) and the Top/Right/Bottom/Left half-splits.
- **Remove `Center` = full-leaf as an implicit arming target.** Center-drop
  (append into the leaf) remains available, but only from an explicit central
  guide target — a compass hit-rect of bounded size (proposal: 96x96 CSS px
  centred in the leaf), not the entire remaining surface.

The result is a genuinely neutral majority of each pane's area: hovering there
shows no ghost and arms nothing, so a floating pane can be parked anywhere over
AgentMux by simply not aiming at a guide. This directly answers the user's
stated goal ("the user may simply want the floating pane above, not a redock")
in a way no dwell value can.

If P3's UI work is deferred, an interim escape hatch is a modifier-key
suppression (hold <kbd>Alt</kbd> during the drag: never arm, never ghost),
which is roughly ten lines and requires no new rendering. It is strictly worse
UX than guides (undiscoverable) and should be treated as a stopgap, not the
answer.

### 5.4 P4 — Collapse the duplicated gate (debt paydown)

`docs/retro/retro-redock-ghost-landing-reliability-2026-07-27.md:47` already
flagged this:

> The dwell/velocity hover gate is hand-implemented twice (once for the Windows
> native-move-loop path, once for the macOS/Linux JS-driven path) with
> near-duplicate state — any future gating tweak has to be applied symmetrically
> by hand or platforms silently diverge.

This spec is exactly such a tweak, and it touches both copies. Extract the
arming state machine into one module (proposal:
`frontend/app/workspace/redock-arming.ts`) exposing a platform-agnostic
`onHoverSample({ target, x, y, t })` returning `{ armed, indicator }`, with the
two platform paths reduced to feeding it samples. This is what makes P1-P3
testable (section 7) — the current state is spread across roughly twenty mutable
closure variables in an `onMount` body and cannot be unit-tested at all.

## 6. Blast radius

| File | Lines | Role | Phase |
|---|---|---|---|
| `frontend/app/workspace/floating-pane-constants.ts` | `12`, `15` | Constant definitions | P2 |
| `agentmux-cef/src/ui_tasks/drag.rs` | `463-464`, `598-616`, `666-672` | Win32 hover emission + wake tick | P1 |
| `frontend/app/workspace/floating-pane-workspace.tsx` | `53` (import), `533`, `540`, `545`, `560`, `562`, `652`, `657`, `659` (arming), `730`, `808` (velocity), `765` (dwell arm), `780`, `830` (IPC lead-in) | Floater-side gate, both platform paths | P1, P2, P4 |
| `frontend/app-init.ts` | `59` (import), `180-182` (`rectForDirection` Center), `187` (`ghostDwellMs`), `217` (ghost show gate) | Ghost-side gate + drop-zone mapping | P1, P3 |
| `frontend/app/tab/tabbar-dnd.ts` | `62` | `SPRING_SWITCH_MS` — update its comment, which currently anchors itself to the 180 ms value | P2 |

No Rust code reads either constant; the Rust change in P1 is emission cadence
only. `SPEC_PANE_DRAG_TO_TAB_2026_07_10.md:31-33` and `:164` cite the 180 ms
value in prose and must be corrected in the same PR.

## 7. Test plan

**There is currently no test coverage of any of this** — no vitest or Rust test
references `REDOCK_DWELL_MS`, dwell arming, or velocity rejection. Changing the
constant today breaks no test, which is the risk, not the reassurance. P4's
extraction exists to make the following table testable as pure functions:

| # | Scenario | Expected |
|---|---|---|
| 1 | Fast transit across main window, release while over it | No ghost, no dock (regression guard for #1249) |
| 2 | Slow aim, hold 200 ms, release | **No dock** (this docked before) |
| 3 | Slow aim, hold 600 ms over a guide, release | Ghost visible, docks |
| 4 | Move floater over main window, pause 2 s in a neutral (non-guide) area, release | No ghost, no dock, floater stays (P3) |
| 5 | Fast entry, velocity rejects, stop dead, release after 600 ms | **No dock** (kills the `:545` inference path) |
| 6 | Cursor stationary over a guide for 600 ms with zero `WM_MOUSEMOVE` | Arms (proves the P1 heartbeat works) |
| 7 | Ghost visible, then drag away fast | Ghost clears within one frame; release elsewhere gives no dock |
| 8 | Same matrix on macOS + Linux JS-driven path | Identical outcomes (P4 symmetry) |

Manual verification is required for 6 and 8 (host-loop and platform behavior);
1-5 and 7 should be unit tests against the extracted arming module.

## 8. Rollout

1. **P1 + P2 together** in one PR — the heartbeat plus the fallback deletions
   plus the constant bump. Splitting them ships either a defeated gate (P2 alone)
   or an unexplained behavior change (P1 alone).
2. **P4** immediately after, as a pure refactor with the section 7 tests added
   against the extracted module. Doing it before P1/P2 would mean writing tests
   for logic that is about to be deleted.
3. **P3** as its own PR with design review — it changes a visible affordance and
   deserves to be evaluated on its own, per #1249's framing of the compass
   widget as a distinct step.

**What actually shipped (2026-09-09):** P4 was folded into step 1 rather than
following it. Step 2's rationale above assumed P4 meant "extract the existing
logic," which would indeed have been tests-for-code-about-to-be-deleted. In
practice the fallback deletions in P1 *are* the extraction — the arming rules
that survive are small enough to state as a pure function, and writing them
into `redock-arming.ts` directly was less work than editing them in place
twice. The section 7 table is therefore covered by unit tests in this PR
rather than the next one. P3 remains a separate, unstarted change.

## 9. Open questions

- **Should the dwell be user-configurable?** `window:*` settings already exist
  (`agentmux-srv/src/backend/wconfig/mod.rs`), so `window:redockdwellms` is
  cheap. Recommendation: **no, not initially.** A correct default plus a neutral
  zone (P3) should remove the need; a setting would ossify the current
  one-channel-two-intents design rather than fix it.
- **Should `SPRING_SWITCH_MS` and `REDOCK_DWELL_MS` become one constant?** They
  would share a value (500) after P2 but gate different subsystems. Keep them
  separate, and update the stale cross-reference comment at `tabbar-dnd.ts:62`
  rather than merging them.
- **Does the P1 heartbeat cost anything measurable?** The 100 ms tick already
  runs; the added work is one `resolve_window_at_cursor` Z-order walk per tick
  while stationary. Expected negligible, but worth confirming against the
  `[redock-resolve]` diagnostic already present at
  `agentmux-cef/src/commands/window/motion.rs:299-301`.
