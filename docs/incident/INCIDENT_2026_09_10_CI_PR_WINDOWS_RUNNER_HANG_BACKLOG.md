# INCIDENT 2026-09-10 — `ci-pr.yml`'s required Windows leg hangs for hours, backing up every open PR

**Status:** partially addressed — Korp's PR #3198 (concurrency groups) is open, not yet
merged. The two structural gaps that let a single hang become an hours-long backlog
(no `timeout-minutes`, no concurrency groups) are analyzed below; only one is fixed so far.

**Severity:** High — every open PR against `agentmuxai/agentmux` is gated on
`check --tests + test (windows-latest)`; while it hangs, nothing merges.

**Reported by:** repo owner, flagged directly against PR #3189's Windows leg.

**Investigated by:** Korp (`korp@agentmux.local`, PR #3198) for the initial root-cause;
this document independently re-derives and extends that finding using the GitHub Actions
API run/job history directly, and adds the timeline, blockage-start point, and the
uncovered second structural gap.

Times are UTC unless noted. Host driving the initial investigation runs at UTC−7.

---

## 1. One-paragraph summary

Two separate CI-hang episodes hit `ci-pr.yml` on 2026-09-10, ten minutes apart in
lasting effect: a short one at 16:37–16:52 UTC that only stalled the *non-blocking*
`ubuntu-latest` leg (no merge impact), and a much longer one starting **23:01:40 UTC on
2026-09-10** that stalled the **required** `windows-latest` leg — this is where the
backlog the repo owner saw actually starts. At least seven independently-started
`windows-latest` jobs, whose start times were spread across more than two and a half
hours (23:01 → 01:40 on 09-11), **all died within the same ~90-second window
(04:08:05–04:09:35 UTC on 2026-09-11)** — a wall-clock-convergent "mass death," not a
per-run timeout expiring on schedule. That signature rules out a deterministic
code-level test deadlock (which would time out at a fixed *offset* from each run's own
start, not at one shared *absolute* moment) and points at a GitHub-hosted-runner-pool-side
event instead — independently confirming Korp's PR #3198 conclusion, not just repeating
it. Two more runs (01:46, 02:00 UTC) hung through a *different* mechanism — no shared
death moment, just GitHub's own default 360-minute job timeout eventually reaping them
(`cancelled`, not `failure`) around 07:46 and 08:00 UTC. Required-leg CI was materially
degraded for roughly **7–8 hours** (23:01 UTC 09-10 → ~06:35 UTC 09-11, when run
durations return to the normal 12–16 minutes). Two structural gaps in `ci-pr.yml` turned
an external flake into a multi-hour repo-wide backlog: no `timeout-minutes` on the `rust`
job (so a hung runner occupies the queue for GitHub's default six hours instead of being
killed early), and — until Korp's still-open PR #3198 — no `concurrency:` group (so every
push spawned a brand-new run without cancelling the now-superseded previous one,
multiplying load on the same constrained pool during the incident).

## 2. Timeline

| UTC | Event | Source |
|---|---|---|
| 2026-09-09 05:12 → 2026-09-10 16:37 | Baseline: ~35 hours of `ci-pr.yml` runs, consistently 12–16 min, zero anomalies (checked across 300 fetched runs) | Actions API, `runs?per_page=100` ×3 |
| 2026-09-10 14:38:17 | `feat(srv): redirect skill/MCP-server catalog reads and writes to identity_store` (#3183) merged to `main` | `git log origin/main` |
| 2026-09-10 16:01:37 | `feat(mcp): add PtyShell — a real PTY-backed interactive shell, no UI required` (#3177) merged to `main` | `git log origin/main` |
| 2026-09-10 16:37:20 | Run `34503227484`: non-blocking `ubuntu-latest` leg starts, does not complete until 22:38:18 (**6h01m**, `cancelled`). Required `windows-latest` leg on the *same run* finishes normally at 16:58:12 — **no merge impact from this one.** | Actions API, run + jobs |
| 2026-09-10 16:52:58 | Run `34504802608`: a second `ubuntu-latest`-only hang, `cancelled` after 348.8 min. Same "non-blocking only" shape. | Actions API |
| 2026-09-10 16:53 → 22:42 | ~13 further runs, all normal (13–23 min) — the first two hangs did not recur immediately and did not yet represent an ongoing incident. | Actions API |
| 2026-09-10 23:01:40 | Run `34540247974` starts. **This is where the backlog actually begins**: its `windows-latest` leg (required) does not complete until 2026-09-11 04:08:06 — 5h06m later, `failure`. | Actions API, run + jobs |
| 2026-09-11 00:12–01:40 | Six more runs start (`34545459640`, `34546072183`, `34546854544`, `34547266894`, `34547983179`, `34551584252`), each with its own `windows-latest` leg hanging from its own start time | Actions API |
| 2026-09-11 04:08:05–04:09:35 | **All seven** of the above `windows-latest` jobs die within a 90-second window, every one `conclusion: failure` — independent of how long each had already been running (durations at death ranged from ~2h28m to ~5h06m). This is the "mass death" signature. | Actions API, per-job timestamps |
| 2026-09-11 01:46:28 / 02:00:16 | Two further runs (`34552004567`, `34552901880`) hang on `windows-latest`, but are **not** caught by the 04:08 mass-death moment (it had already passed) — each instead runs to almost exactly 360 minutes and is `cancelled` by GitHub's own default job timeout, at 07:46:44 and 08:00:54 respectively. | Actions API |
| 2026-09-11 ~02:48–06:35 | Quiet gap in the run history (no pushes recorded in this window) | Actions API |
| 2026-09-11 06:35 onward | Runs resume at the normal 12–16 minute duration. Several near-duplicate runs fire minutes apart from quick successive pushes (06:35:49, 06:38:28, 06:39:55, 06:45:14) — the exact "no concurrency group" symptom Korp's PR targets, still present at this point. | Actions API |
| 2026-09-11 07:18:30 | Korp opens PR #3198, `korp/ci-concurrency-groups` — root-cause writeup (matches this document's §3 independently) + concurrency groups for `ci-pr.yml` and both nightly workflows. **Still open, not merged as of this writing.** | `gh pr view 3198` |
| 2026-09-11 07:23:13 | Codex P1 on #3198: `github.head_ref` alone can collide across unrelated PRs sharing a branch name; Korp's follow-up commit keys the group on `github.event.pull_request.number` instead. | PR #3198 commit `9c28a30` |

## 3. Root cause

**Primary cause — external, not code-level.** The defining evidence is the mass-death
convergence at 04:08:05–04:09:35 UTC: seven `windows-latest` jobs, started at seven
different times spanning over two and a half hours, all terminated within the same
90-second window regardless of individual elapsed time. A deterministic hang inside this
repo's own test suite (a deadlock, an infinite retry loop, a blocked I/O read) would time
out at a fixed **offset** from each job's own start — it has no way to know what wall-clock
time it is. Only a shared external event — a runner-pool-side incident, a mass reap of a
bad runner image, a network-partition-style failure hitting the Actions control plane —
explains termination converging on one **absolute** moment across unrelated jobs. This
independently confirms Korp's PR #3198 conclusion (arrived at by comparing two of these
runs) using five additional data points from the same episode.

**Ruled out:** a regression in a specific merged PR. `feat(mcp): add PtyShell` (#3177)
merged at 16:01:37 UTC, close to first anomaly's 16:37 start, and `main`'s own `push`
trigger for `ci-pr.yml` means that merge did kick off a run — but that first anomaly only
ever affected the non-blocking Ubuntu leg, and the mass-death signature at 04:08 rules out
any single commit as the cause for the blocking episode: the seven affected runs all
carried whatever was on `main`/each PR's branch at their own respective start times, and
still converged on one shared death moment unrelated to what any of them were running.

**Contributing cause #1 — no `timeout-minutes` on the `rust` job.** `ci-pr.yml`'s `rust`
job (the one running `cargo check --workspace --tests` + `cargo test --workspace` on both
platforms) sets no explicit timeout. GitHub's default for a job with none specified is
**360 minutes** — exactly the ceiling `34552004567` and `34552901880` hit (`cancelled` at
almost precisely 360m00s, not caught by the earlier mass-death moment since it had already
passed). Every hang in this incident occupied the runner queue for hours it did not need
to: a `timeout-minutes: 20` (roughly 1.5× the ~13-minute normal Windows-leg duration)
would have failed each of these runs within 20 minutes of onset instead of up to 6 hours,
regardless of which specific external event caused the hang.

**Contributing cause #2 — no `concurrency:` group (until Korp's still-open PR #3198).**
`ci-pr.yml` had no `concurrency:` block at all before PR #3198. Every push to a PR — even
a trivial fixup pushed seconds after the previous one — spawned a brand-new, full-cost run
without cancelling the now-superseded prior run. Korp's PR cites 88 `ci-pr.yml` runs firing
in a sampled 21-hour window; this document's own 06:35–06:45 UTC window (§2) shows the same
pattern directly (four runs within 10 minutes, evidently from repeated pushes). This does
not cause a hang, but it multiplies how many jobs are competing for the same constrained,
shared `windows-latest`/`ubuntu-latest` pool at any moment — worse exposure to exactly the
kind of runner-pool flakiness that caused this incident, and more redundant work stacked up
in the queue once a real hang does start eating capacity.

## 4. Why this became a repo-wide "backlog," not just a slow PR

`windows-latest`'s `check --tests + test` job is the sole **required** status check that
gates every PR against `agentmuxai/agentmux`. From 23:01 UTC on 2026-09-10 until roughly
06:35 UTC on 2026-09-11, nine separate PR-CI runs (the seven mass-death casualties plus
the two reaped at the 360-minute ceiling) each held that gate open for hours instead of
the normal ~13 minutes. Because there was no `concurrency:` group yet, agents working
through the night kept pushing follow-up commits to their own PRs, each spawning another
full-cost run that competed for the same limited pool rather than superseding the stuck
one — compounding queue pressure on top of the hangs themselves. The net effect: for
roughly 7–8 hours, no PR against this repo could merge on a green required check, and the
PRs that happened to be open or pushed-to during that window inherited a multi-hour wait
that had nothing to do with their own diff.

## 5. What's fixed, what isn't

**Addressed by Korp's PR #3198 (open, unmerged):** the second contributing cause.
Concurrency groups on `ci-pr.yml` (keyed on PR number, cancel-in-progress) stop redundant
runs from stacking up on repeated pushes; separate groups on the two nightly workflows
(cancel-in-progress: false — a real nightly run should finish, not be cancelled by its own
schedule re-firing) stop nightly from competing with itself. The PR is explicit that this
does **not** give nightly or PR CI dedicated runner capacity — GitHub-hosted runners are
one shared pool per repo regardless of concurrency group; true isolation would need
self-hosted runners or GitHub Enterprise runner groups, neither of which this repo has.

**Not yet addressed by anything:** the first contributing cause. No PR currently adds
`timeout-minutes` to the `rust` job in `ci-pr.yml` (or its nightly counterparts). The next
runner-pool flake — and per this incident's own two-day sample, this is not a one-off —
will still be free to occupy the required check for up to six hours before GitHub's own
default reaps it.

**Not yet addressed, and out of scope for the concurrency-group fix — the repo owner's
separate, standing ask:** get the normal-case required leg under 10 minutes.
Independent of this incident, the Windows leg's *baseline* (non-hung) duration is already
~11–14 minutes (§2's baseline sample) — over the 10-minute target — driven by `cargo check
--workspace --tests` + `cargo test --workspace -- --test-threads=1` (serial by design, for
test-isolation reasons — see `ci-pr.yml`'s own header comment) plus the CEF C++ build via
CMake/Ninja that `cef-dll-sys`'s `build.rs` triggers on every `cargo check`. Splitting
whichever tests are the largest contributors to that baseline into the existing
`ci-nightly-build.yml` (already running the full cross-platform matrix on a schedule) is a
distinct piece of work from anything in this incident or in PR #3198, and needs its own
per-test timing breakdown before deciding what moves — not attempted here.

## 6. Recommendations

1. **Add `timeout-minutes` to `ci-pr.yml`'s `rust` job** (and the nightly workflows'
   equivalent jobs) — the single change that would have bounded every hang in this
   incident to tens of minutes instead of hours, independent of whatever external cause
   triggers the next one. Suggest ~20 min for the PR leg (normal baseline ~13 min) and a
   more generous cap for nightly's three-platform matrix.
2. **Merge Korp's PR #3198** — reduces redundant queue pressure on the shared runner pool,
   real mitigation even though it does not touch the hang mechanism itself.
3. **Separately, profile `cargo test --workspace` to find what's actually consuming the
   ~13-minute Windows-leg baseline**, and move the slowest/most "rigorous" tests (by the
   repo owner's own framing) to run only in `ci-nightly-build.yml`, gated by `#[ignore]` +
   `cargo test -- --include-ignored` in nightly, or an analogous split — this is the actual
   path to a sub-10-minute required leg and is unrelated to this incident's root cause.

## 7. Evidence index

- GitHub Actions API, `repos/agentmuxai/agentmux/actions/workflows/ci-pr.yml/runs` (300
  runs fetched across three pages, 2026-09-09T05:12Z–2026-09-11T08:10Z) and
  `.../actions/runs/{id}/jobs` for the 16 runs exceeding 30 minutes.
- Korp's PR: https://github.com/agentmuxai/agentmux/pull/3198 (`korp/ci-concurrency-groups`).
- `git log origin/main --since="2026-09-10 14:00" --until="2026-09-11 05:00"` for the merge
  correlation check in §3.
- `.github/workflows/ci-pr.yml` (current, at the time of writing — confirms no
  `timeout-minutes` anywhere in the file).
