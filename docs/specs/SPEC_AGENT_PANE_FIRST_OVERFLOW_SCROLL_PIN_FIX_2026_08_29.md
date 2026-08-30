# Spec: force stick-to-bottom on an agent pane's first-ever overflow

**Date:** 2026-08-29
**Status:** Implemented in PR #2834 — §5.1's design below reflects the
*first* implementation pass; see the addendum immediately below for three
corrections found in code review before merge.
**Verified against:** `main` @ `4573d0d34`.
**Related:** `docs/specs/REPORT_AGENT_PANE_SCROLL_PIN_FLICKER_AUDIT_2026_07_30.md`,
`docs/specs/PLAN_AGENT_PANE_RESIZE_SCROLL_PIN_2026_08_05.md`,
`docs/analysis/ANALYSIS_TOOL_CALL_SCROLL_OSCILLATION_2026_08_17.md`,
`docs/analysis/FINDINGS_TOOL_CALL_SCROLL_OSCILLATION_2026_08_21.md`,
`docs/analysis/FINDINGS_TOOL_CALL_SCROLL_OSCILLATION_LIVE_INSTANCE_DATA_2026_08_22.md`.

## Addendum 2026-08-29 (same day) — three corrections from code review, before merge

Codex and reagent's PR #2834 reviews found three real gaps in the first
implementation pass below. All three are fixed in the shipped commit;
noted here rather than silently rewritten into §5.1 so the reasoning that
led to the final design stays visible:

1. **(codex P1) Missing early return let pagination clobber the forced
   pin.** §5.1 as originally written didn't `return` after forcing the
   pin, so execution fell through into the older-history pagination
   block (`handleScrollNow`'s last statement), which reads the *stale*
   `scrollTop` captured at the top of the event — still near-top at the
   exact moment this fix's own force-pin fires. With `props.onLoadOlder`
   always supplied in the real pane (`agent-view.tsx`), that stale
   near-top reading triggered `captureHeadAnchor()`, flipping
   `stickToBottom` back to `false` in the same tick the fix had just set
   it `true`. Fixed with an explicit `return` right after the forced
   scroll.
2. **(reagent P1) The force-pin didn't check whether stickToBottom was
   *already*, legitimately, `false`.** §4's invariant — "a pane that's
   never overflowed can't have a legitimate scrolled-away state" — is
   true for a real user scroll gesture, but not for
   `captureHeadAnchor()` itself: older-history pagination can fire (and
   disengage stickToBottom) on a pane that hasn't overflowed yet, because
   a non-overflowing pane's `scrollTop` is always `0`, which always
   satisfies `isNearTop`. If that pagination's *own* anchor-restore
   `scrollTo()` (which never routes through `scrollToTrueBottom`) is what
   pushes the pane past its first overflow, the original code would force
   stickToBottom back to `true` and snap to the very bottom, destroying
   the reader's just-restored position. Fixed by gating the force-pin on
   `stickToBottom()` already being `true` at the moment of transition —
   it only ever *protects* an already-following pane, never *re-engages*
   one that's disengaged for any reason, pagination included.
3. **(codex P2) The latch never re-armed.** §5.1's `hasOverflowedOnce`
   was written as a one-way, mount-lifetime latch. But the whole reason
   §2 connects this bug to the still-open whole-pane `scrollHeight →
   0px` collapse is that a still-following pane living through that
   collapse-and-regrow re-lives the exact same transition — and a
   one-way latch only protects it the *first* time that ever happens,
   not every time it recurs. Renamed to a bidirectional `isOverflowing`
   signal (`markOverflowing` / `markNotOverflowing`, the latter called
   from `scrollToTrueBottom` whenever content resizes back down to
   non-overflowing) so each collapse-and-regrow gets independently
   re-armed.

A fourth, narrower finding (reagent P2) — the `[wave-scroll-first-overflow]`
diagnostic logging unconditionally on every detected transition, including
ones where the raw geometry already read near-bottom and no protection was
actually needed — is also fixed (log/act only when the fix changes the
outcome), but doesn't affect §4's reasoning or invalidate anything above
the addendum; noted for completeness since it's the same review round.

## Addendum 2 2026-08-29 (same day) — point 3 above was itself incomplete

Reagent's re-review (after the first addendum's fixes shipped) found that
point 3's own fix didn't actually deliver what it claimed. `markNotOverflowing`
was reachable ONLY through `scrollToTrueBottom()`, and every content-resize
call site of `scrollToTrueBottom()` (both `ResizeObserver`s, the itemized
signal effect) is itself gated on `stickToBottom()` already being `true`.
So a pane that collapses to non-overflowing WHILE scrolled away — the
`/clear`-while-reading-history case the doc comment explicitly claimed to
cover, or the whole-pane 0px collapse happening mid-history-read — never
re-armed `isOverflowing` at all, because nothing that could observe the
collapse ever ran while disengaged. The re-arm only actually worked for a
pane that stayed pinned throughout, which is the one case that's *least*
in need of protection (a still-pinned pane's normal re-pin machinery
already keeps it correct).

Fixed by extracting a `syncOverflowState()` helper that updates
`isOverflowing` from live geometry unconditionally, called from every
geometry-observation point — every `handleScrollNow` invocation, both
`ResizeObserver` callbacks, and the itemized effect — deliberately BEFORE
and independent of each site's own `stickToBottom()` gate on the actual
re-pin action. Point 3's text above ("the latter called from
scrollToTrueBottom whenever content resizes back down to non-overflowing")
describes the superseded first pass; the mechanism described in this
addendum is what actually shipped. Added a fifth test exercising the
collapse specifically while `stickToBottom()` is already `false` — the one
scenario neither of point 3's original two tests exercised.

## 1. Bug report

> If an agent pane loads without any content (no scrollbar yet), the first
> time it finally reaches a point where a scrollbar appears, the scroll
> doesn't stick to the latest content like it should — it sticks to the top
> instead. The conversation keeps streaming and the user doesn't see it.
> The user has to scroll manually or start typing before it catches up.

Rare (most panes open with at least one node already present, so this
specific transition never happens for them) but user-visible and annoying
when it does — new output silently accumulates off-screen with no
indication anything is wrong.

## 2. This is not a new investigation — where it fits

This repo already has an active, unresolved investigation thread into agent
pane scroll-pin fragility, spanning three prior documents:

- `REPORT_AGENT_PANE_SCROLL_PIN_FLICKER_AUDIT_2026_07_30.md` → shipped the
  content-`ResizeObserver` re-pin mechanism (PR #2370, referred to below as
  **RO #2**) that this spec builds directly on.
- `PLAN_AGENT_PANE_RESIZE_SCROLL_PIN_2026_08_05.md` → investigated a
  pane-splitter-resize variant, disproved its own leading hypothesis via a
  dedicated jsdom-fake test suite, closed with the JS logic confirmed
  correct for every resize ordering constructible in that harness.
- `ANALYSIS_TOOL_CALL_SCROLL_OSCILLATION_2026_08_17.md` →
  `FINDINGS_TOOL_CALL_SCROLL_OSCILLATION_2026_08_21.md` →
  `FINDINGS_TOOL_CALL_SCROLL_OSCILLATION_LIVE_INSTANCE_DATA_2026_08_22.md` →
  a still-open live-telemetry investigation into `scrollHeight` shrinking
  while pinned. The last of these found, from real `task package` instance
  logs, **two incidents where every open pane's `scrollHeight` collapsed to
  exactly `0px` simultaneously** (§3 of that doc), root cause unconfirmed.

That last finding matters directly here: a pane whose `scrollHeight` drops
to `0px` and then regrows past `clientHeight` is, geometrically, *identical*
to a pane that "loads without content, then finally overflows for the first
time" — the precondition in this bug report isn't limited to pane-open time,
it can recur mid-session any time that collapse happens. This spec's fix
(§5.1) closes the symptom for both cases at once, without needing to first
resolve what causes the 0px collapse.

## 3. Current mechanism, as verified in code

All in `frontend/app/view/agent/virtualization/`, unless noted.

- `state.ts:111` — `const [stickToBottom, setStickToBottom] = createSignal(true);`
  defaults to `true`, so a fresh pane starts "following."
- Three independent re-pin triggers, all funneling through
  `scrollToTrueBottom()` (`AgentDocumentVirtualList.tsx:188-199`):
  1. **Itemized content-signal effect** (`:494-511`) — re-fires on
     `nodes().length`, `layoutView().totalSize`, or `workingRowHeight`
     changing; scrolls inside a `queueMicrotask` with a live re-check of
     `stickToBottom()` before acting.
  2. **RO #1 — viewport resize** (`:535-551`) — observes `scrollRef` itself;
     re-pins on `clientHeight` change. Does **not** fire on content growth —
     `scrollRef`'s own box is fixed by the flex layout regardless of how
     tall its overflowing content gets (comment at `:561-566`).
  3. **RO #2 — content resize** (`:586-594`) — observes
     `virtualContainerRef` and `streamingBufferRef` directly; re-pins on
     *any* resize of those two elements, "regardless of what caused it"
     (`:569-571`). This is the mechanism specifically built to catch content
     growth an itemized signal doesn't cover (e.g. `MarkdownBlock`'s
     throttled syntax-highlight re-render firing ~90ms after the last
     token, entirely outside the Solid signal graph).
- **The disengage/engage decision** — `handleScrollNow()`
  (`AgentDocumentVirtualList.tsx:683-753`), called via a rAF-coalesced
  `scroll` listener. Reads `scrollRef.scrollTop/scrollHeight/clientHeight`
  live at `:685` (not a cached value). At `:719`:
  ```ts
  if (isNearBottom(scrollTop, scrollHeight, clientHeight)) {
      if (!props.viewState.stickToBottom()) props.viewState.engageStickToBottom();  // :721
  } else {
      if (props.viewState.stickToBottom()) {
          if (wasProgrammatic) {
              // suppressed — our own scrollTo() landed in this batch
          } else {
              props.viewState.disengageStickToBottom();   // :746
          }
      }
  }
  ```
  `wasProgrammatic` (`:692`) is set only when `scrollToTrueBottom()` itself
  triggered the pending scroll (`:199`) — it does not, and cannot, account
  for a native/browser-internal scroll adjustment that isn't the result of
  our own call.
- **`isNearBottom`** (`anchor.ts:68-85`):
  ```ts
  const maxScroll = scrollHeight - clientHeight;
  if (maxScroll <= 0) return true;   // no scrollbar ⇒ always "near bottom"
  ```
- **The only two re-engage paths once disengaged**: a `scroll` event that
  lands near bottom (`:721`), or `jumpToBottom()` (`:599-603`), wired to
  `AgentFooter`'s `onTyping` callback (`agent-view.tsx:2252-2254`) — this is
  exactly the "scroll manually or start typing" workaround in the bug
  report, which confirms the failure mode really is a `disengageStickToBottom()`
  call somewhere, not `stickToBottom` simply never having been `true`.

### Ruled out during this investigation

The obvious "ref not attached yet" shape of bug — RO #2 silently never
observing `streamingBufferRef` because that element doesn't exist at
`onMount` time for a pane that starts with zero nodes — does **not** apply
here. `streamingBufferRef`'s owning `<div>` is gated by
`<Show when={partition()}>` (`:1010`ish), and `partition` (`createMemo` at
`:232-283`) has no path that returns anything falsy — every branch returns
`result` from `partitionForVirtualization`, even for an empty document. So
`partition()` is truthy from the very first render, the streaming-buffer
div exists by the time `onMount` runs, and RO #2 attaches correctly
regardless of how many nodes the pane starts with. Ran this down explicitly
because it's the single most common shape for this class of bug and it
would have been a clean, deterministic, one-line fix — it just isn't what's
happening here.

## 4. Root cause: no single confirmed trigger — but a real, provable gap

Per `PLAN_AGENT_PANE_RESIZE_SCROLL_PIN_2026_08_05.md` §7 and
`ANALYSIS_TOOL_CALL_SCROLL_OSCILLATION_2026_08_17.md`'s own framing, this
environment cannot drive a live GUI repro to catch the exact event sequence
in the act — the prior docs hit the same wall. What *is* provable by
inspection, without needing that repro, is this:

**The code has no concept of "this pane has never had a real scrollbar
before," and it needs one.** `handleScrollNow`'s disengage branch (§3)
treats every non-`wasProgrammatic` "far from bottom" reading identically,
regardless of whether the pane has *ever* actually had `maxScroll > 0`
before this instant. But a pane transitioning from `maxScroll <= 0` to
`maxScroll > 0` for the first time cannot have a legitimate
user-initiated "scrolled away" state — there is nowhere to have scrolled
away *to* before that transition. Any disengage that fires on that exact
transition is illegitimate by construction, whatever produced the
misleading geometry read that triggered it (a native scrollbar-insertion
side effect, an event-batching edge case, or — per §2 — this pane living
through the still-unexplained whole-pane 0px collapse). Today, nothing
distinguishes "first overflow ever" from "the 500th scroll event on a
long-open pane" — both go through the exact same `isNearBottom` check with
no memory of prior state.

## 5. Proposed fix

### 5.1 Primary — make the first-overflow transition un-disengageable (root-cause-agnostic)

Add a `hasOverflowedOnce` boolean to `AgentViewState`
(`frontend/app/view/agent/virtualization/state.ts`), defaulting `false`:

```ts
// state.ts — new signal alongside stickToBottom (:111)
const [hasOverflowedOnce, setHasOverflowedOnce] = createSignal(false);
```

Expose it on the interface (near `stickToBottom` at `:38`) and from the
returned object (near `:152`).

In `handleScrollNow` (`AgentDocumentVirtualList.tsx`), before the existing
`isNearBottom` branch at `:719`, insert a one-time latch:

```ts
const maxScroll = scrollHeight - clientHeight;
if (maxScroll > 0 && !props.viewState.hasOverflowedOnce()) {
    // First time this pane has ever had a real scrollbar. There is no
    // legitimate "user scrolled away" state to preserve — nowhere existed
    // to scroll away to before this instant. Force-pin regardless of what
    // this scroll event's geometry otherwise reads as, and never take this
    // branch again for this pane's lifetime.
    props.viewState.markOverflowedOnce();
    if (!props.viewState.stickToBottom()) props.viewState.engageStickToBottom();
    scrollToTrueBottom();
    return;
}
```

This makes the fix a pure *addition* — the existing `isNearBottom` /
`wasProgrammatic` / disengage logic is untouched for every subsequent
scroll event on that pane, so no currently-correct "user deliberately
scrolled up to read history" behavior can regress. It only removes a
behavior (disengaging on the very first overflow) that could never have
been correct to begin with.

Also call `markOverflowedOnce()` from the two other places that already
learn `maxScroll` first went positive — RO #2's callback (`:588-594`) and
the itemized effect (`:494-511`) — so the latch is set even on a pane
where the very first overflow is caught by one of the *working* re-pin
paths rather than a scroll event; the `handleScrollNow` check above should
key off the same signal so all three paths agree on whether this pane has
already had its "first time" or not. (Simplest implementation: compute the
latch centrally, e.g. inside `scrollToTrueBottom()` itself, since every
re-pin path already funnels through it and it already reads
`scrollRef.scrollHeight`.)

### 5.2 Diagnostics — so the next telemetry pull can attribute this specifically

Extend the existing `[wave-scroll-shrink]` / `[wave-scroll]` console
tracing (`:191-196`, `:733-745`) with a `[wave-scroll-first-overflow]` line
whenever the §5.1 latch fires and finds `stickToBottom()` was already
`false` at that instant (i.e., a real save — this pane really was about to
silently strand the user). Follow the same log-then-correlate methodology
`FINDINGS_TOOL_CALL_SCROLL_OSCILLATION_2026_08_21.md` and
`..._LIVE_INSTANCE_DATA_2026_08_22.md` already established
(`muxlog fe grep wave-scroll`) — the next live-instance pull can then
directly confirm or rule out whether reports of this bug correlate with the
known 0px whole-pane collapse (§2) or are a third, distinct mechanism, the
same way those docs distinguished their Class 1 / Class 2 shrink sources.

### 5.3 Close the test-coverage gap

Neither existing scroll test file exercises this transition:

- `anchor.test.ts` covers `isNearBottom`'s `maxScroll <= 0` case
  (lines ~101-105) only as a static snapshot, never a before/after
  transition.
- `AgentDocumentVirtualList.resize.test.tsx` (the pane-splitter-resize
  regression suite) starts every test from an already-overflowing
  `scrollHeight`/`clientHeight` pair and resizes the *container* — it
  never grows *content* from `scrollHeight <= clientHeight` to
  `scrollHeight > clientHeight` while the container stays fixed size,
  which is exactly this bug's scenario and exactly what RO #2 exists to
  catch.

Add a test to `AgentDocumentVirtualList.resize.test.tsx` (or a new sibling
file) that: mounts with `scrollHeight === clientHeight` (no overflow),
grows `virtualContainerRef`'s or `streamingBufferRef`'s measured box past
`clientHeight` via the suite's existing fake `ResizeObserver` — with the
itemized signals (`nodes().length`, `layoutView().totalSize`,
`workingRowHeight`) held constant, so the test isolates RO #2 specifically
— and asserts `scrollTop` lands at true bottom. This is the direct
regression test for the reported bug; it would have caught the "ref never
attaches" hypothesis in §3 had that turned out to be true, and it locks in
correctness for whatever combination of signals causes the growth.

## 6. Explicitly out of scope

This spec does not attempt to resolve either still-open lead from
`FINDINGS_TOOL_CALL_SCROLL_OSCILLATION_LIVE_INSTANCE_DATA_2026_08_22.md`:
the source of the recurring ~251-252px shrink (§2 of that doc), or the
mechanism behind the whole-pane 0px collapse itself (§3). §5.1 makes this
bug's *symptom* structurally impossible regardless of what eventually
turns out to cause either of those — but the causes themselves stay
tracked in that doc's own "recommended next steps" (frame/mutation-level
instrumentation around `ReconcileTurnActive`, a source-level search for a
~251px fixed-height element).

## 7. Risk

Low. §5.1 only changes behavior for a transition that, by the invariant in
§4, can never have a legitimate reason to disengage — there is no existing
"user scrolled up to read history" case it could regress, because that
case requires overflow to have existed already, which is precisely the
condition this fix's one-time latch excludes itself from after firing
once per pane.
