# CI: why the Linux test job stalls on memory again, and the cut

**Status:** analysis
**Date:** 2026-11-16
**Scope:** the `check --tests + test (ubuntu-latest)` job of `ci-pr.yml` (the PR lane; the merge queue's lane is compared), `scripts/ci-nextest-run.sh`.
**Data:** the 1,806 Ubuntu test jobs of the 1,000 most recent PR runs and 1,000 most recent merge-queue runs (2026-11-11 to 11-16, from the Actions API); the resource traces of three stalled runs; the sampler lines in the logs of 16 passing PR runs.

## 1. What happens

The job runs `scripts/ci-nextest-run.sh`: three `cargo nextest run` calls in one cgroup with a 14 GB memory ceiling on a runner with 15.6 GB, and a sampler that writes a trace line every 5 seconds. The first call builds the core's test harness (`--lib`), the second the server layer's (`--bin agentmux-srv`), the third (`--profile ci`, every target of the eight packages) builds the rest and runs the tests.

A stalled run is one where the machine freezes while rustc holds nearly all of the memory: the sampler's last lines before the freeze show the figures standing still (the cgroup at its ceiling, memory pressure `psi_full10` 35 to 98%), and then the sampler reports it was starved for 799 to 934 seconds. The attempt's own time limit ends it; the retry stalls again or the job's 50-minute limit cancels it with no log.

## 2. How often, and since when

PR lane, jobs that ran to a result (success, or cancelled at 40 minutes or more):

| Period (UTC) | What changed | Stalled |
|---|---|---|
| 11-11 to 11-14 05:08 | | 17 of 261 (6.5%) |
| 11-14 05:08 to 11-15 23:27 | `05b264b52b`: two LLVM threads and the 14 GB ceiling | 2 of 199 (1.0%) |
| 11-15 23:27 to 11-16 02:01 | `73e789f938`: the core is its own package | 2 of 9 (22%) |
| 11-16 02:01 to 20:20 | `2725c6925e`: the core and the server layer are built once, by naming the same packages | 17 of 63 (27%) |

The merge queue's lane (no last build, see below) stalled 8 of 476 before 11-14 05:08 and 0 of 430 since. Passing runs took a median of 17.9, then 16.4, then 20.6 minutes in the three periods after the first; each of the 38 stalled PR jobs cost 55 minutes and, with its re-run, an agent an hour.

## 3. Where: the last build

The sampler lines in passing jobs' logs, split by the three `Finished` lines of the build:

| Run (UTC) | 1st build peak | 2nd build peak | last build + run peak | swap used, last | `psi_full10` max, last |
|---|---|---|---|---|---|
| 11-15 05:17 | 8.6 GB | 13.1 GB | 1.7 GB | 14 MB | 0.09 |
| 11-15 15:27 | 8.9 GB | 14.1 GB | 1.5 GB | 36 MB | 0 |
| 11-16 04:40 | 11.5 GB | 8.6 GB | 14.1 GB | 362 MB | 11.7 |
| 11-16 08:55 | 11.4 GB | 9.0 GB | 14.5 GB | 999 MB | 24.8 |
| 11-16 10:11 | 11.7 GB | 9.2 GB | 14.5 GB | 484 MB | 19.7 |
| 11-16 15:29 | 11.6 GB | 9.1 GB | 15.0 GB | 821 MB | 41.5 |
| 11-16 19:36 | 11.8 GB | 9.2 GB | 14.7 GB | 1,105 MB | 36.8 |

(Peaks are `demand`: memory used plus swap used. The 11-15 rows are from before the core was split out; their builds had other contents, and the peak was in the second one.) Since `2725c6925e` the last build is the peak, at 14.1 to 15.0 GB on every passing run, against a 14 GB ceiling: every run touches the limit and a stall is a coin toss. The traces of the stalled runs have the same shape, in the same place, 8 to 9 minutes into the attempt:

| Run | Moment | `demand` | cgroup | biggest process | `mem_avail` | swap used | `psi_full10` | then |
|---|---|---|---|---|---|---|---|---|
| PR 7551, 43924142203, attempt 1 | 18:11:11 | 14,306 MB | 14,073 MB | rustc 9,374 MB | 2,237 MB | 842 MB | 35 to 98% | frozen figures; sampler starved 799 s |
| main `f9df7891e7`, 43954169035 | 21:02:55 to 21:03:05 | 14,341 MB | 13,652 MB | rustc 9,140 MB | 1,361 MB | 0 to 1,029 MB | 0.01 to 35% | sampler starved 934 s; attempt 1 ended by its 1,500 s limit |
| main `95dd4691ba`, 43946172051 | | | | | | | | no trace: the runner never answered; cancelled at 55 minutes |

Two rustc run at once there: the biggest holds 9 to 10 GB and the cgroup holds about 4.5 GB more than it does. The logs show the core and the server layer both compiling in the last build, started 5 seconds apart (the log of 11-16 19:36: `agentmux-srv-core` at 19:45:25, `agentmux-srv` at 19:45:30), so they are the likely pair; the trace did not name the second process, which `top2=` now does. `CARGO_BUILD_JOBS=2` allows both. The first and second builds, each one big rustc, peak at 11.8 and 9.2 GB and have never stalled. The merge queue's last call builds only the targets the first two already built, which is why it is clean.

## 4. The cut

Chosen: **the last build of a PR attempt runs with one job** (`CI_FINAL_JOBS`, default 1; a separate `--no-run` call so that the test run compiles nothing), **the retry builds everything with one job**, and **the trace line gains `top2=`** (the second biggest process, so the next trace names the pair).

Why this and not the others:

- *Less debug info:* already `line-tables-only` for the dev and test profiles (`ci-pr.yml`); `0` would lose backtrace lines in test failures and was not measured; nothing in the traces suggests it takes 14.7 GB to 12.
- *Fewer jobs everywhere:* the first two builds peak at 11.8 and 9.2 GB with two jobs and have not stalled; one job there costs minutes for nothing.
- *Splitting the server layer's test crate:* the right long-term cut for the build's size, and a change to the code and its tests; not a one-day change, and the last build's two big compiles come from how cargo unifies the call's targets, not from one file.
- *Building the two heavy targets one after the other:* that is what one job does for the last build; naming them in separate calls would change the features each is built with and compile the core again (the lesson of 2026-11-15, in the script's header).
- *A larger runner:* an 8-core, 32 GB runner would fit, at a per-minute price above the standard runner's for every PR run, and needs an organization setting; kept as the fallback if one job does not leave room.
- *A tighter watchdog:* the traces cannot tell the stall from a survivable dip early enough. The stalled runs at 21:02:55 and 18:11:11 look like the passing runs of 11-16 08:55 (swap 999 MB, `psi_full10` 25) and 19:36 (swap 1,105 MB, 37) until the machine freezes, within one 5-second sample, and a frozen machine cannot run a watchdog. Only a lower peak removes the stall.

Expected effect (an estimate, to be read off the PR's own run): one rustc at a time in the last build, a peak near the first build's 11.4 to 11.8 GB, which has never stalled, and 2 GB or more of margin under the ceiling. Cost: the last build's two compiles run one after the other, about 1 to 2 minutes of a 17-minute median job (the merge queue is untouched).

## 5. What the PR's own run shows

Read the sampler lines in the job log (`[resources]`, now with `top2=`): the peak `demand` after the second `Finished` line should be near 12 GB, `top2` should never be a second rustc over 1 GB in that build, and `psi_full10` should stay under 5. The wall time of the step is the number to compare with the 20.6-minute median since `2725c6925e`.

## 6. Not changed here: a stalled gate job on main reports nothing

A push to main whose Ubuntu leg stalls ends 55 minutes later with `CI required` failed and `main status` green: its first line is `main has moved on to <sha>; that push's run reports main's state`. main moves every few minutes, so by the time a stalled run ends its commit is not main's head any more, and the step leaves it to the newer commit's run, which is judged on its own result. That rule is right for a code failure (the newer commit may have fixed it) and wrong for an infrastructure stall, which says nothing about the commit and is never repeated by the newer run. To report it, `main status` would need a second signal that does not depend on being the head: for a `CI required` failure whose failing jobs are all cancelled (not failed), comment on a standing `ci-stalled` issue with the run and the job, without calling main red. Both main runs of today (`95dd4691ba` and `f9df7891e7`) ended this way. That is a change to `ci-pr.yml` and is left to the CI owner.
