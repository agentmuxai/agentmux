# SPEC: Keyboard-driven pause for the AskUserQuestion auto-timeout countdown

**Date:** 2026-08-20
**Status:** proposed
**Builds on:** `docs/specs/SPEC_ASK_USER_QUESTION_TIMEOUT_HOVER_PAUSE_2026_08_10.md`
(implemented). This spec adds a *second trigger* for that spec's existing
`hidden`/hide-timer mechanism — it does not change what "paused" means, how
long a pause lasts, how it resumes, or anything about §2's "live but
strictly bounded" safety argument. It also does not reopen
`docs/specs/SPEC_ASK_USER_QUESTION_AUTO_TIMEOUT_2026_08_06.md`'s original
"merge, no disarm" decision, for the same reason the hover-pause spec's own
§2 already established: a bounded, self-resuming pause is a different kind
of mechanism than a permanent one-shot disarm, and this spec doesn't touch
that distinction at all — it only adds a second *way to enter* an existing,
already-bounded pause state.

---

## 0. Ask

> for the askquestion, we need to extend the timer to listen to keyboard
> events too .. urrently it only listens to mouse. take a look, write spec
> to file

---

## 1. Current behavior (audited against source, 2026-08-20)

`frontend/app/view/agent/components/AgentQuestionPanel.tsx`, confirmed
against the live file directly (not from the hover-pause spec's own prose,
which predates one change noted below):

- The hover-pause mechanism from the prior spec is implemented exactly as
  that spec describes: `hidden` signal (line 150), `hideTimeoutId` +
  `clearHideTimer` (lines 151–158), `onPanelPointerEnter` (lines 204–211)
  which hides the countdown and starts a flat, unconditional
  `HOVER_HIDE_GRACE_MS` (15s, line 47) window, and the timer-arm effect
  (lines 350–367) gated on `hidden()`.
- **The only thing that currently calls `onPanelPointerEnter` is
  `onMouseEnter` on the panel's root `<div class="agent-question-panel">`**
  (line 479) — confirmed via grep, this is the sole call site. There is no
  `mousemove`/`mousedown`/`click`/`focus`/`keydown` listener wired to it
  anywhere in the file.
- **One relevant drift from the hover-pause spec's own text:**
  `AUTO_TIMEOUT_MS` is no longer a hardcoded constant — it's now
  `autoTimeoutMs()` (lines 118–138), reading `agent:askquestiontimeoutms`
  from settings with `DEFAULT_AUTO_TIMEOUT_MS` (30s, line 39) as the
  fallback. Not relevant to this spec's design (the timer-arm effect
  already calls `autoTimeoutMs()` fresh at line 355), but worth naming so a
  reader comparing this file against the hover-pause spec's own quoted code
  isn't confused by the rename.
- **A keyboard listener already exists in this file, for a different
  purpose.** `handleKey` (lines 395–421), wired via a global
  `window.addEventListener("keydown", ..., true)` inside its own
  `createEffect` (lines 427–432, gated on `request()`). It handles exactly
  two keys: `Enter` (submit, unless the target is an editable control
  outside this panel) and `Escape` (defer/minimize). Every other key is a
  no-op today — typing a letter into the "Other" field, or Tab-navigating
  between radio options, does nothing to the countdown at all.
  - This listener is global-on-`window` with capture, specifically because
    the panel root has `tabindex={-1}` and never auto-focuses (see the
    comment at lines 423–426) — a plain `onKeyDown` on the root div would
    only fire after the user had already clicked something inside it. This
    same reasoning is why the new keyboard-pause trigger below must reuse
    this listener rather than attach a local one — see §3.3.
  - It already has the exact scoping primitives this spec needs:
    `paneRoot`/`paneRoot.contains(target)` (lines 400–401, scopes to "this
    question's own pane, not some other pane on screen") and `inPanel`
    (lines 403–406, scopes to "the keystroke actually landed inside this
    panel's own DOM, not just its pane") and `isEditableTarget` (lines
    388–393).

**Gap:** a user who answers (or is about to answer) purely by keyboard —
Tab-ing between options and pressing Space, or typing into the "Other"
field without ever moving the mouse over the panel — gets none of the
breathing room the hover-pause spec added for mouse users. The countdown
keeps ticking and can auto-submit out from under them mid-keystroke, which
is exactly the scenario the hover-pause spec exists to prevent for mouse
users (§0 of that spec: "if there is any mouse hover for typing into that
pane, the timer disappears").

---

## 2. Design

### 2.1 Trigger: reuse `onPanelPointerEnter`, don't parallel it

**Decision: any qualifying keydown calls the existing
`onPanelPointerEnter()` directly.** Not a new signal, not a new timeout, not
a new grace-period constant — the exact same function the mouse path
already calls, so "paused" has one single meaning and one single
implementation regardless of what triggered it. This is the direct
extension of the DRY principle the hover-pause spec itself was built under
(see that spec's own "Reuses" line).

Practically: `onPanelPointerEnter` hides the countdown and starts a flat,
unconditional `HOVER_HIDE_GRACE_MS` window from *this* trigger — identical
behavior, identical bound, whether the trigger was a `mouseenter` or a
qualifying `keydown`. Everything in the prior spec's §2 ("live but strictly
bounded," worst case one `HOVER_HIDE_GRACE_MS` window per fresh trigger)
carries over unchanged, because it's the same mechanism, not a new one.

### 2.2 Scope: which keydowns count

**Decision: reuse `handleKey`'s existing `inPanel` check** (lines 403–406)
— a keydown counts as engagement only if it targets something inside the
panel's own DOM (an option, the "Other" input, the panel root, or any focus
inside it), the same scope the hover-pause spec chose for mouse entry
(§3.1 of that spec: "the entire expanded panel," not narrowly the "Other"
input, and not the wider pane).

Considered and rejected: firing on any keydown within the whole pane
(`paneRoot.contains(target)`, the *broader* scope `handleKey` already uses
for Escape). Rejected because a user typing in the main chat composer, or
searching with Ctrl+F, elsewhere in the same pane, is not "engaged with
this question" — pausing its timer on unrelated keystrokes would silently
extend the safety-net window for a question nobody is actually looking at,
which is the opposite of what the hover-pause spec's own bound was
designed to guarantee stays tight. `inPanel` — mirroring the mouse trigger's
own "must actually enter the panel" scope exactly — is the correct
analog.

**Excluded: the minimized chip**, for the identical reason the hover-pause
spec excludes it from mouse-hover (§3.1 of that spec): no typing surface,
no keyboard focus target, and the "keeps running while minimized" guarantee
is untouched by this spec.

### 2.3 Where the listener lives: extend `handleKey`, don't add a second one

**Decision: add the pause trigger inside the existing `handleKey` function**
(lines 395–421), not a second `window.addEventListener`. Two independent
global capture-phase keydown listeners on the same window, both scoped by
overlapping-but-not-identical logic, is a maintenance hazard for no
benefit — `handleKey` already runs on every keydown this panel cares about
and already has the exact scoping primitives (§2.2) this needs. The
existing listener-registration effect (lines 427–432) is unchanged; only
`handleKey`'s body gains one new line.

Sketch (illustrative, not final code — implementation is a separate PR):

```ts
const handleKey = (e: KeyboardEvent) => {
    const target = e.target as HTMLElement | null;
    const paneRoot = rootRef?.closest(".agent-view") as HTMLElement | null;
    if (paneRoot && target && !paneRoot.contains(target)) return;

    const inPanel = !!rootRef && !!target && rootRef.contains(target);

    // New: any keydown that actually lands inside this panel counts as
    // engagement, the same as a mouseenter — see
    // SPEC_ASK_USER_QUESTION_TIMEOUT_KEYBOARD_PAUSE_2026_08_20.md §2.
    // Deliberately unconditional on which key (§2.4) and fires before the
    // Enter/Escape branches below, which is a no-op change for those two
    // keys specifically since they tear the panel state down/reset it
    // anyway (§2.4).
    if (inPanel) onPanelPointerEnter();

    if (e.key === "Enter" && !e.shiftKey) {
        if (!inPanel && isEditableTarget(target)) return;
        e.preventDefault();
        submit();
    } else if (e.key === "Escape") {
        e.preventDefault();
        defer();
    }
};
```

### 2.4 Does it matter that this also fires for Enter/Escape?

**Decision: no special-casing — let it fire for every in-panel key,
including Enter and Escape.** Considered excluding those two (since Enter
submits and Escape defers, both of which make the just-started hide-timer
moot within the same tick) and rejected it as unnecessary complexity for a
harmless case:

- **Enter (submit):** `submit()` doesn't touch `hidden`/`hideTimeoutId` at
  all, so the freshly-started hide-timer is simply abandoned. If
  `props.onAnswer` advances the queue to a new `tool_use_id`, the reset
  effect (lines 170–178) unconditionally clears it (`clearHideTimer();
  setHidden(false);`) same as it always does. If the component unmounts
  instead, `onCleanup(clearHideTimer)` (line 163) tears it down. No leak
  either way — see §4 for the explicit case.
- **Escape (defer):** `defer()` (lines 369–379) already calls
  `clearHideTimer(); setHidden(false);` itself, *before* `setMinimized(true)`
  — so a hide-timer started one line earlier by the same keydown is
  immediately cleared again by `defer()`'s own existing cleanup. Net
  effect: none.

Special-casing these two keys out would only save starting-then-instantly-
discarding one `setTimeout`, at the cost of a second conditional a reader
has to reconcile against `handleKey`'s existing branches. Not worth it.

---

## 3. Edge cases

| Case | Behavior |
|---|---|
| User Tabs into the panel (no typing yet) | Tab is a keydown targeting an element now inside the panel → counts as engagement, same as a mouseenter. Countdown hides, resumes in 15s at a fresh timeout, same as the mouse path. |
| User types into "Other" continuously for 20s straight | Same as continuous mouse hover today (§4 "Mouse enters and never leaves" case in the hover-pause spec): the countdown still resumes visibly 15s after the *first* keystroke and runs its normal course from there — typing alone does not extend the hide window past `HOVER_HIDE_GRACE_MS`. A second keystroke *after* that resumption starts a fresh window, same recursive behavior as repeated mouse re-entry. |
| User presses Space to toggle a checkbox option | Space while focus is on a checkbox `<input>` inside the panel → `inPanel` is true → counts as engagement. No special handling needed; falls out of the existing scoping. |
| User is mid-hover (mouse-triggered hide already active) and also presses a key | `onPanelPointerEnter()` re-arms the same hide-timer from this new trigger (`clearHideTimer()` at its top, same as any repeated call) — equivalent to the existing "fresh mouseenter mid-window restarts the window" case, just via a different trigger. One shared mechanism, no cross-trigger special case needed. |
| User presses Enter while a question is fully answered | `onPanelPointerEnter()` fires first (harmless, see §2.4), then `submit()` — unchanged behavior, no visible effect from the new code. |
| User presses Escape | `onPanelPointerEnter()` fires first, immediately undone by `defer()`'s own `clearHideTimer()`/`setHidden(false)` — net no-op, see §2.4. |
| Keydown targets the main chat composer or Ctrl+F search, elsewhere in the same pane | `inPanel` is false (target isn't inside `rootRef`) → does not trigger the pause, even though `paneRoot.contains(target)` is true and `handleKey` still runs. Matches §2.2's decision not to use the broader pane-level scope. |
| Keydown happens while the panel is minimized | The minimized chip isn't inside `.agent-question-panel` (it's a separate `<button class="agent-question-panel-minimized">`, lines 450–468) — `rootRef` points at whichever root is currently mounted, so `inPanel` is false while minimized. No pause triggered; matches the mouse path's existing exclusion of the minimized chip (§2.2). |

---

## 4. Resolved design decisions

1. **New mechanism vs. reuse — resolved: reuse `onPanelPointerEnter`
   directly.** §2.1. One pause mechanism, two triggers, not two parallel
   pause mechanisms that could drift apart.
2. **Scope of "keyboard activity" — resolved: `inPanel`, mirroring the
   mouse trigger's whole-panel (not whole-pane) scope.** §2.2. Rejected the
   broader pane-wide scope `handleKey` uses for Escape, because unrelated
   keystrokes elsewhere in the pane aren't engagement with *this* question.
3. **New listener vs. extend the existing one — resolved: extend
   `handleKey`.** §2.3. Avoids a second global capture-phase listener with
   overlapping scoping logic to keep in sync with the first.
4. **Special-case Enter/Escape — resolved: no, let them fire through
   uniformly.** §2.4. Both are provably harmless no-ops given the existing
   cleanup paths; special-casing would add complexity for zero behavioral
   difference.

---

## 5. Non-goals

- No new grace-period constant — reuses `HOVER_HIDE_GRACE_MS` (15s)
  unchanged. A keyboard-specific duration was not requested and there's no
  stated reason keyboard engagement should be trusted for a different
  length of time than mouse engagement.
- No change to what happens at zero (`applyRecommendedDefaults`,
  `submit()`, `autoFilledCount`) — entirely orthogonal to this spec, same
  as the hover-pause spec's own §2 non-goal list.
- No change to the minimized-chip countdown, or to `defer()`'s existing
  "always force an immediate resume" behavior — §2.2, §2.4.
- No configurability of scope or duration — matches both prior specs'
  "hardcoded, no settings toggle" stance for the grace window; only
  `autoTimeoutMs()` itself is user-configurable, and that's unrelated to
  this spec.
- `AgentDecisionPanel` remains out of scope, same as both prior specs.

---

## 6. Files touched (implementation, not this spec)

- `frontend/app/view/agent/components/AgentQuestionPanel.tsx` — one new
  call (`onPanelPointerEnter()`) inside the existing `handleKey` function,
  gated on the existing `inPanel` check. No new signals, no new timers, no
  new listener registration.
- `frontend/app/view/agent/components/AgentQuestionPanel.test.tsx` — new
  test cases per §7. No changes to existing mouse-hover tests expected;
  this is purely additive.
- No `.scss` changes — the countdown's hidden state already renders via the
  existing `<Show when={!hidden()}>` (line 489), unaffected by which
  trigger set `hidden` to `true`.
- No `agentmux-srv` (Rust) changes, no wire-format changes — entirely
  frontend-local, same as both prior specs in this chain.

---

## 7. Test plan

**Unit** (`AgentQuestionPanel.test.tsx`, extending the existing
`vi.useFakeTimers()` conventions and the file's existing raw
`KeyboardEvent` dispatch helpers — `enterOn`/`escapeOn` — with a new
generic helper, e.g. `keydownOn(el, key)`):

- A qualifying keydown inside the panel (e.g. `Tab`, or a printable
  character while focus is on the "Other" input) hides the countdown
  immediately, mirroring the existing "mouse enters at t=5s" test.
- Countdown resumes exactly 15s after that keydown, at a fresh
  `autoTimeoutMs()` — mirroring the existing hover-resume test.
- A second qualifying keydown mid-window restarts the 15s window from that
  second keydown — mirroring the existing "recursive" mouse test.
- **Regression guard, mirroring the hover-pause spec's own §9 guard:** a
  keydown followed by *no further activity at all* for the rest of the
  test still resumes and auto-submits on schedule — confirms this doesn't
  reintroduce an indefinite-pause failure mode via a different trigger.
- Keydown targeting an element *outside* the panel but inside the pane
  (e.g. a mock composer textarea in the same `.agent-view`) does **not**
  hide the countdown — confirms the `inPanel` scoping decision (§2.2), not
  the broader pane scope.
- Keydown while minimized does not hide/pause anything (there's nothing to
  hide — the countdown chip in the minimized state is unaffected, matching
  existing behavior).
- Enter and Escape, fired inside the panel, still submit/defer exactly as
  today — confirms §2.4's "no special-casing needed" claim isn't a
  regression on the pre-existing key-handling tests.
- A keydown-triggered pause and a mouse-triggered pause compose correctly:
  hover in, then press a key mid-window — window restarts from the
  keydown, single shared `hidden` state, no double-hide/double-timer
  artifacts.

**Manual / integration:**

- `task dev`, trigger a live `AskUserQuestion`, answer entirely via
  keyboard (Tab between options, Space to select, type into "Other")
  without ever moving the mouse over the panel. Confirm the countdown
  disappears on first keystroke and behaves identically to the
  mouse-hover flow described in the hover-pause spec's own manual test
  plan.
- Tab into the panel, then stop touching the keyboard or mouse entirely.
  Confirm the countdown reappears at 15s showing a fresh full timeout, and
  proceeds to auto-submit at 0 if still untouched — the keyboard analog of
  the hover-pause spec's own "cursor parked, no further activity" manual
  check.

---

## 8. Post-review revisions (2026-08-24, PR #2787)

Three gaps found during review, all fixed before merge:

1. **reagentx P1 — unbounded pause via key-repeat/continuous typing.** §2.1's
   "any qualifying keydown calls `onPanelPointerEnter()` directly" turned out
   not to be a safe direct reuse: `mouseenter` only fires once per actual
   boundary-crossing (a real browser can't spam it under normal use), but
   `keydown` fires on every keystroke, including OS key-repeat and every
   character typed. Re-arming unconditionally on each one meant typing
   faster than 15s apart — or simply holding a key — suppressed the
   auto-timeout indefinitely, breaking the "at most one `HOVER_HIDE_GRACE_MS`
   window" invariant this spec's own §3 edge-case table claimed ("typing
   alone does not extend the hide window"). **Fix:** gate the trigger on
   `!hidden()` — only the transition *into* the paused state (re)arms the
   window; further qualifying events while already hidden are no-ops. This
   also revises §7's "second qualifying keydown mid-window restarts the
   window" test expectation, which described the exact behavior being fixed
   here — the corrected expectation is the opposite: a second keydown
   *mid-window* does **not** restart it; only a keydown *after* the window
   has actually resumed does.
2. **Codex P2 — minimized chip not excluded.** §2.2 says the minimized chip
   is excluded "for the identical reason" as the mouse trigger, but that
   exclusion isn't automatic for keyboard: while minimized, `rootRef` is
   reassigned to the minimized `<button>`, so a keydown/focus landing
   directly on that focusable button read as `inPanel = true`. **Fix:**
   also gate the trigger on `!minimized()`.
3. **Codex P2 — Tab-into-panel from outside doesn't pause.** A real browser
   moves focus only *after* a `Tab` keydown's default action runs, so the
   keydown that causes a keyboard-only user's first entry into the panel has
   `e.target` still pointed at whatever was focused before — outside the
   panel. `handleKey`'s target-at-dispatch-time `inPanel` check necessarily
   misses that one event. **Fix:** a second, separate `focusin` listener
   (which bubbles and fires once focus has actually landed) reusing the same
   gated check, registered in the same effect as the `keydown` listener.

All three fixes share one helper, `maybePauseFor(target)` — computes
`inPanel` and gates on `!minimized() && !hidden()` — called from both
`handleKey` and the new `handleFocusIn`. §2.3's "extend `handleKey`, don't
add a second listener" decision is narrowed by fix 3: it still holds for a
second *keydown* listener, but a `focusin` listener addresses a distinct
browser-timing gap `keydown` structurally cannot catch.
