# Spec: a nightly performance regression check

**Status:** active. Phase 1 (the workflow) shipped in #5168, Phase 2 (the issue, `perf-run-failed`) in #5187, Phase 3 (history, charts, drift) in #5192. Remaining: the email section (§4.7, Camper, in `a5af/shared-infrastructure`) and the later decisions in §5.5.
**Date:** 2026-10-19
**Author:** Camper (coordinator); owner of the work: Loap-3
**Builds on:**
- `.github/workflows/memory-soak.yml` and `scripts/perf/memory-lab` (Loap-3: #5085 onward, the lab's metrics in #5135–#5155);
- `PLAN_NIGHTLY_RED_LEGS_ARE_HEARD_2026_10_18.md` (the `nightly-red` issue, #5129, #5131), whose issue logic this reuses.

## 1. Ask

The owner, 2026-10-19: "can we leverage our nightly CI on github to run performance regression testing? do we need a UI for it?", then "ok, lets get this into a formal spec. it would run alongside and seperate from the nightly builds?", then "we'd also like to add the results to the nightly github email, see a5af/shared-infrastructure"

## 2. What exists

`memory-soak.yml` runs every day at 13:30 UTC on `windows-latest`:
- It fetches the Windows portable that `ci-nightly-artifacts.yml` built that morning, by looking up the run, not through a `workflow_run` trigger.
- It runs the lab once: 12 agents, 10 rounds, seed 1.
- It writes `gate.mjs`'s table to the job summary and uploads `result.json`, `summary.md` and the instance log for 90 days.

It measures, but it compares against nothing: every line in `budget.json` is null (report only), and a GitHub runner's numbers vary between machines and days, so a fixed line would fire falsely. It also tells nobody when something gets worse.

The lab already records everything a regression check needs (#5135–#5155):
- memory and CPU per process group;
- round time, slow page commands, page long tasks and timer lag;
- per-send timings;
- bytes written;
- srv's store-lock holds.

`compare.mjs` already flags a side that is worse past a margin when the two sides' ranges don't overlap: +10% for app and srv CPU and bytes written, +20% for other CPU groups and round time.

## 3. Goals and non-goals

**Goals**
- Each night, find out whether the day's merges made AgentMux slower, heavier, or more disk-hungry, and say so where people will see it.
- Name the suspects: the merges between the two builds compared.
- Keep a history, so a slow drift that no single night shows is visible too.

**Non-goals**
- Not a PR gate. Nothing here blocks a merge. A PR-time perf check is a later decision.
- Not part of the nightly build and test. It never makes `ci-nightly-build.yml` or `ci-nightly-artifacts.yml` slower or red.
- Not a reproduction of slow disks or large displays. Those stay manual lab work (`--disk-load`, Posa's Windows 10 machine).

## 4. Design

### 4.1 Separate from the nightly builds

- **Its own workflow:** `perf-regression.yml`, which replaces `memory-soak.yml`'s job. It has its own concurrency group, its own runner, and a schedule after the nightly artifacts.
- **It only reads the builds:** it downloads the `windows-portable` artifacts the nightly artifacts workflow already made, and never builds AgentMux itself.
- **It can't affect the others:** a failure or timeout here doesn't touch the build workflows' status, and theirs doesn't touch this, except that with no new build there is nothing to compare (§4.6).
- **It runs on a standard GitHub-hosted runner:** free for a public repository.

### 4.2 What it compares

- **B:** today's nightly portable, the latest successful `ci-nightly-artifacts` run.
- **A:** the previous successful nightly's portable, the run before B whose artifact still exists. If A's artifact has expired or is missing, the job runs B alone, as the soak does today, and says so.
- **Both on the same runner in the same job,** so machine and day variance cancels out.
- **The job records** both builds' commits, by the artifact run's `headSha`. Nightly portables don't bump the version, so the version can't tell them apart. It also records the merges between them, from GitHub's compare API on the two SHAs (the job's checkout is shallow), with titles and PR numbers.
- **The artifacts are kept 7 days,** so an A older than that is missing and handled as above.
- **Manual dispatch** can name A and B: two run ids, or a release tag for A, to bisect or to compare against a release.

### 4.3 The runs

- Interleaved A, B, A, B: **4 runs per side by default.** The workload is the lab's standard, with its version recorded; `compare.mjs` refuses to mix versions.
- **Size** (Phase 0, measured on `windows-latest`): 12 agents, 10 rounds. One run is about 4.7 minutes, so 4 per side is about 40 minutes of lab time plus about 5 minutes of setup. With the confirmation pair it is about an hour. 4 agents isn't used: it is noisier on memory (29–33% run to run, against 13% at 12).
- **The runs use `scripts/perf/memory-lab/ab.mjs`** (#5165), the same interleaved A/B the lab uses by hand.
- **Confirmation:** if the first matrix flags anything, the job runs one more A/B pair. The issue opens only if the flag holds on all the runs together. One noisy run must not open an issue.

### 4.4 What it flags

`compare.mjs`'s existing rules decide; nothing new is invented here.

Margins from Phase 0's noise on the runner (3 runs, spread as highest−lowest over the median):

| Measure | Noise on the runner | Flag when B is worse by | And |
|---|---|---|---|
| srv bytes written | 0.2% | more than 3% | the ranges don't overlap |
| App, srv and renderer CPU seconds | 5–6% | more than 10% | same |
| GPU CPU, round time (median) | 9–12% | more than 20% | same |
| Memory (app, renderers, srv) | 13% | report only for the first two weeks, then more than 10% | same |
| srv's longest store-lock hold, slow page commands, page long tasks | not yet known | reported, not flagged until a few weeks of data set their noise | |

srv's bytes written is the best signal: almost noiseless, and it catches the write-volume regressions behind #5141. The margins are re-tuned after the first week of nightlies.

Every measure appears in the summary whether or not it flags. Margins live in `budget.json`, so they change without touching code. The owner sets them after a few weeks of data.

### 4.5 Who hears about it

- **The job summary:** a table of every measure, A, B, the change and its range, plus the merges between A and B.
- **A `perf-regression` GitHub issue,** opened on a confirmed flag. It names each flagged measure, the size of the change, the two builds, the merges between them, and links to the run and its artifacts. Later flagged nights comment on it; a clean night closes it.
  - The issue logic is `scripts/ci-nightly-status.mjs` (#5129, #5131), generalised to take a label and a title. It already handles one issue per streak, a cancelled run saying nothing, and a timeout read from annotations.
  - Agents watching the repository are notified, as with `nightly-red`.
- **No `perf-regression` issue** for an unflagged night, a night with nothing to compare, a night whose confirmation pair did not finish (a flag nobody confirmed is not reported), or a run that didn't finish.
- **A night that didn't finish** (the lab failed, or the job hit its timeout) opens one `perf-run-failed` issue ("nightly perf run failed", naming the failed steps), with a comment for each later failed night; the first night that finishes closes it. A separate label rather than `nightly-red`, so the nightly's own logic never closes it. It reuses `decide()` from `scripts/ci-nightly-status.mjs`, and reads a cancelled job's annotations to tell a timeout from a cancel, as the nightly does. A plain cancel says nothing. Without it, a check that fails every night would be a red run nobody watches, and the history would quietly stop growing. The `failed` job runs on main only.
- **Only a run on main writes** to issues and to `perf-data`; a dispatch from another branch prints what it would have done.

### 4.6 History and trends

- **Each night's medians,** one JSON line per night with the date, both builds and every measure, are appended to `perf/history.jsonl` on a `perf-data` branch. It is a plain branch, written only by this job (`contents: write` for this workflow alone), and has no effect on `main`.
- **The job summary draws the last 30 nights** of the main measures (app memory, app and srv CPU, round time, srv bytes written) as Mermaid `xychart-beta` charts, which GitHub renders in markdown. So the trend is visible with nothing to host.
- **A slow drift,** where no single night flags but the last 7 nights' median is more than 15% above the 30 nights before, gets one comment on the open issue or a new one, worded as drift.
  - It looks at the five main measures (app memory, app and srv CPU, round time, srv bytes written), each night's B figure.
  - It compares only nights on the newest night's workload version, 12 agents and 10 rounds, one night per date, and needs at least 5 recent and 10 earlier nights, so nothing is said in the first weeks.
  - It is said once, on the night the drift starts, not on every night it lasts (a night where the median is already over the line the night before adds nothing). A new issue is titled "nightly performance drift"; both kinds carry the `perf-regression` label.
  - A drifting night is not a clean one: it does not close the open issue, even when nothing flags.
- **Only the standard workload is recorded.** A manual run with other panes or rounds prints the trend but writes no line, so a test dispatch cannot skew the history.
- **A rerun of the same workflow run** replaces its own line (the run id is the key), it doesn't add a second.

### 4.7 The nightly GitHub email

The daily GitHub email (`gh-reporter` in `a5af/shared-infrastructure`, a private repository) already has sections for the nightly build, the nightly artifacts and the nightly release. It gains a **Nightly performance** section:

- **The run's status:** the latest `perf-regression.yml` runs, with the same rows and helper (`get_nightly_build_status`) the other nightly sections use.
- **The numbers:** the last line of `perf/history.jsonl` on `perf-data`, read through the contents API. It shows the two builds compared, then a row per main measure: A, B, the change, and a flag mark.
- **The open `perf-regression` issue,** if there is one, linked.
- **Missing data:** an error stub, as the other sections show, if the data can't be read. A note if the newest entry is more than 36 hours old.
- **Timing:** the email goes out at 02:00 Pacific (about 09:00–10:00 UTC). The perf job runs after the morning's nightly artifacts, around 13:30 UTC or later. So each email reports the previous day's check, and says which date it is.

The section reads only the published history line and the issue, so the email needs no access to artifacts. Its format is §4.9's.

### 4.8 A UI

**None is needed to start.** The job summary (table and charts) and the issue are the interface. Later, if the owner wants trends inside AgentMux, a sandboxed widget pane can read `perf-data`'s `history.jsonl` from GitHub: one `net:` permission and no backend change. That is a separate decision after a few weeks of data.

### 4.9 The history line's format

One JSON object per line in `perf/history.jsonl`. The perf job writes it; the email and any later widget read it. Fields are added, never renamed.

```json
{
  "date": "2026-10-20",
  "run_id": 123, "run_url": "https://github.com/agentmuxai/agentmux/actions/runs/123",
  "a": { "commit": "71a3e2300", "version": "0.59.34", "artifacts_run_id": 111 },
  "b": { "commit": "5ba04f91f", "version": "0.59.34", "artifacts_run_id": 112 },
  "workload": { "version": 2, "panes": 12, "rounds": 10, "runs_per_side": 5 },
  "measures": {
    "app_mb":        { "a": 640, "b": 652, "change_pct": 1.9,  "flag": false },
    "srv_cpu_s":     { "a": 38.0, "b": 44.1, "change_pct": 16.1, "flag": true }
  },
  "flags": ["srv_cpu_s"],
  "confirmed": true,
  "issue": 5200
}
```

- `measures` keys: `app_mb`, `renderer_mb`, `srv_mb`, `app_cpu_s`, `srv_cpu_s`, `renderer_cpu_s`, `gpu_cpu_s`, `round_ms`, `srv_written_mb`, `store_lock_max_ms`, `slow_page_commands`, and, added since the lab measures them, `heavy_srv_written_mb`, `heavy_srv_storage_written_mb`, `open_first_screen_ms`, `open_heavy_first_screen_ms`, `open_longest_task_ms`. A measure the lab produced no figure for is left out of the line, not written as null.
- `commit` is the build's identity (the artifact run's `headSha`); `version` is informational only, since nightlies share a version.
- `a`, `b`, `change_pct` and `flag` are medians and the §4.4 rules; `a` and `b` are rounded to two decimals and `change_pct` to one.
- The top-level `a` and `b` (the builds) are `null` for `a` on a night without a previous build; then `measures` is `{}` (B was measured alone, there is nothing to compare) and `flags` is `[]`. The email says so.
- `runs_per_side` counts every run used, the confirmation pair included.
- `confirmed` is true when a measure flagged and the confirmation pair's runs were part of the comparison. It is false for an unflagged night.
- `version` comes from the portable's file name (`agentmux-<version>-x64-portable.7z`).
- `date` is the UTC date of the run, which is the day after the build it compares. `run_id` is the key a rerun replaces.
- `issue` is the open `perf-regression` issue's number after the night's issue step ran (so `null` on a night that closed it), or `null`.

## 5. Plan

0. **Measure the runner** (done, 2026-10-19, Loap-3): 12 agents, about 4.7 minutes a run; the noise per measure is in §4.4. It decided §4.3's size and §4.4's margins.
1. **The workflow:** A/B download, interleaved runs, `compare.mjs`, summary table, merges between builds. Report only: no issue yet. Shipped in #5168.
2. **The `perf-regression` issue,** with the confirmation pair, reusing `decide()` from `ci-nightly-status.mjs`, plus the `perf-run-failed` issue. Shipped in #5187.
3. **History:** `perf-data`, the Mermaid charts, and the drift check. Shipped in #5192.
4. **The email section** (`gh-reporter`, a PR in `a5af/shared-infrastructure`), once Phase 3 writes the history line. Owner: Camper.
5. **Later, each a separate decision:**
   - a Linux leg (`ubuntu-latest`);
   - macOS, once runners are reliable enough;
   - the widget pane;
   - turning a measure from report-only into a flag (the store-lock and page metrics).

## 6. Risks

- **Runner variance hides a real change, or invents one.** Mitigations: the A/B in one job, the non-overlap rule, the confirmation pair, and margins tuned from data. A change smaller than the noise (about 3–5% for srv and app CPU, about 15% for renderer CPU and round time) won't be caught night to night; the drift check is for those.
- **A missing or failed nightly build** gives nothing to compare. The job reports that and opens no `perf-regression` issue.
- **Time:** 8–10 runs may need about 2 hours of a runner. That is free on a public repository, but it is a long job, so the timeout is set from Phase 0's numbers.
- **Writing to `perf-data`** needs `contents: write` for this workflow. Only this job writes the branch, and only the one file.

## 7. Decisions for the owner

- **D1.** Report only, with the issue, until a few weeks of data have set the margins. *Recommended.*
- **D2.** The history on a `perf-data` branch in this repository, rather than an external store. *Recommended.*
- **D3.** No UI now; the widget pane is decided later. *Recommended.*
