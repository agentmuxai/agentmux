# Report: the memory soak gate — what it measures, what is known about its noise, and what the operator has to set

**Date:** 2026-10-18
**Status:** analysis — the gate ships report-only; no line is set and nothing can fail on a number until the operator sets one.
**Follows:** `PLAN_MEMORY_BUDGET_CLOSE_THE_GAP_2026_10_18.md` (track A: "a soak job that fails a PR when the renderer or the total grows past a set line").

## 1. What shipped

- `.github/workflows/memory-soak.yml`: every night, after the nightly artifacts build, a Windows runner downloads that build's portable, runs the memory lab on it (12 agents over 4 tabs, 10 rounds, seed 1) and runs the gate. The numbers are kept as an artifact for 90 days and printed in the job summary.
- `scripts/perf/memory-lab/gate.mjs` and `budget.json`: the lines. `budget.json` names the gated workload, a line in MB for the app total, the renderers and srv, and a mode. As shipped, every line is `null` and the mode is `report`, so the job reports and cannot fail on a number. In `enforce` mode it exits 1 over a line, and 2 when the result is not the gated workload or a line's number could not be read: a gate that passes on missing data is not one.
- It is not a required check and does not run on PRs. The PR CI builds no CEF app (it compiles and tests), so a per-PR soak would need a full ~25 minute build per push on top of a ~10 minute run.

## 2. What is known about the noise

All on one development machine (Windows, one build), not on a CI runner:

| measurement | runs | spread |
|---|---|---|
| app total, 4 agents, 20 rounds, forced GC, control runs | 883, 876, 910 MB | about 2% |
| renderers, same runs | 648, 622, 646 MB | about 2% |
| srv, same runs | 80, 101, 109 MB | about 15% |
| app total, idle for 10 minutes after the same load | 764, 758 MB | under 1% |

Within a run, the unforced curve is noisy: one control curve spiked 1153 MB at round 12 and was back to 939 MB by round 15, which is why the gate reads a forced-GC median of three samples and not a point on the curve.

What is **not** known: the spread on a GitHub-hosted runner (different CPU count, memory pressure and GPU path from this machine), or between nightly builds of different commits. A line chosen from the numbers above would be a guess about a machine the job does not run on.

## 3. What the operator needs to decide

1. **Which numbers are gated.** Proposed: the app total and the renderers. srv is reported but not gated: its run-to-run spread (15%) is wider than the changes worth catching, and the one srv difference seen between a recycled and a control run did not repeat.
2. **How the lines are set.** Proposed: let the job run report-only for about two weeks (14 nightlies), then set each line at the highest of those runs plus a margin the operator picks (10% is a starting point), and tighten it as the budget work lands. An absolute line at the 500 MB target would fail every night today (the same 12-agent load measures about 1.35 GB).
3. **Where it may block.** Proposed: stay nightly and informational (a failed nightly opens a conversation, not a blocked PR) until the lines have held for a few weeks; only then consider a PR-time check on PRs that touch the renderer, which needs a build per push.
4. **Who is told.** Not built: a failing nightly in `enforce` mode fails the workflow run, which shows in the Actions tab; a message to a channel is a separate decision.

Until 1 and 2 are answered, `budget.json` stays as shipped.
