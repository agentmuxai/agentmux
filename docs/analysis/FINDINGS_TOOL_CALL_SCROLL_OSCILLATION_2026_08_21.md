# Findings: Tool-Call Scroll Oscillation — Correlated Live Repro Data (2026-08-21)

> **SUPERSEDED IN SCALE by
> `docs/analysis/FINDINGS_TOOL_CALL_SCROLL_OSCILLATION_LIVE_INSTANCE_DATA_2026_08_22.md`**
> (added 2026-08-29, docs-cleanup Phase 4). This document correlates **8
> events from one pane in one short dev-branch session**; the 08-22
> follow-up covers two running instances with hours of real usage —
> roughly two orders of magnitude more data — and surfaces two distinct
> anomalies this dataset was too small to separate: a recurring ~251px
> shrink, and multiple panes' `scrollHeight` dropping to exactly `0px`
> within milliseconds of each other. The 08-22 doc is explicit that this
> second phenomenon's mechanism is unknown — treat it as measured pane
> state, not a window-lifecycle event.
>
> This doc's conclusions aren't retracted, but **don't size the problem
> from this dataset**. The root-cause fix remains open — issues **#2648**
> and **#2718**.

Follow-up to `docs/analysis/ANALYSIS_TOOL_CALL_SCROLL_OSCILLATION_2026_08_17.md`,
which shipped the `[wave-scroll-shrink]` diagnostic (commit `70c6decb`,
2026-08-17) and theorized three unproven sources, with the explicit
recommendation to run a live repro and correlate the log output before
attempting a fix. **That live repro already happened, the same day the
diagnostic shipped — via a dedicated `ScrollPinTest` agent
(`.agentmux/agents/scrollpintest-host-7c31-.../`) on the
`loap-fix-tool-call-scroll-shrink-oscillation` dev branch — but the
collected data was never pulled and correlated.** This doc does that.

## Data source

`C:\Users\asafe\.agentmux\dev\loap-fix-tool-call-scroll-shrink-oscillation\415610cb303ae11d\logs\agentmux-host-v0.55.11.log.2026-08-17`
(+ a 1-line continuation in the `.log.2026-08-18` rotation). 8 total
`[wave-scroll-shrink]` events, all on the same pane (`2a15f3f`,
agent "ScrollPinTest-host-7c31"), correlated against the same log's
`[wave-turn]` state-machine trace for that pane.

## Two distinct shrink classes found — not one mechanism

### Class 1: small, ~16–24px, tied to tool running→terminal transitions

```
23:33:26.126  Done → Streaming  cmd=ToolStart      toolsActive=1 currentTool=Bash
23:33:41.561  [wave-scroll-shrink] 1023px -> 1004px (delta=19px)      ← mid-stream
23:33:44.337  Streaming → Done  cmd=TurnEnd
23:33:44.343  Done → Streaming  cmd=ToolStart       toolsActive=1 currentTool=Bash
23:33:50.408  Streaming → Interrupting  cmd=RequestStop
23:33:51.909  Interrupting → Done  cmd=TurnEnd
23:33:51.921  [wave-scroll-shrink] 1023px -> 1004px (delta=19px)      ← IDENTICAL delta
```

**Correction (Codex P2, caught after this doc first merged):** the
original wording here overclaimed. Only the SECOND 19px shrink
(23:33:51.921) is actually temporally adjacent to a terminal transition —
12ms after that tool's own `Interrupting → Done`. The FIRST one
(23:33:41.561) fires 2.8 seconds *before* its tool's `TurnEnd`, while the
pane is still `Streaming` — it is not evidence of an end-of-tool DOM swap
at all, just a coincidentally-identical-magnitude shrink from an
unidentified cause. There is exactly **one** clean example here, not two,
and "confirmed" was too strong a word for one data point.

That one example is still worth something, though, and more precisely
than originally stated: the 08-17 analysis's own addendum already ruled
out `ToolBlock.tsx`'s summary row as a possible cause (`_document-nodes.scss`
forces `.agent-tool-summary` to `white-space: nowrap; overflow: hidden` —
fixed height regardless of content) — so this is NOT that component. The
addendum instead flags `ToolOverlayLog.tsx`'s `<Switch>` branch-swap as
"the most credible source of a genuinely large vertical delta," but notes
the existing FLIP-transition mitigation should already engage for the
*common* case, with a real bypass gap "only for `denied`/`canceled`
tools." The one clean example in this dataset **is** a canceled tool
(`RequestStop` → `Interrupting` → `Done`) — consistent with, and a
plausible live instance of, exactly that narrow bypass gap. Worth
targeted verification (does `heightStale` read `true` for this specific
pane at this timestamp), not yet confirmed.

Two more same-class events (23:32:59, 1023→1007 delta=16px; 23:33:14,
1007→991 delta=16px) occur back-to-back with no tool-state transition
logged between them, ~15–25s apart — consistent with "Source C"
(`MarkdownBlock.tsx`'s throttled syntax-highlight re-render) or ordinary
streamed-content growth/settle, not further isolated by this data.

### Class 2: one enormous 13,502px collapse, tied to a `ReconcileTurnActive` double-flip

```
23:32:32.767  Done → Submitting        cmd=TurnStart
23:32:32.973  Submitting → Streaming   cmd=StreamFlushObserved
23:32:33.784  Streaming → Idle         cmd=ReconcileTurnActive
23:32:33.799  [wave-scroll-shrink] 14549px -> 14525px (delta=24px)     ← small, Class 1-like
23:32:33.940  Idle → Streaming         cmd=ReconcileTurnActive         ← flips back, 141ms later
23:32:33.961  [wave-scroll-shrink] 14525px -> 1023px (delta=13502px)   ← the big one, 21ms later
23:32:39.024  Streaming → Done         cmd=TurnEnd
```

**Correction (Codex P2, caught after this doc first merged):** "in one
frame" and "synchronous DOM mutation" overclaimed what this data can
show. `lastKnownScrollHeight` only updates when `scrollToTrueBottom()`
itself runs — so consecutive log lines are the cumulative delta between
two *pin-check invocations* (~162ms apart here), not two consecutive
render frames. The actual collapse could have happened across several
frames/mutations within that window, not necessarily one. Correspondingly,
`ReconcileTurnActive` firing 21ms before the big shrink is a **temporal
correlation, not an established causal link** — one occurrence in the
whole dataset is not enough to claim it caused anything, only that it's
the most notable nearby event.

With that walked back: a `Streaming → Idle → Streaming` double-flip
within ~156ms (`ReconcileTurnActive` firing twice in a row) precedes 93%
of the pane's scroll height disappearing (14525px → 1023px) sometime in
the following ~21ms-to-next-pin-check window. This is qualitatively
different from Class 1's per-tool DOM swap — the magnitude implies a
large chunk of the transcript (plausibly one or more full raw tool
outputs, or a subagent's dispatch transcript, collapsing to compact form)
rather than one tool's summary row changing height. **Not characterized
by name in the 08-17 analysis's three sources.** Confirming an actual
causal link to `ReconcileTurnActive` — as opposed to noting the
correlation — needs mutation- or component-level timing instrumentation
finer-grained than this pin-check-level diagnostic, not just more log
data of the same kind.

## What this confirms vs. still doesn't

**Confirmed:** the pin-teleport mechanism (`scrollToTrueBottom()` always
uses `behavior: "auto"`, no easing) reads as a visible backward jump for
*any* shrink — this part is architectural fact, not inference from this
dataset.

**Plausible, one supporting data point, not confirmed:** a canceled tool's
`ToolOverlayLog.tsx` `<Switch>` branch-swap (the narrow FLIP-mitigation
bypass gap the 08-17 addendum already flagged for `denied`/`canceled`
tools specifically) as the source of the one clean Class 1 example.
`ToolBlock.tsx`'s summary row is ruled OUT (fixed height, per that same
addendum) — don't point a fix there.

**Not established:** whether Source B (spinner-collapse reclassification)
or Source C (markdown throttle) independently contribute — none of the 8
events isolate one from the other cleanly. **New, not previously
characterized, correlation only (not causation):** the
`ReconcileTurnActive`-adjacent large-scale collapse (Class 2) — one
instance in this dataset, magnitude alone makes it worth investigating
further, but neither its exact cause nor its frame-level timing is
established by this diagnostic.

## Recommended next step

Don't re-run a blind repro — this data already answers "does the
instrumentation work and does it correlate with real events" (yes, on
both counts). Two concrete, narrower follow-ups, in priority order:

1. **Class 2 (the large collapse):** the pin-check-level diagnostic used
   here can't resolve frame- or mutation-level timing. Add finer-grained
   instrumentation (a `ResizeObserver`/mutation-observer trace, or
   per-render logging in the agent-document reducer around
   `ReconcileTurnActive`) before concluding anything about its actual
   cause — this is the highest-value unanswered question, but needs
   different tooling than more `wave-scroll-shrink` log lines.
2. **Class 1 (the canceled-tool shrink):** check whether `heightStale`
   reads `true` for the specific pane/tool at 23:33:51.921 — a direct,
   cheap way to confirm (or rule out) the `denied`/`canceled` FLIP-bypass
   hypothesis without needing a new repro.
