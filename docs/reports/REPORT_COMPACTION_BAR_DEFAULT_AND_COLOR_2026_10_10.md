# REPORT — Compaction progress bar: a built-in first estimate, and the pane's colour

**Status:** analysis
**Date:** 2026-10-10
**Author:** AgentY@narko
**Trigger:** Operator: "compacting is always about 1 minute, so the first [compaction] with the hidden bar isn't
needed. The first one should be just 1m, and every one after can build off that. Second, the colour of the bar
should be the same as the progress status text, which derives from the agent pane colour."

Builds on `docs/specs/SPEC_COMPACTION_ESTIMATED_PROGRESS_AND_STREAM_FRAMES_2026_10_01.md` (the estimated bar).
Read against `main` at e3a741dcc.

## 1. How it works today

- `frontend/app/view/agent/compaction-estimate.ts` keeps the last 30 finished compactions (`uuid`, `preTokens`,
  `durationMs`, `model`) in the window's `localStorage` under `agentmux:compaction-samples:v1`. They are recorded
  from each `compact_boundary` frame in `useAgentStream.ts`.
- `estimateCompactionMs(samples, contextTokens)` returns the median duration, scaled by the current context size
  against the samples' median (clamped to 0.5x–2x, then to 5 s–600 s). **With no samples it returns `null`.**
- `AgentWorkingRow` in `components/AgentFooter.tsx` (lines 321–338, 418–431, 476–513) reads the estimate once
  per compaction. When the estimate is `null` there is no bar, and the right side shows only the elapsed time.
  With an estimate it shows `12s / ~1m 17s`, a 2 px bar along the row's bottom edge held at 95 %, and after
  overshooting, "longer than usual" with an indeterminate sweep.

### Finding 1: the first compaction in every build has no bar

`localStorage` is per build channel, so the store starts empty for every new build (dev, local package, a fresh
install), and the first compaction there always shows no bar. This is what the operator saw on 2026-10-09 after
moving to the isolated build. The spec chose it on purpose (R3: "no history → plain elapsed counter, no bar";
§ out-of-scope: "seeding the sample store … left out").

### Finding 2: the bar is the theme accent, not the pane's colour

`styles/_control-bar.scss`:

| Element | Colour |
|---|---|
| Status text (`.agent-working-row--loading`, line 136) | `var(--block-identity-color, var(--accent-color))` |
| ASCII spinner (line 228) | `var(--block-identity-color, var(--accent-color))` |
| **Bar fill** (`.agent-working-row-progress-fill`, line 209) | **`var(--accent-color)`** |
| Bar track (line 203) | `color-mix(in srgb, var(--main-text-color) 10%, transparent)` |

`--block-identity-color` is set on the block frame (`blockframe.tsx:1365`) and unset when the pane has no colour,
so the text and spinner fall back to the accent. The fill was written before the row switched from the accent to
the pane's colour (SPEC_AGENT_WORKING_ROW_MONO_SUMMARY_2026_10_02) and was missed. On any pane with its own colour
the bar is a different colour from the text above it.

## 2. How long compactions really take

All Claude transcripts on this machine from the last 21 days (`compactMetadata.durationMs` on `compact_boundary`
lines, deduplicated by `uuid`):

| | |
|---|---|
| Compactions | 59 |
| p10 / p25 / **median** / p75 / p90 | 58 s / 67 s / **77 s** / 90 s / 121 s |
| Longest | 174 s |
| Within 30–90 s | 73 % |
| Median context at start | ~967k tokens (56 of 59 were above 500k) |

So "about a minute" is right in shape, but the typical run is closer to 75–80 s. A fixed 60 s first estimate would
overshoot into "longer than usual" on about 9 in 10 first compactions (only the fastest 10 % finish under 58 s).

## 3. Proposed change

### 3.1 A built-in first estimate

In `compaction-estimate.ts`:

```ts
/** The first compaction's estimate, before this build has timed any. */
export const DEFAULT_COMPACTION_MS = 75_000;

export function estimateCompactionMs(samples, contextTokens): number {
    if (samples.length === 0) return DEFAULT_COMPACTION_MS;
    // … unchanged: median × context scale, clamped
}
```

- **The first compaction uses the fixed default**, with no context scaling, as asked ("just 1m"). Scaling a constant
  by a guessed typical context would add a second guess.
- **Every later one builds on real timings**, unchanged: as soon as one compaction finishes, its sample replaces the
  default. `samplesForModel` still prefers the model's own samples and falls back to all of them, so a newly
  selected model starts from the other models' timings, not the default.
- The return type drops `| null`, so the `!est` branch in `compactionBar` goes, and the bar is always shown while
  compacting (and still never while reconnecting).
- **Tooltip:** with samples, keep "Estimate from your earlier compactions. Claude Code doesn't report compaction
  progress." With none, use "Estimate: a compaction usually takes about a minute. Claude Code doesn't report
  compaction progress." `AgentWorkingRow` needs to know which case applies, so `estimateCompactionMs` (or a small
  wrapper) should also say whether it used the default.
- The right-hand readout keeps its format: `12s / ~1m 15s`, then "longer than usual" past the estimate.

**Value: 75 s recommended.** The operator said 1 minute; the measured median is 77 s. At 60 s most first
compactions show "longer than usual" for their last 15–20 s. At 75 s about half finish inside the estimate, which is
what a median should give. If 60 s is preferred for the readout (`~1m`), it's one constant.

### 3.2 The bar in the pane's colour

```scss
.agent-working-row-progress {
    background: color-mix(in srgb, var(--block-identity-color, var(--accent-color)) 15%, transparent);
}
.agent-working-row-progress-fill {
    background: var(--block-identity-color, var(--accent-color));
}
```

The fill matches the status text and spinner exactly. The track becomes a faint tint of the same colour instead of
grey, so the empty part reads as the same bar. The overshoot sweep uses the fill, so it follows automatically.

### 3.3 Tests and spec

- `compaction-estimate.test.ts:88`: "no samples → null" becomes "no samples → `DEFAULT_COMPACTION_MS`"; add "one
  sample replaces the default".
- `AgentFooter.test.tsx:1180` ("no bar when there is no history (R3)"): becomes "a bar at the default estimate, with
  the default's tooltip". The R4 test (estimate fixed for the whole run) also covers the default → first-sample
  transition: a sample landing mid-run must not move the bar.
- `styles/working-row-indicator-color.test.ts` already asserts the identity colour for the text and the spinner;
  add the fill and track.
- Amend R3 and the out-of-scope line in `SPEC_COMPACTION_ESTIMATED_PROGRESS_AND_STREAM_FRAMES_2026_10_01.md`, with
  the data in §2 as the reason.

About 30 lines of code plus tests, all frontend.

## 4. Left out

- **Sharing timings across builds** (storing them in srv or seeding them from transcript history). The default makes
  the first compaction in a new build look right, so this is no longer needed for the bar to show.
- **Scaling the default by context size.** Almost every compaction measured started near a full 1M-token context,
  so there is no data for a smaller one (1 sample under 200k, 2 between 200k and 500k).

## 5. Coordination

`AgentFooter.tsx`'s Working row is shared ground: Agent5 changed it three times in the week of 2026-10-06 (#4492,
#4509, #4510), and other agents edit `_control-bar.scss`. Check with whoever is editing it before implementing.
