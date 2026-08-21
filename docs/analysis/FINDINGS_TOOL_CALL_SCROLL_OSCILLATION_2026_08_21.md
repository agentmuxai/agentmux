# Findings: Tool-Call Scroll Oscillation — Correlated Live Repro Data (2026-08-21)

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

The same **19px delta recurs exactly** across two different tool
lifecycles ending two different ways (one runs to natural completion,
the next is user-interrupted) — strong, clean, reproducible evidence
that a tool block's DOM swap between its running/streaming shape and its
terminal/compact shape (`ToolOverlayLog.tsx`'s `<Switch>`, `ToolBlock.tsx`'s
summary row — "Source A" in the 08-17 analysis) is real and fires
consistently regardless of *how* the tool call ends, not just on clean
completion. This is now confirmed, not theorized.

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

A `Streaming → Idle → Streaming` double-flip within ~156ms
(`ReconcileTurnActive` firing twice in a row), and 21ms after the second
flip, **93% of the pane's scroll height disappears in one frame**
(14525px → 1023px). This is qualitatively different from Class 1's
per-tool DOM swap — the magnitude implies a large chunk of the transcript
(plausibly one or more full raw tool outputs, or a subagent's dispatch
transcript, collapsing to compact form all at once) rather than one
tool's summary row changing height. **Not characterized by name in the
08-17 analysis's three sources** — closest to an extreme case of Source A
if multiple tool blocks flip state in the same reconcile pass, but the
`ReconcileTurnActive` correlation (a turn-boundary re-evaluation event,
not a single tool's own state change) suggests a broader mechanism worth
its own investigation: what does `ReconcileTurnActive` actually
re-evaluate, and can it be batched/eased rather than applied as one
synchronous DOM mutation?

## What this confirms vs. still doesn't

**Confirmed:** the pin-teleport mechanism (`scrollToTrueBottom()` always
uses `behavior: "auto"`, no easing) reads as a visible backward jump for
*any* shrink, and Class 1 (tool running→terminal swaps, "Source A") is a
real, reproducible, correctly-identified contributor — now with two
independent trigger-path confirmations instead of zero.

**Still not established:** whether Source B (spinner-collapse
reclassification) or Source C (markdown throttle) independently
contribute — none of the 8 events isolate one from the other cleanly.
**New, not previously characterized:** the `ReconcileTurnActive`-linked
large-scale collapse (Class 2) — only one instance in this dataset, not
enough to characterize its trigger condition precisely, but its magnitude
makes it plausibly the more user-visible/jarring half of the "oscillation"
complaint, more so than the smaller Class 1 shrinks.

## Recommended next step

Don't re-run a blind repro — this data already answers "does the
instrumentation work and does it correlate with real events" (yes, on
both counts). The next useful step is narrower: instrument or trace
specifically what `ReconcileTurnActive` re-evaluates in
`AgentDocumentVirtualList.tsx`/the agent-document reducer, to explain
Class 2's 13,502px single-frame collapse — that's the highest-value
unanswered question this dataset raises, not a repeat of the original
"does this even happen" question.
