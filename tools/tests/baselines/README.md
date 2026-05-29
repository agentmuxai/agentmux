# Input-latency bench baselines

Committed baselines for the input-first latency benches, consumed by
`tools/tests/bench-aggregate.mjs` (Phase 0.1, discussion #1161).

## Why baselines, not absolute thresholds

Sub-50 ms keystroke timing is high-variance and **hardware-dependent**. An
absolute gate ("fail if P95 > 50 ms") either false-positives on slow/shared
runners or passes trivially on fast dev machines — so it gets disabled and
becomes theater. Instead we gate on **delta vs a baseline captured on the same
pinned device**: a change regresses only if it moves the median-of-per-run-P95
by more than `--tolerance-pct` (default 20%).

## File schema

One file per (bench, device). Naming: `<bench>.<device>.json`.

```json
{
  "metric": "medianP95",
  "value": 12.4,
  "device": "thinkpad-x250",
  "bench": "agent",
  "metricPath": "keystroke.stats.p95",
  "runs": 5,
  "capturedAt": "2026-05-29T19:00:00.000Z",
  "note": "median of per-run P95. Recapture after a deliberate perf change."
}
```

`value` is the **median of the per-run P95s** — the most stable headline number.

## The pinned reference device (REQUIRED — this is the manual step)

Baselines and CI gating are only trustworthy on **dedicated, stable hardware**:

- A **self-hosted runner on a low-end Windows box** (the worst realistic user
  machine), not GitHub-hosted shared VMs (too noisy for this timing).
- Add a **macOS** reference device too (v0.40.0 made `task dev` launchable on
  macOS — AgentA #1169).
- Keep the device otherwise idle during a run; document its exact spec here.

> **No baselines are committed yet** — they must be captured on the pinned
> device(s) once those runners exist. Until then `bench-aggregate` reports
> `no-baseline` and never blocks.

## Capturing / updating a baseline

On the pinned device, with AgentMux running and the relevant pane open:

```bash
# Agent composer (agent pane open):
node tools/tests/bench-aggregate.mjs --bench agent --runs 5 \
  --baseline tools/tests/baselines/agent.thinkpad-x250.json \
  --update-baseline --device thinkpad-x250 -- --cdp-port 9223 --count 200

# Terminal (terminal pane open):
node tools/tests/bench-aggregate.mjs --bench term --runs 5 \
  --baseline tools/tests/baselines/term.thinkpad-x250.json \
  --update-baseline --device thinkpad-x250 -- --cdp-port 9223
```

Commit the resulting JSON. Recapture (and commit) only after a **deliberate**
perf change, with a note explaining the move.

## Reporting → gating promotion path

1. **Report (now):** run in `--mode report` (default) on PRs touching input
   paths. Never blocks; surfaces the delta + run-to-run CoV for humans.
2. **Characterize variance:** collect a few weeks of reporting data; confirm
   the pinned runner's per-run CoV stays under `--max-cov` (0.25).
3. **Gate (later):** flip to `--mode gate`. A change then blocks merge only on a
   >tolerance regression or a `noisy` runner (CoV over budget) — never on an
   absolute number the device can't reproduce.

Gate on the **stable** signal first (`agent-keystroke` synchronous-body P95,
which is low-variance) before the noisier end-to-end keystroke-to-paint.
