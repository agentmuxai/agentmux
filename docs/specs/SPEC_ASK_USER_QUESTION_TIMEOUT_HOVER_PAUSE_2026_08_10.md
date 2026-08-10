# SPEC: Hover-pause for the AskUserQuestion auto-timeout countdown

**Date:** 2026-08-10
**Status:** implemented — see §9 for a TDD-driven revision made during
implementation, before this PR opened.
**Builds on:** `docs/specs/SPEC_ASK_USER_QUESTION_AUTO_TIMEOUT_2026_08_06.md`
(implemented, PR #2441, verified in code 2026-08-10). This spec amends only
that spec's *timer-arming* logic (§2.3). Its recommended-option detection
(§2.2), merge-at-zero semantics (§2.3's merge rule, §2.5), audit trail
(`autoFilledCount`), and all backend/wire behavior are unchanged — see §2.
**Reuses:** `docs/specs/SPEC_SWARM_ROW_AUTO_LINGER_COUNTDOWN_2026_08_06.md`'s
already-resolved "resume fresh, not mid-count" call (§5 point 2) instead of
re-deriving one, per the DRY ask.

---

## 0. Ask

> get latest agentmuxai/agentmux .. in the AskQuestion tool we recently added
> a 30s timeout .. we want to refine it such that if there is any mouse hover
> for typing into that pane, the timer disappears for 15s (show nothing) ..
> if it goes back to idle and the user never submitted, after the 15s go
> back to the normal retrigger the 30s countdown and the same applies
> recursively. audit the state, write DRY spec to file

---

## 1. Current behavior (audited against source, 2026-08-10)

`frontend/app/view/agent/components/AgentQuestionPanel.tsx`, confirmed
against the live file (matches its own spec exactly, no drift):

- `AUTO_TIMEOUT_MS = 30_000` (line 31), hardcoded per the original spec's
  §5.2 non-goal (no settings toggle).
- **Reset effect** (lines 111-117): keyed on `request()?.tool_use_id`,
  re-runs on every new question-set — clears `minimized`, rebuilds `state()`
  fresh, resets `remainingMs` to `AUTO_TIMEOUT_MS`.
- **Timer effect** (lines 243-259): a second `createEffect`, also keyed on
  `tool_use_id`. Starts one `setInterval(1000ms)` that decrements
  `remainingMs`; at zero, clears itself and calls
  `submit(applyRecommendedDefaults())`. `onCleanup` clears the interval on
  unmount or `tool_use_id` change — no other teardown path exists.
- **Countdown rendering**: `.agent-question-panel-countdown` appears twice —
  in the full panel's header (lines 371-379, `margin-left: auto`-pushed to
  the row's right edge) and in the minimized chip (lines 343-351). Severity
  bands (`countdownSeverity()`, lines 322-327): default `>10s`, warning
  `≤10s`, critical `≤5s` (pulse animation) — SCSS in
  `AgentQuestionPanel.scss` lines 62-78.
- **No hover or interaction listeners exist anywhere in this file today.**
  The timer fires unconditionally at zero regardless of mouse activity — a
  deliberate choice, not an oversight: the original spec's §5.1 explicitly
  rejected disarming on interaction, because a *permanent* one-shot disarm
  (triggered by, say, answering question 1 of 2) would cancel the safety net
  entirely if the user then walked away, leaving question 2 blocked forever
  — reproducing the exact bug the feature exists to fix. §2 below explains
  why this spec's pause mechanism doesn't reopen that failure mode.

**Closest existing precedent for the requested behavior**, found via audit
rather than designed fresh (per the DRY ask):
`docs/specs/SPEC_SWARM_ROW_AUTO_LINGER_COUNTDOWN_2026_08_06.md` gives a Swarm
row's own 60s auto-retire countdown the same shape being asked for here —
"Hovering/interacting with a row pauses its countdown... Resume the
countdown (fresh 60s, not resumed mid-count) on mouse-leave" (§5 point 2 of
that spec). That spec freezes the *displayed number* on hover rather than
removing it, and resumes immediately on mouse-leave with no extra delay —
this spec's ask is stronger ("show nothing") and adds a 15s window; §3
covers the design, and §9 covers a real difference in *what "pause" safely
means* that surfaced only once this spec was actually implemented and
tested against the Swarm spec's own "resume fresh, not mid-count" call,
which is reused verbatim (§5 point 2 below).

`frontend/app/hook/useTick.ts` exists as a shared, ref-counted `setInterval`
primitive and is what the Swarm spec migrated its own countdown onto — noted
for completeness, but `AgentQuestionPanel`'s existing raw `setInterval` isn't
being touched by this spec; see §6.

---

## 2. Relationship to the original spec's "no disarm" decision

This is the one point worth being explicit about before the design, because
it looks at first glance like this spec reopens a question the original
spec already closed (§5.1: "resolved: merge, no disarm").

**It doesn't, because the two mechanisms are different in kind, not degree:**

- The rejected design was a *permanent, one-shot* disarm — triggered once,
  by any single interaction, with no path back to an armed state. That's
  what made it dangerous: it could cancel the whole safety net for a
  question a human touched once and then never returned to.
- This spec's pause is *live but strictly bounded* — every hover starts a
  flat, non-extending window (§3, §9) after which the countdown
  unconditionally resumes with a fresh deadline, regardless of whether the
  mouse is still over the panel. There is no state in this design where the
  countdown can be suppressed for longer than one `HOVER_HIDE_GRACE_MS`
  window per hover-triggering event.

Worst case for "how long can a human postpone the safety net": one
`HOVER_HIDE_GRACE_MS` window (15s) per fresh hover — and a "fresh hover"
requires the mouse to have actually left and come back, not just sat there.
The moment a human stops re-entering the panel — whether they left
deliberately or their cursor is simply parked motionless over it after a
click — the countdown resumes within that same bound and then runs its
normal, unconditional 30s. §9 explains why this flat-bound design (rather
than an open-ended "hidden for as long as hovered") is what actually
delivers on that guarantee.

**Unchanged by this spec:** what happens when the countdown reaches
zero — `applyRecommendedDefaults()`'s per-question merge, `submit()`,
`buildOutcome()`, `autoFilledCount`, the transcript annotation in
`useAgentQuestions.ts`. This spec only changes *whether the countdown is
currently running*, never what it does when it finishes.

---

## 3. Design

### 3.1 Scope of "hover"

**Decision: the entire expanded panel** (`.agent-question-panel` root, lines
358-364), not narrowly scoped to the "Other" free-text `<input>`
(lines 426-433).

Considered and rejected: scoping hover detection to just the "Other" input
(the literal reading of "hover for typing"). Rejected because most answers
in this panel are given by clicking a radio/checkbox option
(`.agent-question-panel-option`, lines 400-421), not by typing — a user
moving their pointer toward an option to click it is exactly as "present and
about to act" as one hovering the text field, and scoping this narrowly
would mean the feature rarely fires in the actual common case. Root-level
`mouseenter` (which, unlike `mouseover`, doesn't rebubble per descendant, so
no debouncing is needed for moving between child elements) is also the
simpler implementation — one handler, not one per interactive child.

**Excluded: the minimized chip** (`.agent-question-panel-minimized`, lines
334-354). There's no typing surface there, and the original spec is explicit
that the timer "keeps running while minimized" regardless of interaction —
this spec doesn't touch that guarantee. Calling `defer()` (minimize) while
hidden forcibly clears the hide state (§3.3) so the timer reverts to the
original, unconditional behavior the instant the panel is minimized.

### 3.2 State machine

Two phases per question-set (i.e., per `tool_use_id`, same scope as the
existing timer) — see §9 for why this is two phases, not the three-phase
Counting/Hidden-hovering/Hidden-grace model originally drafted:

| Phase | Countdown UI | Underlying deadline | Entered when |
|---|---|---|---|
| **Counting** (default) | Visible, ticking | Armed, ticking toward 0 | Question-set becomes head; or the hide window (below) elapses |
| **Hidden** | Nothing rendered | Paused (not armed) | Mouse enters the panel |

Transitions:
- **Counting → Hidden**: mouse enters the panel (`mouseenter`). Immediate —
  no debounce on entry, matching the ask's plain "if there is any mouse
  hover." Starts a flat `HOVER_HIDE_GRACE_MS` (15s) timer, timed from this
  entry.
- **Hidden → Counting**: the 15s window elapses — **unconditionally**,
  regardless of whether the mouse is still over the panel or has left. The
  countdown UI reappears and the underlying deadline **resets to a full
  30s** (not resumed from wherever it was paused) — see §5 point 2 for why
  "fresh, not resumed" is the resolution, reusing the Swarm spec's identical
  call rather than re-deciding it.
- **Hidden → Hidden**: a *fresh* `mouseenter` (the mouse genuinely left the
  panel and came back — `mouseenter` only fires on a real boundary
  crossing, never while the pointer stays inside) restarts the 15s window
  from that new entry. This is the "recursively" behavior from the ask —
  every fresh hover independently gets its own window, with no
  special-casing for "the second time" and no cap on how many times it can
  repeat.
- **Any phase → (submitted)**: a manual `Submit answer` click, or Enter,
  wins immediately regardless of phase — unchanged from the original spec's
  existing "manual submit tears down the interval via the `tool_use_id`-keyed
  cleanup" behavior (§2.3 of the original). Nothing here needs new handling
  for this case; there is no interval running to race against while hidden.

### 3.3 Implementation sketch

New constant, colocated with `AUTO_TIMEOUT_MS`:

```ts
/** How long the countdown stays hidden after a hover into the panel, before
 *  an unanswered question-set's timer resumes (fresh AUTO_TIMEOUT_MS, not
 *  resumed from wherever it was paused) — a flat window timed from the
 *  triggering mouseenter, unconditional regardless of whether the mouse is
 *  still over the panel when it elapses. See §9 for why unconditional. */
const HOVER_HIDE_GRACE_MS = 15_000;
```

New signal, alongside `remainingMs`:

```ts
/** True while the countdown is suppressed by a recent hover. Drives both
 *  the UI (§3.4) and whether the timer effect below is armed. Bounded, NOT
 *  tied to "is the mouse currently over the panel" — see §9. */
const [hidden, setHidden] = createSignal(false);
let hideTimeoutId: ReturnType<typeof setTimeout> | undefined;

const clearHideTimer = () => {
    if (hideTimeoutId !== undefined) {
        clearTimeout(hideTimeoutId);
        hideTimeoutId = undefined;
    }
};

const onPanelPointerEnter = () => {
    clearHideTimer();
    setHidden(true);
    hideTimeoutId = setTimeout(() => {
        hideTimeoutId = undefined;
        setHidden(false); // → timer effect re-arms at a fresh AUTO_TIMEOUT_MS, §3.2
    }, HOVER_HIDE_GRACE_MS);
};
```

Wire `onPanelPointerEnter` as `onMouseEnter` on the root
`<div class="agent-question-panel">` (line 358) — same prop-naming
convention already used for `NotificationItem`'s `onMouseEnter`/
`onMouseLeave` (`frontend/app/notification/notificationitem.tsx`), though
this spec only needs the enter half (§9). `clearHideTimer` also needs a
top-level `onCleanup` (it's a raw `setTimeout`, not owned by a reactive
effect, so it must be disposed on unmount independently).

**Reset effect** (lines 111-117): add `clearHideTimer(); setHidden(false);`
alongside the existing resets, so a brand-new question-set never inherits
hidden state left over from the previous one's hover history.

**Timer effect** (lines 243-259): extend its reactivity to also read
`hidden()`, and skip arming when hidden — this is the minimal-diff way to
implement "pause," reusing the exact same effect-teardown-on-dependency-
change shape the file already has for `tool_use_id`, not a new mechanism:

```ts
createEffect(() => {
    const r = request();
    void r?.tool_use_id;
    if (!r || hidden()) return; // paused while hidden; re-arms when hidden() flips false

    setRemainingMs(AUTO_TIMEOUT_MS); // fresh retrigger, not resumed from wherever it was paused
    const intervalId = setInterval(() => {
        setRemainingMs((prev) => {
            if (prev <= 1000) {
                clearInterval(intervalId);
                submit(applyRecommendedDefaults());
                return 0;
            }
            return prev - 1000;
        });
    }, 1000);
    onCleanup(() => clearInterval(intervalId));
});
```

Because `createEffect` naturally re-runs and tears down its own
`onCleanup` whenever a read dependency (`hidden()`) changes, entering the
Hidden phase automatically clears the running interval with no extra code —
the same cleanup path that already handles `tool_use_id` changes and
unmount.

**`defer()` (lines 261-264)**: add `clearHideTimer(); setHidden(false);`
before `setMinimized(true)`, per §3.1 — minimizing always returns the timer
to its original, hover-independent behavior.

### 3.4 UI rendering

Wrap the header's countdown span (lines 371-379) in
`<Show when={!hidden()}>` — this renders nothing at all when hidden, per the
ask's literal "show nothing," not a CSS opacity/visibility toggle. The
minimized chip's countdown (lines 343-351) is untouched — §3.1 excludes it
from hover-pause entirely, and it already shows unconditionally today.

No change to `.agent-question-panel-option--recommended` highlighting
(SCSS lines 145-148) — it's driven by `state()`/`recommendedOptions()`, not
the timer, and stays visible throughout so a user comparing options mid-hover
can still see which one the timer would pick once it resumes.

Minor cosmetic note, not requiring extra work: because the countdown chip is
`margin-left: auto`-positioned at the header's right edge (SCSS line 58-60),
removing it via `<Show>` will shrink that row slightly when hidden and it's
the only right-aligned element present (the `+N more` queue chip, when
present, stays anchored the same way regardless). Acceptable per the ask's
own "show nothing," not a bug.

---

## 4. Edge cases

| Case | Behavior |
|---|---|
| Mouse enters, leaves well before 15s, never re-enters, never submits | Countdown resumes visible at the 15s mark (from entry), reset to a full 30s |
| Mouse enters and never leaves at all (cursor parked over the panel) | Countdown STILL resumes at the 15s mark regardless — no indefinite hide; see §9 for why this specific case is load-bearing |
| Mouse enters, then enters again (left and came back) at 10s into the window | Window restarts from that second entry — resumes 15s after the *second* entry, not the first |
| Mouse enters, user submits manually while hidden | Normal `submit()` path — nothing timer-related needed, since no interval is running to race against |
| Mouse enters mid-countdown (e.g. at 12s remaining) | Deadline resets to a full 30s once the 15s window elapses — the 12s that had already elapsed before the hover is not preserved; same "fresh, not resumed" call the Swarm spec made for its own countdown (§5 point 2) |
| New question-set arrives (queue advances) while a prior one was hidden | Reset effect (§3.3) unconditionally clears `hidden`/hide-timer for the new `tool_use_id` — the new head always starts in Counting, regardless of the previous question-set's hover state |
| User clicks "Answer later" (minimize) while hidden | `defer()` forces `hidden(false)` immediately — timer resumes exactly as the original spec describes for a minimized panel, unaffected by hover history |
| Mouse hovers the *minimized* chip | No effect — §3.1 scopes hover-pause to the expanded panel only; the minimized chip's countdown (if any time remains) keeps ticking as it does today |
| User clicks an option with the mouse (the common case) | Clicking requires the pointer to already be over the target, so this fires `mouseenter` on the panel exactly like a deliberate hover — hides for 15s, then resumes at a fresh 30s, same as any other hover trigger. See §9. |

---

## 5. Resolved design decisions

1. **Hover scope: whole panel, not just the "Other" input — resolved.**
   §3.1. The literal "for typing" reading was considered and rejected: most
   answers are clicks, not typing, and scoping this narrowly would rarely
   trigger in practice. (§9 covers a consequence of this choice caught
   during implementation, not a reversal of it.)
2. **Resume fresh 30s vs. resume from remaining time — resolved: fresh.**
   Reused directly from `SPEC_SWARM_ROW_AUTO_LINGER_COUNTDOWN_2026_08_06.md`
   §5's identical resolution for its own hover-pause countdown ("Resume the
   countdown (fresh 60s, not resumed mid-count)") — same class of problem,
   same answer, not re-derived. Simpler to reason about than persisting
   elapsed time across a pause, and it's what "retrigger the 30s countdown"
   in the ask most naturally means (a retrigger, not a resume).
3. **Does the underlying deadline pause, or only the visible chip? —
   resolved: both, together.** A UI-only hide with the deadline still
   silently ticking underneath would make "retrigger the 30s countdown"
   incoherent (there'd be no full 30s left to retrigger). §3.3 pauses the
   same effect that arms the interval, so hiding and pausing are one
   mechanism, not two things that could drift out of sync.
4. **Hide window bound to hover duration, or a flat window from entry? —
   resolved during implementation: flat window from entry, unconditional.**
   See §9 for the full account — this is a revision from what was
   originally drafted here (open-ended "hidden for as long as hovered, plus
   a post-leave grace"), caught by a failing pre-existing test before this
   spec's implementation ever reached a PR.
5. **Minimized-panel interaction — resolved: hover-pause is a full-panel-
   only affordance.** §3.1, §3.3. Minimizing always forces an immediate
   resume, preserving the original spec's "keeps running while minimized"
   guarantee unchanged — this spec adds a new way to *pause* the timer, not
   a new way to make it stop running altogether.

---

## 6. Non-goals

- No change to recommended-option detection, merge-at-zero semantics,
  `autoFilledCount`, or answer delivery — all untouched (§2).
- No transcript/audit-trail marker for "this question's timer was extended
  by hovering" — `useAgentQuestions.ts`'s existing three-band summary
  (§2.5 of the original spec) is unaffected. Reasonable follow-up if ever
  wanted; not built here.
- No configurability of either duration (`AUTO_TIMEOUT_MS` or the new
  `HOVER_HIDE_GRACE_MS`) — both are plain constants, matching the original
  spec's own "hardcoded, no settings toggle" non-goal (§5.2 there).
- `AgentDecisionPanel` remains explicitly out of scope, same as the
  original spec (§6 there) — different tool, different risk profile.
- No migration of the existing raw `setInterval` to the shared `useTick`
  hook (`frontend/app/hook/useTick.ts`) that the Swarm countdown uses —
  orthogonal refactor, not needed to implement this feature, not requested.

---

## 7. Files touched

- `frontend/app/view/agent/components/AgentQuestionPanel.tsx` — `hidden`
  signal + hide-timeout handling, extended reset effect, extended
  timer-arm effect (gated on `hidden()`), `onMouseEnter` on the panel root,
  `defer()` forces an unhide, countdown span wrapped in
  `<Show when={!hidden()}>`.
- `frontend/app/view/agent/components/AgentQuestionPanel.test.tsx` — new
  test cases per §8.
- No `.scss` changes required — `<Show>` removes the element outright, no
  new class/state needed for the hidden case itself.
- No `agentmux-srv` (Rust) changes. No `agentmux-common` changes. No wire
  format changes — same as the original spec, this stays entirely
  frontend-local.

---

## 8. Test plan

**Unit** (`AgentQuestionPanel.test.tsx`, extending the existing
`vi.useFakeTimers()` conventions from the original spec's own test suite):

- Fully idle panel, no hover: countdown fires at exactly 30s, unchanged
  from today (regression guard — this spec must not change the no-hover
  path at all).
- Mouse enters at t=5s (25s remaining): countdown UI disappears immediately.
- **Regression guard for §9:** mouse enters and never leaves for the rest
  of the test — countdown still resumes exactly 15s after entry (fresh
  30s) and auto-submits 30s after that, with no `mouseleave` fired
  anywhere in the test. This is the scenario that caught the original,
  over-broad design (§9) and must stay pinned.
- A fresh `mouseenter` fired again partway through an existing hide window
  restarts a new 15s window from that point (the "recursive" case) —
  confirm the countdown reappears at the *second* entry's 15s mark, not the
  first's.
- Manual submit while hidden: submits immediately, no leftover timer fires
  afterward.
- New question-set arrives while a prior one was mid-hover: new head starts
  with the countdown visible and running immediately (Counting phase),
  regardless of the old `tool_use_id`'s hover state.
- `defer()` called while hidden: countdown resumes ticking in the
  minimized chip exactly as if hover had never happened.
- Existing "merges: keeps a manually-answered question..." test (predates
  this spec): update its timing to account for the click-triggered
  `mouseenter` — total time-to-auto-submit is now
  `HOVER_HIDE_GRACE_MS + AUTO_TIMEOUT_MS` (15s + 30s) after the click, not
  the original bare 30s.

**Manual / integration:**

- `task dev`, trigger a live `AskUserQuestion`, hover the panel while
  reading the options. Confirm the countdown chip disappears immediately.
- Move the mouse away and don't touch anything for the full 15s. Confirm
  the countdown reappears showing 30s (not wherever it left off) and
  proceeds to auto-submit at 0 if still untouched.
- Click an option with the mouse, then leave the cursor sitting exactly
  where it is (don't touch the mouse again). Confirm the countdown still
  reappears 15s later and the question still auto-submits on schedule —
  this is the real-world version of §9's regression guard, and is the
  scenario that matters most: a user who answers by mouse click and then
  steps away must not have the safety net silently disabled.
- Hover, let it resume, hover again mid-countdown — confirm the cycle
  repeats cleanly an arbitrary number of times with no leaked timers (watch
  for console warnings / growing interval counts across repeated cycles).

---

## 9. Revision made during implementation (TDD-driven, before this PR opened)

The design originally drafted in §3.2/§3.3 (now superseded by the version
above) kept the countdown hidden for **as long as the mouse remained over
the panel**, with the 15s window applying only *after* `mouseleave` — a
three-phase model (Counting / Hidden-hovering / Hidden-grace). While
implementing it, the pre-existing test `"merges: keeps a manually-answered
question and only auto-fills the untouched one"` started failing:

```
expected "spy" to be called 1 times, but got 0 times
```

That test clicks a radio option, then advances fake time by 30s and expects
an auto-submit. Root cause, confirmed with a minimal repro before touching
the fix: `userEvent.click()` on any element fires a real `mouseenter` on
its ancestors on the way to the click — because the pointer has to already
be over an element to click it, in a real browser exactly as much as in
this test harness. Under the open-ended design, that `mouseenter` hid the
countdown and never un-hid it, because nothing in the test (or, critically,
in a realistic *human* workflow) ever fires `mouseleave` afterward — a user
who answers by clicking with their mouse and then steps away from their
desk typically leaves the cursor exactly where they last clicked, over the
panel. **That reintroduces the exact "work does not stop" failure mode
`SPEC_ASK_USER_QUESTION_AUTO_TIMEOUT_2026_08_06.md` §5.1 rejected** — not
through a permanent one-shot disarm this time, but through an unbounded
pause triggered by the single most common way to answer a question.

Fix: collapse the three-phase model to two phases (§3.2) and make the hide
window a **flat, non-extending 15s timed from the triggering `mouseenter`**,
unconditional on whether the mouse is still there when it elapses. This
closes the gap entirely — worst case is bounded at exactly
`HOVER_HIDE_GRACE_MS` per fresh hover, never indefinite — while still
satisfying the ask's literal wording ("the timer disappears for 15s") and
the "recursively" requirement (a genuinely fresh `mouseenter`, i.e. the
mouse actually leaving and coming back, still restarts the window). §2's
"live but strictly bounded" framing was updated to describe this version,
not the original draft.

This is documented here, not silently fixed, because it changes a claim
made earlier in this same document (§2 originally argued the pause was safe
because "the mouse continuously being over the panel" was itself bounded by
human behavior — that assumption was wrong, since a stationary cursor after
a click is not the same as continued active engagement) and because it's
exactly the kind of regression a reviewer diffing this PR against the
implementation would otherwise have to reconstruct independently.
