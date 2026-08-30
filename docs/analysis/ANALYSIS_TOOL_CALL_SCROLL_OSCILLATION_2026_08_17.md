# Analysis: agent-pane scroll "spins backward" during tool-call streaming

> **§6'S LIVE REPRO HAS SINCE HAPPENED — read the follow-ups before acting
> on this document** (added 2026-08-29, docs-cleanup Phase 4):
> 1. `docs/analysis/FINDINGS_TOOL_CALL_SCROLL_OSCILLATION_2026_08_21.md` —
>    correlated a single dev-branch repro's 8 `[wave-scroll-shrink]` events
>    against this doc's three theorised sources.
> 2. `docs/analysis/FINDINGS_TOOL_CALL_SCROLL_OSCILLATION_LIVE_INSTANCE_DATA_2026_08_22.md`
>    — two running instances, hours of real usage, ~2 orders of magnitude
>    more data. Surfaced **two distinct anomalies**: a recurring ~251px
>    shrink, and multiple panes' `scrollHeight` dropping to exactly
>    `0px` within milliseconds of each other (3 panes in one incident, 2 in
>    another). That second one's mechanism is explicitly unknown — the
>    08-22 doc hypothesises an app-wide event but does not establish it,
>    and it is pane-measurement state, not the window collapsing.
>
> The diagnostic this doc recommended shipped in commit `70c6decb`. **The
> root-cause fix is still open** — tracked in issues **#2648** ("diagnostic
> shipped, root-cause fix still blocked on live verification") and
> **#2718** (the two anomalies). So this document is *not* resolved; its
> §6 "needs a live repro" framing is simply no longer the blocker.

**Date:** 2026-08-17
**Status:** Root-cause analysis from direct code inspection + prior internal
docs. No code changed. One question (§6) needs a live repro this agent
cannot drive (no GUI automation available in this environment — same
limitation `PLAN_AGENT_PANE_RESIZE_SCROLL_PIN_2026_08_05.md` §7 hit).
*(That repro has since been run — see the banner above.)*

## Addendum 2026-08-17 (same day) — corrections from a first implementation pass, plus diagnostic instrumentation shipped

Two corrections to §2 below, found while attempting to implement a fix, both
important enough to flag before anyone acts on the original text:

1. **`ToolBlock.tsx`'s summary-row swap (§2, "Source A", `ToolBlock.tsx`
   half) does NOT affect vertical scroll at all.** Confirmed by reading
   `_document-nodes.scss:229-233`: `.agent-tool-summary` is
   `white-space: nowrap; overflow: hidden` — a forced single-line row. The
   live-tail ↔ result-pill swap changes horizontal content only; the row's
   height is fixed regardless of which child is present. This was wrong in
   the original analysis and should not be treated as a fix target for the
   symptom in this doc.
2. **A "shrink-aware eased `scrollToTrueBottom()`" fix (the initially
   proposed remediation) cannot work as stated**, for a specific physical
   reason: the browser clamps `scrollTop` down to the new
   `scrollHeight - clientHeight` **synchronously, as part of the same
   layout pass that produces the shrink** — before any application code
   (effects, `ResizeObserver` callbacks) runs. By the time our code
   observes the shrink, the visible jump has already happened; there is
   nothing left to animate. This is the same fact
   `PLAN_AGENT_PANE_RESIZE_SCROLL_PIN_2026_08_05.md` §2 ("H1") established
   for the grow case, just not carried through to the shrink case
   originally. The technique that DOES work is the one already partially
   implemented in `ToolOverlayLog.tsx` (`flipHeight()`, PR #1975): freeze
   the shrinking element at its old height BEFORE the browser paints the
   smaller layout, then ease it down via CSS transition — so the native
   `scrollTop` clamp happens in many imperceptible per-frame steps instead
   of one visible jump. Fixing the outer scroll's *symptom* requires
   preventing shrinks at the *content* layer, not intercepting the scroll
   position after the fact.

Re-tracing `createSpinnerCollapser` (`output-cap.ts`) more carefully after
(1): the "retroactive reclassification" it documents can only ever affect
the single most-recent, not-yet-committed chunk (the code's own comment
confirms at most one chunk is ever "pending" at a time) — so in practice it
swaps at most one rendered line for another line of the same count. §3's
original framing ("lower-confidence... 1-line-for-1-line") was already
appropriately hedged and doesn't need correction, but is reiterated here:
this is not a "large block ↔ small block" mechanism on its own.

What's left standing from the original analysis, unweakened: the
`ToolOverlayLog.tsx` `<Switch>` branch-swap (§2, the `ToolOverlayLog.tsx`
half of "Source A") is still the most credible source of a genuinely large
vertical delta — but tracing `ToolBlock.tsx`'s `autoExpanded()` /
`isFailTerminal()` logic shows the existing FLIP mitigation should already
engage correctly for the common case (a `success`/`failed` tool holds its
panel open — not `content-visibility: hidden` — straight through the
`running` → terminal transition, so `heightStale` should read `false` at
exactly the moment it matters). The `heightStale` bypass appears to be
real but narrower than originally implied — it reliably fires only for
`denied`/`canceled` tools, whose panel collapses in the same synchronous
tick as the branch change (`isFailTerminal()` skips the held-open hold).
**This means the dominant real-world cause is not yet pinned down with
confidence** — static tracing has been pushed about as far as it usefully
goes; the two corrections above are both things that only became clear by
tracing actual CSS/timing rather than by reasoning about the component
code alone, which argues for live data over a third round of the same.

**Shipped this pass (no behavior change, diagnostic only):**
`AgentDocumentVirtualList.tsx`'s `scrollToTrueBottom()` now logs
`[wave-scroll-shrink] pane=<id> scrollHeight <old>px -> <new>px
(delta=<n>px)` via `console.info` whenever it's invoked (from any of the
three pin mechanisms, or `jumpToBottom`) and the container's `scrollHeight`
has decreased since the last such call. This is the single choke point all
three pin mechanisms already funnel through, so it catches a shrink from
*any* source (branch swap, spinner reclassification, markdown re-highlight,
or something not yet identified) without needing per-source instrumentation.
Reachable live via `muxlog fe grep wave-scroll-shrink` against a running
instance — see the branch `loap/fix-tool-call-scroll-shrink-oscillation`.
121/121 existing tests in `frontend/app/view/agent/virtualization` still
pass (no logic changed, only a diagnostic branch added inside an existing
function).

**Recommended next step:** run a `task dev` build from this branch, drive a
real repro (a chatty tool call, ideally including at least one
`denied`/`canceled` completion to isolate the `heightStale` hypothesis),
and correlate `wave-scroll-shrink` log lines against what's visually
observed. Only implement a content-layer fix (FLIP-style freeze-then-ease,
scoped to whichever element the log identifies) once that correlation is
in hand — guessing a third time without it has a track record now (twice
in this same session).

---

## 0. Symptom, as reported

> Sometimes it appears like the scrolling is going backward, during the
> expansion of tool calls — it spins back as it's outputting. A large block
> comes out, then it's replaced with a small block causing the scroll to go
> backward, then replaced again by a large block. Ideally there should be no
> scrollback behavior — a continuous one-direction flow.

This is a **repeating oscillation** observed while tool calls are actively
streaming/expanding, not a single one-shot glitch. That framing matters —
see §4.

## 1. This is not one bug — it's three independent DOM-shape-swap
mechanisms, all missing a "never shrink visibly" guarantee, sitting under
scroll-pin machinery that only ever *snaps*, never eases

The agent pane (`frontend/app/view/agent/virtualization/AgentDocumentVirtualList.tsx`)
keeps `viewState.stickToBottom()` pinned to true bottom via three
independent triggers — verified directly in the current file:

- **Itemized `createEffect`** (`AgentDocumentVirtualList.tsx:464-483`) — re-pins on `nodes().length` / `layoutView().totalSize` / `workingRowHeight()` changes, via `queueMicrotask`.
- **RO #1 — viewport resize** (`:505-525`) — re-pins on `scrollRef.clientHeight` change (sibling panels, pane resize).
- **RO #2 — content resize** (`:556-565`) — re-pins on `virtualContainerRef`/`streamingBufferRef` box-size change (this one exists specifically to catch late reflow the other two miss; added per `REPORT_AGENT_PANE_SCROLL_PIN_FLICKER_AUDIT_2026_07_30.md` §4's recommendation).

All three call the same `scrollToTrueBottom()` (`:167-171`):

```ts
function scrollToTrueBottom(): void {
    if (!scrollRef) return;
    pendingProgrammaticScroll = true;
    scrollRef.scrollTo({ top: Number.MAX_SAFE_INTEGER, behavior: "auto" });
}
```

`behavior: "auto"` is an **instant** jump, by design (§3.4 of the 07-30
report explicitly recommends "auto" over "smooth" during streaming, to
avoid a laggy eased-scroll fighting a fast-arriving stream — a reasonable
call for the *growing* case). But it means the pin has exactly one gear:
teleport to `scrollHeight - clientHeight`. There is no code path anywhere in
this component that *eases* a scroll-position correction.

That's fine as long as `scrollHeight` only ever **grows** while pinned — a
teleport-to-bottom during monotonic growth is invisible (you're already
where the teleport lands, modulo the pin *lag* bugs the 07-24/07-27/07-30/
08-05 docs chased). It stops being invisible the moment `scrollHeight`
**shrinks** while pinned:

1. The browser's own layout step auto-clamps `scrollTop` down to the new,
   smaller `scrollHeight - clientHeight` the instant the shrink lays out —
   this is unconditional native behavior, confirmed in
   `PLAN_AGENT_PANE_RESIZE_SCROLL_PIN_2026_08_05.md` §2 ("H1" analysis: "the
   browser's auto-clamp always sets `scrollTop` to *exactly* the new max").
2. That clamp is a **discrete jump**, not a transition — nothing animates a
   `scrollTop` correction in this codebase.
3. Any of the three pin mechanisms above then fires and calls
   `scrollToTrueBottom()` again — a no-op (already at the new max), but
   irrelevant: the visible jump already happened in step 1.

**So the actual bug is not "the pin doesn't work" — the pin works exactly
as designed. The bug is that something legitimately shrinks the rendered
content while streaming, and a shrink-then-regrow cycle is visually
indistinguishable from "the scrollbar spun backward."** Three confirmed,
independent sources of that shrink:

## 2. Source A (primary suspect) — the running→terminal DOM-shape swap, only half-mitigated

`mergeReplacement()` (`frontend/app/store/agent-document/reducer.ts:906-926`)
flips a tool's `status` out of `"running"` **and** force-clears
`log.open` to `false` in the same reducer tick when a `tool_result` lands:

```ts
const terminal =
    replacement.status === "success" ||
    replacement.status === "failed" ||
    replacement.status === "denied";
const mergedLog: ToolStreamingLog = {
    chunks: existingLog.chunks,
    open: terminal ? false : existingLog.open,
};
```

Two UI surfaces key off exactly those two signals and both swap DOM shape
on that same tick:

- **`ToolOverlayLog.tsx:310-323`** — an exclusive `<Switch>` that unmounts
  `ChunkList` (raw streamed lines — can be dozens of `<pre>` rows for a
  chatty command) and mounts `ToolOverlayResult` (a compact, structured
  view — `BashOutputViewer`, `DiffViewer`, etc.). These are genuinely
  different component trees with different natural heights; a long-running
  `npm install`'s live tail is routinely much taller than its terminal
  `exit 0` summary.
- **`ToolBlock.tsx`'s summary row** (`:390-434`) — the live-tail/elapsed
  span (`.agent-tool-live-tail`, gated on `log?.open === true`) disappears
  the same instant the result pill (`.agent-tool-result-pill`, gated on
  `resultPill() != null`) appears. Confirmed by direct read: these are two
  independent sibling `<Show>` blocks with **no shared box, no cross-fade,
  no height reservation** — the row's own height can change too.

**This was diagnosed once already** (`docs/analysis/ANALYSIS_TOOL_PREVIEW_RUNNING_TO_COMPLETED_JERK_2026_07_05.md`)
and **partially fixed** in PR #1975 (`fix(agent): smooth the running ->
completed tool-preview transition`): `ToolOverlayLog.tsx:238-289` now runs a
FLIP-style height transition (`flipHeight()`, `:342-363`, 150ms
`cubic-bezier`) whenever the rendered `<Switch>` branch changes, so the
*panel body's* height eases across the swap instead of jump-cutting.

**But the fix is narrower than the bug in three ways, all confirmed by
direct read of the current file:**

1. **It only covers `ToolOverlayLog`'s own box.** `ToolBlock.tsx`'s summary
   row (the live-tail ↔ result-pill swap, part of *every* tool call, not
   just ones with an open panel) has no equivalent treatment — still an
   instant content-shape swap today, exactly as the 07-05 analysis
   described it, unchanged since.
2. **The FLIP itself is skipped — jump-cut instead — whenever the panel was
   `content-visibility: hidden` at the moment of transition** (`heightStale`,
   set at `:226-235, 252-253, 257-266`; gated at `:273-280`). A tool that
   auto-collapses immediately on leaving `running` (any `denied`/`canceled`
   tool, per `ToolBlock.tsx`'s `isFailTerminal()`), or one whose panel is
   simply scrolled out of the measurable viewport, commonly hits exactly
   this branch — the case the mitigation exists for is also the case most
   likely to disable it.
3. **Easing the *content's* height does not ease the *outer scroll
   container's* `scrollTop`.** Even where the 150ms FLIP is active, it
   changes `ToolOverlayLog`'s box size on every animation frame — and RO #2
   (§1) fires on every one of those frames and re-teleports `scrollTop` to
   the instantaneous new bottom, since `scrollToTrueBottom()` has no eased
   variant. The content glides; the viewport's read of "true bottom" still
   jumps in discrete per-frame steps chasing it. Whether that reads as
   smooth or juddery to the user depends on frame timing that hasn't been
   measured live (§6).

## 3. Source B — retroactive spinner-run reclassification (real, continuous, lower-confidence as the *dominant* cause here)

`frontend/app/view/agent/components/output-cap.ts`'s `createSpinnerCollapser()`
(`:362-433`) folds consecutive terminal redraw frames (progress bars,
spinners) into a single updating DOM node, to avoid one `<pre>` per frame.
Its own doc comment (`:349-360`) states the mechanism explicitly:

> "sometimes retroactively groups a previously-standalone trailing chunk
> into a new run" — a chunk rendered on one call as its own committed
> `display` entry can, on the *next* chunk, be pulled back out and folded
> into `spinnerSlot` instead, once a second frame arrives that makes the
> first one *look* like the start of a redraw run in hindsight
> (`startsRun`/`continuesRun`, `:280-292`; the ambiguity is inherent — you
> can't know a chunk starts an animation run until you see the chunk that
> follows it).

Traced through the algorithm (`:372-432`), a single reclassification event
is usually a 1-line-for-1-line swap (a tentative `display` entry becomes an
identical-length `spinnerSlot` entry), so on its own it's a weaker fit for
"large block ↔ small block" than Source A. It's flagged here because it:

- Fires **continuously while a tool is actively streaming** (matches "as
  it's outputting" more literally than A, which is a one-shot event per
  tool call), and
- Compounds with A — a chatty tool with progress-bar output hits both the
  per-chunk reclassification *and* the running→terminal swap when it
  finishes, and
- Is the kind of mechanism worth a live check (`REDRAW_SIMILARITY_THRESHOLD`
  false-positive/negative behavior, `:186-188`) specifically for whatever
  tool the user was watching when they saw this — output-cap.ts has its own
  documented history of false-positive tuning (PR #2330, referenced inline
  at `:184-188`), so a tool whose real output looks progress-bar-ish is a
  plausible amplifier even if it isn't the root cause.

## 4. Source C — MarkdownBlock's throttled highlight re-render (documented, unfixed, still live)

`frontend/app/view/agent/components/MarkdownBlock.tsx:64-80` — confirmed by
direct read, unchanged since `REPORT_AGENT_PANE_SCROLL_PIN_FLICKER_AUDIT_2026_07_30.md`
§2.1 flagged it (no commits to this file touch this logic since; `git log`
confirms). During streaming, markdown re-parses render a cheap intermediate
(`highlight: !streaming`); ~90ms after the last token in a burst, a trailing
`setTimeout` commits `highlight: true` — a full syntax-highlighted
re-render that "routinely reflow[s] to a different height than the
plain-text intermediate" (the report's own words). This is a text/thinking
block mechanism, not tool-call-specific, but it's the same *family* of bug
(a throttled/delayed re-render silently swapping DOM shape) and fires on
the same cadence ("during normal streaming... not just at turn boundaries")
the user describes. **Unlike Source A, this one is not mitigated at all** —
no FLIP, no cross-fade; RO #2 (§1) should now at least *catch* the resulting
resize and re-pin (it didn't exist yet when the 07-30 report called this
out — it was the report's own recommended fix, landed since), but "caught
and re-pinned" still means "a hard snap," not "no visible motion" — see §1.

## 5. Why this reads as an "oscillation" specifically during tool-call expansion

None of A/B/C individually oscillates forever — each is a shrink (or grow)
event tied to a state transition. The *sequence* is what produces the
described spinning:

- A running tool streams output → content is large (raw `ChunkList` /
  live-tail).
- It completes → Source A fires → content shrinks (compact result) →
  scroll snaps backward (§1).
- The **next** tool call in the same turn starts running → its own
  `ChunkList` starts accumulating → content grows again → scroll (already
  pinned) rides the growth forward, back toward where it was.
- That tool completes → shrinks again → snaps backward again.

For a turn that chains several tool calls (a very common shape — read
several files, run a few greps, edit, run tests), this reproduces exactly
the reported "large block, small block, large block" cycle, once per tool
boundary, for the duration of "the expansion of tool calls." Source B adds
a faster, smaller-amplitude version of the same thing *within* a single
still-running tool's output if it looks progress-bar-like.

## 6. What isn't confirmed yet (needs a live repro, not static analysis)

Following the same discipline `PLAN_AGENT_PANE_RESIZE_SCROLL_PIN_2026_08_05.md`
used (that plan explicitly refused to ship a fix without a live repro, and
correctly disproved its own leading hypothesis by testing rather than
reasoning alone):

1. **Relative magnitude of A vs. B vs. C** in the user's actual repro —
   the code confirms all three are real and live, but not which one
   dominates the specific session that prompted this. The existing
   `[wave-scroll]` console channel (`AgentDocumentVirtualList.tsx:666-676`
   per the 08-05 plan's citation) already logs engage/disengage decisions
   with `scrollTop`/`scrollHeight`/`clientHeight`/gap on every pin
   decision — filtering that during a repro, cross-referenced with which
   tool was active at each backward jump, would disambiguate directly
   without new instrumentation.
2. **Whether RO #2's per-frame re-teleport during an active FLIP (§2 point
   3) is itself visually smooth or adds its own micro-judder** — a
   plausible *additional* contributor on top of A/B/C, not yet measured.
3. Whether the specific tool(s) in the user's repro have progress-bar-style
   output (implicates B) or are simple one-shot commands (points to A/C).

## 7. Why this is an architecture question, not just "add another special case"

The repo's own history already reached this conclusion once, for a related
but distinct symptom (pin *lag*, not shrink-triggered *reversal*) —
`REPORT_AGENT_PANE_SCROLL_PIN_FLICKER_AUDIT_2026_07_30.md` §2 states it
plainly: *"This is the third documented pass at this bug class, and each
pass added one more named dependency or observer rather than a structural
fix... The pattern itself is the root cause."* That report's own fix (RO #2)
has since landed — and the shrink-side problem analyzed here is the same
architectural gap wearing a different face:

**There is no single place in this codebase where "a node's rendered DOM
shape is about to change" is a first-class event.** Instead there are, by
current count, at least **six independent mechanisms**, each independently
deciding whether/how to react to a height change, none aware of the others:

| # | Mechanism | File | Reacts to |
|---|---|---|---|
| 1 | Itemized pin effect | `AgentDocumentVirtualList.tsx:464-483` | node count / layout total / working-row height |
| 2 | RO #1 (viewport) | `AgentDocumentVirtualList.tsx:505-525` | `scrollRef.clientHeight` |
| 3 | RO #2 (content) | `AgentDocumentVirtualList.tsx:556-565` | `virtualContainerRef`/streaming-buffer box size |
| 4 | Local FLIP | `ToolOverlayLog.tsx:238-289` | its own `<Switch>` branch changing |
| 5 | Local auto-scroll-within-panel | `ToolOverlayLog.tsx:179-200` | `chunks()` / `panelHidden()`, scoped to the panel's own internal scroll, not the outer pane |
| 6 | Throttled re-render timers | `MarkdownBlock.tsx:64-80`, `createSpinnerCollapser` (`output-cap.ts:362-433`) | their own internal timers/heuristics, invisible to 1-5 entirely |

Each was added in response to a specific reported symptom (the 07-24 →
07-27 → 07-30 → 08-05 → this-doc chain, plus #1975/#2330 on the content
side) rather than from a shared contract. The result is what you'd expect
from six uncoordinated systems layered over two build cycles: individually
reasonable, collectively unable to guarantee the one property the user
actually wants — **monotonic, one-directional visual flow while pinned to
bottom** — because no component in the stack owns that guarantee end to
end. `scrollToTrueBottom()` is correct *arithmetic* (always lands exactly
at true bottom) but provides zero *visual continuity* guarantee, and
nothing upstream of it prevents the content it's chasing from legitimately
getting shorter mid-stream.

### Two different depths of fix, worth weighing separately

**A. Contain the symptom (bounded, incremental, consistent with how every
prior pass in this file's history has shipped):**
- Extend the existing FLIP pattern from `ToolOverlayLog`'s body to
  `ToolBlock`'s summary row (§2, point 1) — reserve a fixed-height slot and
  cross-fade live-tail ↔ result-pill instead of an instant swap, per the
  07-05 analysis's own unimplemented recommendation (§"A viable fix shape").
- Fix the `heightStale` jump-cut gap (§2, point 2) — even a short, cheap
  fallback animation (or simply not skipping the FLIP for the common
  `denied`/`canceled` auto-collapse case) removes the largest share of
  cases where the existing mitigation silently doesn't apply.
- Give `scrollToTrueBottom()` a **shrink-aware eased variant**: when the new
  true-bottom target is *less* than the current `scrollTop` (a shrink, not a
  grow), ease over ~120-150ms instead of teleporting: this is the single
  highest-leverage change, because it fixes the *visible* symptom
  regardless of which of A/B/C caused the shrink, without having to track
  down and individually FLIP every current and future source of content
  shrinkage. Growth-case teleporting can stay instant (already correct,
  already relied on for streaming responsiveness per the 07-30 report's own
  §4 rule 4).

**B. Fix the structural gap (larger, matches what the repo's own audit
already called for on the lag side):** give DOM-shape-changing nodes (tool
completion, spinner-run reclassification, markdown highlight commit, and
whatever the next one turns out to be) a single shared "content is about to
resize" contract — e.g. every such site reports a from/to height (or simply
relies on one, generalized, outer-scoped ResizeObserver + FLIP helper
instead of five bespoke local ones) — so that "don't let visual height
changes jump-cut while pinned" is a property of the framework layer, not
something each new component has to remember to opt into. This is the
"stop whack-a-moling, fix the pattern" move the 07-30 report explicitly
called for and only partially delivered (it unified the *pin-lag* side via
RO #2; the *pin-direction/shrink* side analyzed here was out of that
report's scope and remains exactly as fragmented as the lag side was before
RO #2 landed).

Recommend (B) NOT be scoped as one big rewrite — the six mechanisms in the
table above are individually well-tested and several have their own
regression suites (`AgentDocumentVirtualList.resize.test.tsx`, `anchor.test.ts`,
`state.test.ts`). A safer sequencing: ship the shrink-aware eased scroll
(A, third bullet) first since it's the one change that structurally
subsumes the others' symptoms without touching five files, verify it live
against §6, then decide whether the remaining jump-cut/FLIP gaps (A, first
two bullets) are still worth closing individually or whether the eased
scroll alone already reads as "no scrollback" to a user.

## Files referenced (all read directly)

| File | Role |
|---|---|
| `frontend/app/view/agent/virtualization/AgentDocumentVirtualList.tsx:167-171, 464-565` | The three pin mechanisms + the un-eased `scrollToTrueBottom()` |
| `frontend/app/store/agent-document/reducer.ts:906-926` | `mergeReplacement()` — couples `status` + `log.open` on one tick |
| `frontend/app/view/agent/components/ToolOverlayLog.tsx:91-108, 202-289, 310-323, 328-363` | The `<Switch>` DOM-shape swap + its partial FLIP mitigation + the `heightStale` bypass |
| `frontend/app/view/agent/components/ToolBlock.tsx:227-279, 390-434` | `resultPill()` / live-tail — unmitigated sibling-swap in the summary row |
| `frontend/app/view/agent/components/output-cap.ts:343-433` | `createSpinnerCollapser` — retroactive reclassification |
| `frontend/app/view/agent/components/MarkdownBlock.tsx:32, 64-80` | Throttled streaming re-parse/highlight commit |
| `frontend/app/view/agent/virtualization/anchor.ts:66-85` | `isNearBottom` / `STICK_TO_BOTTOM_THRESHOLD_PX = 200` |

## Related prior docs (read in full, not just referenced)

| Path | Relevance |
|---|---|
| `docs/analysis/ANALYSIS_TOOL_PREVIEW_RUNNING_TO_COMPLETED_JERK_2026_07_05.md` | Diagnosed Source A originally; its recommended FLIP fix landed (#1975) but only for `ToolOverlayLog`'s body, not `ToolBlock`'s summary row |
| `docs/specs/REPORT_AGENT_PANE_SCROLL_PIN_FLICKER_AUDIT_2026_07_30.md` | Diagnosed the *lag* half of this bug class (content grows, pin doesn't follow) and correctly identified "itemized dependency whack-a-mole" as the root pattern; its RO #2 recommendation has since landed and is confirmed present in the current file |
| `docs/specs/PLAN_AGENT_PANE_RESIZE_SCROLL_PIN_2026_08_05.md` | Same bug class, splitter-resize trigger; methodology model for this doc (disproved its own leading hypothesis via a real-component test rather than reasoning alone) — its H4 (overlay-lag) and the general "no visible motion should ever come from a hard snap" theme are adjacent to §7 here |

No prior doc connects the running→terminal shrink (Source A), the spinner
reclassification (Source B), and the markdown re-highlight (Source C) as
one shared "un-eased shrink" symptom class, or names the un-eased
`scrollToTrueBottom()` itself as the common amplifier — that synthesis is
new in this document.
