# Where the merge queue's minutes and the release run's two hours go

**Date:** 2026-11-24. **Source:** recorded GitHub Actions runs only (job and step timings, job logs, run lists). No run was started, nothing was benchmarked. Every number is tagged **[M]** measured from a recorded run, **[I]** inferred from what the logs and workflow files show, or **[E]** estimated (needs one measurement before anyone relies on it).

**Decisions this informs:** (1) which cheap CI changes to make next, in order; (2) whether a paid larger runner is worth asking for.

## 1. Answers in brief

1. **The queue is compile-bound, not test-bound.** In queue run 45447896545 the Ubuntu nextest job took 14.4 min **[M]**. The tests took 6.9 s for 1,212 tests **[M]**. About 13.2 min was `cargo` compiling, in four builds one after another: 3m51s, 4m53s, 0m54s (a `dev` check) and 3m34s **[M]**. The same crate (`agentmux-srv-core`) is compiled three times in that one job, and `agentmux-common` three times **[M]**.
2. **The queue's Rust cache hits but does not help much.** The cache restored on an exact key (749 MB) and 64 third-party crates were still compiled in the same job **[M]**. The queue never saves (`save-if` is off for `merge_group`) **[M]**. The 64 compiles cost at most about 2 min by their timestamps **[I]**; the big cost is the workspace crates, not the dependencies.
3. **Release wall time (122 to 138 min) is one job.** The macOS app build's one `cargo build --release` took 78m 21s in the 0.60.39 run and 100.3 min for the whole build step in the 0.60.40 run **[M]**. The release profile is fat LTO, one codegen unit, `opt-level = "s"` **[M, from Cargo.toml]**. The server-package legs build the same programs with `fast-release` (thin LTO) in 10 to 31 min on the same platforms **[M]**.
4. **A fixed 15-minute wait sits at the end of every release.** `sign-release` waited 15.0, 15.1 and 15.1 min in the last three runs **[M]**. It is a deliberate environment wait (a window in which a run can be cancelled before signing) **[M, from the workflow comment]**. It starts only after every build is done.
5. **macOS runner availability is a second bottleneck.** The macOS check in PR runs waited a median 11.2 min (max 29.2) for a runner **[M, 8 runs]**; in the one push-to-main run that finished it waited 32.7 min **[M]**; the release's macOS sign and notarize job waited 12.4 min in the 0.60.40 run **[M]**.
6. **A change already merged covers frontend-only entries.** PR 8287 (a631a3b459) skips Rust for them. Since then the four frontend-only queue entries took 5.4 to 8.3 min (median 7.7) against a median of 16.6 min for entries with Rust **[M, n=4 and n=36]**. Vitest (two shards, 5.9 to 7.5 min median) is now the longest part of such an entry.

## 2. The merge queue

### 2.1 The population

The last 40 completed `merge_group` runs of `ci-pr.yml` **[M]**:

| | runs | total minutes: median / min / max |
|---|---|---|
| with Rust (the Ubuntu nextest job ran) | 36 | 16.6 / 12.4 / 21.2 |
| without Rust | 4 | 7.7 / 5.4 / 8.3 |

Job run times in a queue entry with Rust (median, max) **[M]**: `check --tests + test (ubuntu-latest)` 15.1 / 18.1; `vitest (2/2)` 7.5 / 9.2; `vitest (1/2)` 5.9 / 7.4; `doc status + grep gates` 2.1 / 2.6; `specs index` 0.6; `classify`, `CI required` 0.1. Wait for a runner in the queue: median 0.1 min, max 1.9 **[M]**. The entry's time is the Ubuntu job's time; nothing else is on its critical path.

### 2.2 The Ubuntu nextest step: compile against tests (run 45447896545, job 138636759675)

| Phase | Start | What |
|---|---|---|
| checkout, toolchain, linker setup | 0 s | 33 s |
| Rust cache restore (exact key hit, 749 MB) | 34 s | to 51 s |
| `scripts/ci-nextest-run.sh` starts | 55 s | |
| build 1: `nextest run --no-run <4 crates> --lib` | 56 s | `Finished` at 287 s (3m51s) |
| build 2: `--bin agentmux-srv` | 288 s | `Finished` at 581 s (4m53s) |
| `cargo check --tests` on the other crates (`dev` profile) | 596 s | `Finished` at 635 s (53.75 s) |
| build 3: `nextest run --profile queue ... --lib --bin ... --test ...` compiles again | 637 s | `Finished` at 850 s (3m34s) |
| **tests run** | 850 s | 1,212 tests, `Summary [6.911s]` |

**[M]** from the job log. The script's own header says the three builds exist to stop cargo from compiling the core and the server layer twice at once under a 16 GB memory limit, and that the queue's run was meant to cost "about 7" minutes. The log shows the core crate compiled at +71 s, +288 s and +655 s: three compiles. Whether the three differ in features (the likely cause, as the script's own comment explains for an earlier case) is **not visible in the log [I]**; `cargo build -v` or `--timings` on one run would show it.

The test run itself is 0.8% of the job. Which tests dominate the run: not recorded (nextest prints only the summary line for a passing queue run, and the queue profile filters out 17,087 of 18,299 tests **[M]**). A "slowest 20" list needs the full suite's JUnit or `--final-status-level` output, which is not stored.

### 2.3 What a pull request runs (30 recent `pull_request` runs, 13 with Rust) **[M]**

| Job | n | run median / max | wait median / max |
|---|---|---|---|
| Ubuntu `check --tests + test` | 9 | 23.6 / 37.2 | 1.9 / 3.1 |
| Windows `nextest build` | 9 | 18.1 / 19.8 | 0.2 / 0.8 |
| Windows `nextest` shards 1/3, 2/3, 3/3 | 8 | 8.0, 8.5, 8.0 / 8.4 to 9.2 | 0.1 |
| macOS `cargo check --tests` | 8 | 16.8 / 20.9 | **11.2 / 29.2** |
| vitest (1/2, 2/2) | 11, 12 | 7.0, 6.0 / 7.5, 7.9 | 1.1, 0.8 |
| total run, with Rust | 13 | **39.3 median**, 11.9 to 57.2 | |
| total run, without Rust | 17 | 8.1 median, 2.4 to 13.5 | |

A pull request runs the full suite on every leg: Ubuntu builds all eight packages with `--profile ci` and runs everything (a 35 min step on the push and release runs, see 4); Windows builds an archive (16 min) and runs it in three shards; macOS runs a compile check (19.4 min in the push run).

### 2.4 What runs more than once for the same commit content

| Stage | What it runs | Why it can or cannot be dropped |
|---|---|---|
| Pull request | all legs: Ubuntu full suite, Windows archive and shards, macOS check, vitest, docs gates | The only stage that tests the PR's own tree. Cannot be dropped. |
| Merge queue | the fast gates and the tests that read the tree, on the merge of the PR with current main (Ubuntu with the `queue` profile filter, vitest, docs gates; no Windows or macOS) | Tests the combination with main, which only this stage can do. The part that is repeated work is the Rust compile (2.2). |
| Push to main | the full PR set again: Ubuntu 36.7 min, Windows build 17.6 + shards 5.9 to 8.2, macOS 20.3 (after a 32.7 min wait); the one completed run took 68.7 min (45463039676) **[M]** | The tree is the merge commit the queue just tested, and the pull request already ran the same legs. The only new information is a break that appears from the combination with main on Windows or macOS, which the queue does not run. Of the last 60 push runs, 50 were cancelled by the next push and 10 completed **[M]**: the runs that survive are the long ones. |
| Release | `suite` job on the tagged commit: Ubuntu 35.5 to 37.1 min, Windows check 6 to 8 min **[M]** | The tagged commit is a version-bump commit on a tree the queue and the main run already tested. The suite is not on the critical path (the macOS build is), so it costs runner minutes, not time. |
| Nightly | full workspace including CEF | Owns the CEF-inclusive build; not a repeat. |

Dropping the Windows and macOS legs from the push-to-main run (keeping `main status` and the cheap gates) would free about 55 runner-minutes of Windows and macOS per completed run and take macOS demand off the runner pool that PR runs wait in **[E]**: what that does to the PR waits is not measurable from recorded runs.

## 3. The release run

Last three completed runs (`release.yml`; job minutes are the run time, not the wait) **[M]**:

| | 0.60.39 (45070650307) | 0.60.38 (44955408487) | 0.60.37 (44838583303) |
|---|---|---|---|
| wall time | 126.7 min | 121.6 | 138.0 |
| sum of all job minutes | 397 | 378 | 402 |
| macOS app: unsigned build step | **93.2** | 87.3 | **104.5** |
| Windows app: build step | 45.1 | 58.5 | 45.5 |
| Linux AppImage: build step | 50.8 | 50.4 | 51.1 |
| macOS sign, notarize | 5.2 | | |
| `suite` Ubuntu / Windows | 37.1 / 6.2 | 35.9 / 7.9 | 35.8 / 5.5 |
| server package: macOS / Windows / Linux x64 / Linux arm64 | 35.1 / 31.6 / 12.6 / 14.8 | 25.0 / 24.9 / 12.7 / 14.5 | 31.4 / 24.6 / 18.7 / 14.1 |
| container image: build and push | 33.3 | 21.8 | 32.7 |
| `sign-release` wait (environment) | **15.0** | 15.1 | 15.1 |

The in-progress 0.60.40 run (45456723153) at the time of writing: macOS unsigned build step 100.3 min; sign and notarize waited **12.4 min** for a macOS runner, then ran 6.0 **[M]**.

**Where the two hours go [M, I].** The critical path is the macOS unsigned build (87 to 104 min), then macOS sign and notarize (5 to 6 min, plus up to 12.4 min waiting for a runner), then the 15 min environment wait, then publish (about 1.4 min). The Windows (45 to 58 min) and Linux (50 min) builds, the suite (36 min) and the container image (22 to 33 min) all run in parallel with it and finish first, so shortening any of them does not shorten the release.

**Inside the macOS build step (0.60.39, job 137356798857) [M].** Setup to 378 s (checkout, `npm install` about 60 s, CEF runtime download, a toolchain for the remote helpers). Then one `cargo build --release`: `Finished` at 4,941 s, **78m 21s**. Then a second `cargo build --release` for the CEF host, mcp, bashwrap and launcher: 9m 03s. Then packaging, signing preparation and upload to 5,756 s.

**What is serial that could be parallel.** The 15-min wait could start with the run instead of after the builds: the environment wait is meant as a window in which the run can be cancelled before signing, and the run is cancellable for the two hours before it anyway **[I]**. Whether that satisfies the control is a security decision, not a CI one. Nothing else on the critical path is serial by dependency: the macOS build starts about 1 min into the run.

**What a re-run costs when a job fails late [M].** Since 11-14, 4 of about 13 releases needed a re-run attempt (0.60.28 attempt 2, 0.60.30 attempt 3, 0.60.34 attempt 2, 0.60.36 attempt 2). Run 44150498170 (0.60.33) failed after 124 min in the Linux x64 server package, at the step that runs the package in an image with no certificates, so the whole run's 124 min were lost for a step that is 12 to 18 min of work. The tests that gate a release (the `suite`) run in parallel with the builds, so a test failure is found at minute 36, but the run keeps building until it ends **[I]**.

## 4. Transient and repeated failures, 2026-11-10 to 2026-11-24

**Merge queue [M]:** 1,868 `merge_group` runs of `ci-pr.yml`: 1,606 success, 164 failure (8.8%), 82 cancelled. The 164 failed runs burned 2,057 run-minutes. Failing jobs: `vitest` 97 (shard 1: 77, shard 2: 20), the size and function ratchets in `doc status + grep gates` 68 (function size 40, UI primitives 23, file size 5), the `vite-port` self-test 7, Linux build-dependency install (apt) 7 (57 min; 2 of the 7 were on 11-11, before the retry wrapper `scripts/ci-apt.sh` landed, and the other 5 ran with it, see below). By day: 11-13: 54, 11-15: 39, 11-14: 19, 11-16: 15, 11-18: 10, 11-20: 10, 11-21: 10, 11-19: 3, 11-11: 2, 11-22: 1, 11-24: 1. **Most failures come in bursts on four days; the last three days have two.** A burst of repeated `vitest` shard-1 failures on 11-13 (25 in an hour) is one broken state on main hitting every entry behind it, not 25 independent faults **[I]**.

**Release and container image [M]:**
- 0.60.40: the container image's `build-and-push` failed in 0.5 min: `toomanyrequests: Rate exceeded` on the anonymous pull of the pinned `rust:1-bookworm` base from the public ECR gallery (the image is referenced by digest). A failed image job does not stop the release (the manifest then names the versioned tag), but it costs a re-run attempt or a release without the image digest.
- Runner loss, apt and registry failures in the 11-10 to 11-24 release and server-package runs: besides the above, 1 server-package failure at the no-certificates image step (0.60.33), and 7 apt failures in the queue (above). `build-server-package.yml` had 40 runs in the window with 5 failures and 4 cancellations, all on branches of the server-package work **[M]**.

**The apt failures, read from their logs [M].** `scripts/ci-apt.sh` (a retry wrapper with a time limit and a budget per call) has been in the queue's install step since 11-11, so item 9 below is not a missing wrapper. The five queue failures after it (runs 44370568235, 44472038063, 44482238409, 44522702847, 44664254183) stopped after two attempts, for two reasons: in two, the first attempt was killed by its 90 s limit part way through unpacking and the second then stopped at "dpkg was interrupted, you must manually run 'sudo dpkg --configure -a'"; in three, a slow mirror (30 s per package before apt gave up on it) used up the queue's 150 s budget after two attempts, with the third never tried.

**Cheap guards.** The 429: a retry with a pause on the image build when it fails before any compile could have run, keeping the base pinned by digest; a signed-in pull or a copy of the base in the project's own registry would also work but each adds a credential or a registry to maintain **[I]**. Apt: run `dpkg --configure -a` before an install retry, and give the queue a budget that leaves room for a third attempt **[I]**. Neither needs a larger runner.

## 5. Recommendations, ranked

Minutes are per occurrence, with how often it occurs and how sure the number is. Each line can be filed as a task.

| # | Task (one line) | Saves | Effort / risk |
|---|---|---|---|
| 1 | *Done:* frontend-only queue entries skip Rust (PR 8287). | 16.6 to 7.7 min on those entries **[M]** | done |
| 2 | In `ci-nextest-run.sh`, find why `agentmux-srv-core` compiles three times in the queue job (one diagnostic run with `cargo build -v` or `--timings`) and fold the builds into one that cargo can reuse. | up to 8 to 9 of the 14.4 min, on every Rust queue entry **[E]** | M. Risk: the staged builds exist to stop memory stalls on a 16 GB runner; the change must keep the memory trace and the retry. |
| 3 | Start the release's 15-minute signing wait at the start of the run instead of after the builds. | 15 min per release, about 1.4 releases a day **[M, for the 15 min]** | Declined by the security lead: the wait stays on the signing job. |
| 4 | Release profile: try thin LTO or four codegen units for the app builds; compare the binary sizes. | up to about 45 min on the macOS critical path (the server package's thin-LTO build of the same programs took 31 min against 78 for the app's) **[E]**; the release would end near the Windows and Linux builds (45 to 58 min) | M. Risk: binary size and speed change; this is a product decision with a size measurement first. |
| 5 | Push-to-main: run only the Linux leg and `main status`; leave Windows and macOS to the pull request and the nightly. | about 55 Windows and macOS runner-minutes per completed push run; takes load off the macOS pool where PR runs wait a median 11 min **[E]** | S. Risk: a platform-specific break reaches main until the nightly. |
| 6 | Vitest: four shards instead of two. | 6 to 7.5 min down to about 4 on every entry, and on frontend-only entries 7.7 down to about 4.5 **[E]** | S. More runner minutes, free on the public repository. |
| 7 | Container image: authenticated pull or a retry with backoff for the base image; keep the digest. | removes a release re-run that cost a failed image on 0.60.40; saves nothing when it works **[M, 1 in the last 4 releases]** | S |
| 8 | Let the push-to-main run save the queue's Rust cache (under the queue's own key) so the queue restores what it builds. | up to about 2 min **[I]** | S |
| 9 | Linux build-dependency install in the queue: finish an interrupted dpkg before a retry, and a budget that allows a third attempt. | 5 queue failures in the 7 days after the wrapper landed, 2 of them from the interrupted dpkg and 3 from the 150 s budget, each ejecting an entry **[M]** | S |
| 10 | Server-package legs and the app builds compile the same programs twice with two profiles; build once. | about 30 min of Windows and macOS runner time per release, nothing on the critical path **[I]** | L. Only after 4 decides the profile. |

## 6. The runner question

**Prices, from GitHub's public price list (fetched 2026-11-24) [M]:** standard Linux 2-core $0.006 per minute, Windows 2-core $0.010, macOS 3 or 4-core $0.062. Larger runners: Linux 4-core $0.012, 8-core $0.022, 16-core $0.042, 32-core $0.082; Windows 4-core $0.022, 8-core $0.042, 16-core $0.082, 32-core $0.162; macOS 12-core (x64) $0.077, macOS 5-core M2 Pro $0.102. The page says larger runners are **not free for public repositories**. The workflow's own header says it uses standard runners only because they are free on public repositories.

**What the recorded runs say a larger runner would and would not do:**

- **macOS release build.** The 78-minute step is one `cargo build --release` with fat LTO and one codegen unit, so most of it is single-threaded; more cores would not cut it much **[I]**. At $0.102 a minute a 100-minute build would cost about $10 per release for an unknown gain. The lever is item 4, which costs nothing.
- **macOS runner waits.** The waits (median 11.2 min on pull requests, 32.7 min on the push run, 12.4 min in the release) are in the shared free pool. A larger runner is a separate pool, so it would probably start at once **[I, not measured]**. For the release's sign, notarize and check jobs (about 7 min) that is about $0.71 per release at the M2 Pro price, for up to 12 min saved. For the pull-request macOS check (16.8 min median) it would be about $1.71 per run.
- **Ubuntu queue job.** The job runs with two build jobs because 16 GB is tight (the script's header measures 14.3 to 14.7 GB demanded at four). An 8-core larger runner at $0.022 a minute would allow more jobs, and a worst case of no time saved costs 15.1 min × $0.022 = **$0.33 per entry**; with 36 of 40 entries running Rust and about 115 successful entries a day **[M]** that is about $38 a day if it saved nothing. A saving of the compile time of 25 to 40% **[E, no recorded run]** would bring a Rust entry from 16.6 to about 12 min. This is the only larger-runner case whose gain is not first available for free, and it is worth measuring only after item 2, because item 2 removes the duplicate compile it would otherwise pay for.

**Recommendation.** Do not ask for a paid runner yet. Items 1 to 3 and 5 to 9 are free and cover most of the measured time. If the operator wants one paid thing, the macOS sign, notarize and check jobs on a larger macOS runner are the cheapest to try (under $1 per release for up to 12 minutes), and one measurement would show whether larger runners queue less.

## 7. What this report did not measure

- Which tests dominate a full-suite run (no per-test timings are stored).
- Whether the three `agentmux-srv-core` compiles in the queue job differ in features.
- The binary-size and speed cost of thin LTO, and the build time of the release profile at four codegen units.
- Whether a larger runner starts faster or builds faster; all such figures here are estimates.
- Any run of the fixes above; everything is read from recorded runs.

Agent: Agent2@narko
