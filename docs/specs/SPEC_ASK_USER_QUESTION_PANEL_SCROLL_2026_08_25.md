# SPEC: Scrolling for the AskUserQuestion panel

**Date:** 2026-08-25
**Status:** implemented, PR #2805 — see §3.1's two corrections (Codex P1/P2)
made during that PR's review, before merge.
**Builds on:** `docs/specs/SPEC_ASK_USER_QUESTION_2026_06_15.md` (original
design), `SPEC_ASK_USER_QUESTION_AUTO_TIMEOUT_2026_08_06.md` /
`SPEC_ASK_USER_QUESTION_TIMEOUT_HOVER_PAUSE_2026_08_10.md` /
`SPEC_ASK_USER_QUESTION_TIMEOUT_KEYBOARD_PAUSE_2026_08_20.md` (the 30s
auto-timeout countdown this spec must keep reachable — see §2). Does not
touch `AnsweredQuestionMessage.tsx` (the post-answer history rendering,
governed separately by `SPEC_ASK_USER_QUESTION_HISTORY_STYLING_2026_08_17.md`)
— out of scope, see §4.

---

## 0. Ask

> back into agentmuxai/agentmux .. pull in latest, we need to be able to
> support scrolling in the ask part of the askquestion tool for the agent
> pane .. write spec to file

## 1. Current behavior (audited against source, 2026-08-25)

`frontend/app/view/agent/components/AgentQuestionPanel.tsx` renders the
live AskUserQuestion prompt (`AgentQuestionPanel`, JSX at lines 512-650),
mounted as a normal-flow sibling inside `.agent-view`
(`frontend/app/view/agent/agent-view.tsx:2273-2277`), directly below
`AgentDecisionPanel` and directly above the message queue / composer.

Structure (lines 541-645):
```
<div class="agent-question-panel">
  <div class="agent-question-panel-header">  <!-- title + countdown -->
  <For each={r.questions}>
    <fieldset class="agent-question-panel-q">  <!-- prompt + options + "Other" input -->
  </For>
  <div class="agent-question-panel-actions">  <!-- Answer later / Submit answer -->
</div>
```

`frontend/app/view/agent/components/AgentQuestionPanel.scss` has **no
`max-height`, `height`, or `overflow` rule anywhere** on `.agent-question-panel`
or any descendant — confirmed by reading the full 251-line file. It's a plain
`display: flex; flex-direction: column` (lines 19-31) that grows to fit
however much content `r.questions` produces: any number of questions, each
with any number of options, each option with an optional multi-line
description.

The parent, `.agent-view`, is `display: flex; flex-direction: column;
height: 100%; overflow: hidden` (`agent-view.scss:305-317`). Its children —
the scrollable conversation feed, `AgentDecisionPanel`, `AgentQuestionPanel`,
the message queue, the composer — all compete for that one fixed height in
normal flow. `AgentQuestionPanel` has no `flex-shrink: 0` guard and no cap,
so a large enough question set can grow past the pane's remaining height.
Because the parent clips overflow, there is nothing to scroll to reach the
part that got pushed out — which, since the buttons are the *last* child,
is usually the **Submit answer / Answer later buttons themselves**, along
with the composer and message queue beneath the panel.

This is not hypothetical: a single `AskUserQuestion` call frequently carries
2-4 questions at once, each with 2-4 options plus per-option descriptions
plus a free-text "Other" field — comfortably enough content to exceed a
short or split agent pane's available height.

**Established precedent for the fix already exists in a sibling panel**:
`AgentDecisionPanel`'s tool-arg preview
(`frontend/app/view/agent/styles/_decision-panel.scss:150-167`,
`.agent-decision-panel-preview`) already uses `max-height: 160px;
overflow-y: auto;` for exactly this reason, just on a much smaller content
block. This spec applies the same idea to the whole panel, with header/footer
handled specially (see §3).

## 2. Why this is more than cosmetic

`AgentQuestionPanel` runs a 30-second auto-timeout countdown
(`SPEC_ASK_USER_QUESTION_AUTO_TIMEOUT_2026_08_06.md`) that auto-selects the
recommended option(s) and submits if the user doesn't act. If the Submit/
Answer-later buttons are scrolled out of reach by an overflowing panel, the
user can still see the question and pick options, but may not be able to
reach the button to submit *early* or defer — they're stuck waiting for the
auto-timeout to fire on their behalf, on a decision they may not have
finished making. A panel that overflows must not be allowed to hide the
controls that let the user act on it.

## 3. Design

### 3.1 New scroll boundary: header and footer fixed-size, question content scrolls

Wrap the `<For each={r.questions}>` block in a new element,
`.agent-question-panel-scroll`:

```
<div class="agent-question-panel">      <!-- max-height cap + min-height: 0, flex column -->
  <div class="agent-question-panel-header">   <!-- flex-shrink: 0 -->
  <div class="agent-question-panel-scroll">   <!-- NEW: overflow-y: auto, min-height: 0 -->
    <For each={r.questions}>
      <fieldset class="agent-question-panel-q">
    </For>
  </div>
  <div class="agent-question-panel-actions">  <!-- flex-shrink: 0 -->
</div>
```

This keeps the countdown (in the header) and the Submit/Answer-later buttons
always visible regardless of how much question content there is — the thing
that actually matters per §2 — while only the middle (the questions
themselves) scrolls.

**Correction (Codex P2 on the implementation PR #2805 — `position: sticky` on
the header/actions was inert, not just unnecessary):** the first draft of
this design put `position: sticky; top: 0` on the header and `position:
sticky; bottom: 0` on the actions bar, claiming it mirrored
`frontend/app/view/agent/styles/_search.scss`'s header-pinning pattern.
That claim was wrong, and the CSS did nothing as a result: `position: sticky`
only sticks an element within its nearest *scrolling* ancestor. The header
and actions are siblings of `.agent-question-panel-scroll` (the actual
scroll container), not nested inside it, and `.agent-question-panel` itself
has no `overflow` set — there is no scrolling ancestor for sticky to attach
to here. `_search.scss`'s header, by contrast, genuinely is nested inside
its own scrolling container, which is why sticky works there and doesn't
here — the two cases only look alike, they aren't the same shape.

The actual mechanism that keeps the header/actions visible is plain flex
sizing, not positioning: give the header and actions `flex-shrink: 0` so
they never give up space, and leave `.agent-question-panel-scroll` as the
one flexible, shrinkable child (default `flex-shrink: 1`) that absorbs all
of the panel's own size changes. No `position: sticky` needed or used.

**Correction (Codex P1 on #2805 — `max-height` on `.agent-question-panel`
was also ineffective on its own):** the first draft set `max-height: 420px`
on `.agent-question-panel` without also setting `min-height: 0` on it. As a
flex item of `.agent-view`, `.agent-question-panel`'s *automatic* minimum
height defaults to its content's min-content size (since the panel itself
has no `overflow` set) — and per the flex sizing algorithm, `used-size =
max(min-size, min(max-size, tentative-size))`, so that automatic minimum
wins over `max-height` whenever content is taller than 420px. Concretely:
the panel still grew to fit *all* its content regardless of the cap, and
`.agent-view`'s `overflow: hidden` clipped it exactly as before this spec —
the fix did nothing in the exact scenario it was built for. `min-height: 0`
on `.agent-question-panel` itself (not just on `.agent-question-panel-scroll`
— see §3.3, a *different* instance of the same underlying gotcha, one level
up) closes this: with it, the panel can actually shrink to whatever
`.agent-view` can spare, capped at 420px, instead of always claiming its
full content height.

CSS additions to `AgentQuestionPanel.scss`, corrected:

```scss
.agent-question-panel {
    // existing rules unchanged, plus:
    max-height: 420px; // see §3.2 for why a fixed px value, not vh/%
    min-height: 0;      // REQUIRED — see the Codex P1 correction above
}

.agent-question-panel-header {
    flex-shrink: 0; // never gives up space to the scroll region below
}

.agent-question-panel-scroll {
    display: flex;
    flex-direction: column;
    gap: var(--space-3); // was .agent-question-panel's own gap, between
                          // stacked <fieldset> blocks when multiple questions
    overflow-y: auto;
    min-height: 0; // REQUIRED — see §3.3, the classic flexbox scroll gotcha
                    // (a distinct instance of it from the panel-level one above)
}

.agent-question-panel-actions {
    padding-top: var(--space-2); // breathing room above this bar once the
                                   // scroll region above it is scrolled
    flex-shrink: 0; // never gives up space to the scroll region above
}
```

`.agent-question-panel`'s own `gap` (currently spacing header / questions /
actions as three flex children) still works exactly as before — it now
spaces header / scroll-wrapper / actions instead of header / N-question-
fieldsets / actions, with the new wrapper's own `gap` handling spacing
*between* multiple question fieldsets internally.

### 3.2 Fixed pixel cap, not `vh`/`%`

`.agent-view` has `height: 100%` (relative to whatever split/pane it's in,
not the OS window) and `container-type: inline-size` — **width**-based
container queries only; there is no height-based container query available
to size this panel as a fraction of its actual pane. A `vh` unit would be
relative to the full browser viewport, which is wrong the moment the agent
pane is one of several splits and is much shorter than the window itself —
`max-height: 50vh` could still exceed a short split pane's *entire* height,
defeating the purpose.

Use a fixed pixel cap instead, consistent with the existing
`.agent-decision-panel-preview` precedent (`max-height: 160px`) — just
larger, since this panel holds more than a single preview block. **420px**
is a starting estimate (roughly: ~30px header + 3-4 options at ~50px each +
~50px actions bar, enough to show a typical single question without
scrolling, while still capping a multi-question or many-option outlier) —
tune against real screenshots during implementation rather than treating it
as load-bearing; it's a UX tuning value, not a correctness one.

### 3.3 The flexbox min-height gotcha

`.agent-question-panel-scroll` **must** set `min-height: 0`. Flex items
default to `min-height: auto`, which means "at least as tall as my content"
— this silently defeats `overflow-y: auto` (the box still grows to fit
content instead of scrolling) even though `max-height` is set on an
ancestor. This is the single most common way this exact pattern fails
silently in a code review that "looks right." Call it out explicitly in the
PR/tests, not just in this doc.

### 3.4 Scrollbar styling

No new work needed: `*::-webkit-scrollbar-track` / `*::-webkit-scrollbar-thumb`
/ `*::-webkit-scrollbar-thumb:hover` (`frontend/app/app.scss:104-116`) are
already global wildcard selectors — any new scrollable element, including
`.agent-question-panel-scroll`, automatically inherits the app's custom
scrollbar coloring with no extra rule. Only *width* is ever customized
per-element (e.g. `.agent-document::-webkit-scrollbar`); leave width at
default here unless a real visual mismatch shows up in review — this is a
much narrower panel than the full conversation view.

### 3.5 Minimized-chip mode is unaffected

`.agent-question-panel-minimized` (the collapsed "Question waiting" chip,
`AgentQuestionPanel.tsx:517-536`) is a completely separate render branch
with its own tiny fixed layout — it never grows with question content and
needs no changes here.

## 4. Non-goals

- **No settings toggle for the max-height.** Matches the existing
  `AUTO_TIMEOUT_MS` precedent (`SPEC_ASK_USER_QUESTION_2026_06_15.md` §5.2)
  of no per-behavior settings surface for this panel — one sensible fixed
  value, not a new knob.
- **`AnsweredQuestionMessage.tsx`** (the post-answer, non-interactive history
  rendering, `SPEC_ASK_USER_QUESTION_HISTORY_STYLING_2026_08_17.md`) is a
  separate component in the normal scrollable document flow already — it
  scrolls with the rest of the conversation and has no overflow problem of
  its own. Not touched by this spec.
- **No change to the 30s auto-timeout's own logic** (arming, hover-pause,
  keyboard-pause, recommended-option detection) — this spec only changes
  layout/overflow, not timing behavior. The countdown chip simply needs to
  stay visible in the header, which it already is (it's part of
  `.agent-question-panel-header`, unchanged, and the header is now
  `flex-shrink: 0` per §3.1's correction).
- **No virtualization.** Question/option counts in practice are small
  (single digits); a plain `overflow-y: auto` is sufficient and matches the
  weight of the existing `_decision-panel.scss` precedent. Virtualizing a
  handful of DOM nodes would be solving a problem that doesn't exist here.

## 5. Testing

- A single short question (1 question, 2-3 options, no long descriptions)
  renders identically to today — no visible scrollbar, no layout shift, no
  regression for the common case.
- A question set long enough to exceed 420px (e.g. 2 questions × 4 options
  each, each option with a 2-line description) scrolls internally; the
  header (with countdown, if a countdown is active) and the Submit/Answer-
  later buttons remain visible and clickable throughout, never scrolled out
  of view.
- Scrolling the question content does not scroll or otherwise affect the
  conversation feed above it, or the queue/composer below it — the new
  scroll region is fully contained.
- Keyboard navigation (Tab through options, arrow keys within a radio
  group) still reaches every option, scrolling the container into view as
  needed, and never gets trapped — verify focus doesn't silently land on an
  option currently scrolled out of the visible region without the browser
  auto-scrolling it into view (native behavior for focus, should work for
  free, but confirm rather than assume).
- The minimized chip (`.agent-question-panel-minimized`) is visually
  unchanged.
- Multi-select questions (checkboxes, `q.multiSelect === true`) behave
  identically to today when scrolled — checking/unchecking an option that's
  only reachable after scrolling works the same as one visible without
  scrolling.
