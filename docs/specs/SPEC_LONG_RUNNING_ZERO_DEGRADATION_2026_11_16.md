# Spec: hour 2,000 is as fast as hour 1: no degradation over a session's life, with budgets and local telemetry that find a slow decline

Task #47819 (build: #47820). Loap-3@narko, 2026-11-16. Status: draft for the operator's "looks good"; nothing is built before it.

The operator's words (2026-11-16): "part of agentmux's main value prop is it can stay open for a year without leaking memory. in that case, it needs extremely strict memory boundaries and managemenet ... we also want to add adaquant memory telemetry since these issue can only be reproduced and uncovered after running for a long time, perahps weeks", and, widening it: "lets then generalize this to performance degradation. ideally we want zero performance degrdation, it may not necessarily be memory only".

## 1. Decision

AgentMux makes three commitments about a session of any age, and records enough on the machine to see a breach from the record, without a repro:

1. **Bounded size.** Every process, pane kind and data store has a budget in bytes (and in counts: handles, sockets, timers, children). Reaching it sheds the least valuable thing first and says so; it never kills silently and never loses work.
2. **Bounded time.** Every thing a person feels has a latency budget that does not depend on session age, and nothing slow runs on a main thread (the host's UI thread, a page's main thread).
3. **Always a way back.** A window, a pane or a process that has grown past its budget can be reset safely with its state kept, and nothing idle (an open search bar, a forgotten dialog, a flag that never cleared) can stop that reset.

The record: a compact local time series kept for weeks across restarts and upgrades, with the session's age on every sample, that nothing ever sends anywhere.

The smallest first phase is the record plus the log fields that make today's declines readable (phase 0 and 1, §9). The rest is staged so each phase is useful alone.

## 2. Evidence: what a long session did, from the logs of three machines

Three recent investigations, each from logs only (no run). Here are the ratios and the lessons.

- **Renderer growth over a day (#47768).** One window's renderer grew about six times in 22 hours (0.8 to 4.9 GB private). About 55% of the final size was memory the page still held after a forced garbage collection (grew about 4x: roughly 1.3 GB in steps when the person opened content, a creep of 14 MB/h with nobody there and of about 45 MB/h at night with agents streaming, while the atom cache stayed flat); about 45% was garbage the page had made since the last collection (0.1 to 0.2 GB by day, about 2 GB at night, about 5 GB in a 20-minute jam). The hourly figure the operator watched was a sawtooth sample: the host forces a collection every 10 minutes and the sample landed at a different point of that cycle each hour, so two of the three "drops" were not drops. **Lessons:** record memory after a collection as well as before; never read a fixed-minute sample as a trend; the retained part and the garbage part have different causes and different fixes.
- **The reset that never ran (#47824).** The host asked the window to recycle (reload itself back to its starting size, state kept) 79 times in the day; every ask was declined, 77 of them because a find bar was open somewhere in the window. So the one mechanism built for this growth was disabled by idle UI state. **Lesson:** a safety net must not depend on the person's idle state; each reason that can decline it needs a way through (carry the state across, or a time limit). PR #7556 does this for the search bar and the other reasons.
- **An open search made garbage on every change (#47824).** With the bar open and a query typed, every transcript change re-ran the search and allocated a set of every row id; measured, one pass over 10,000 rows grew the heap about 0.7 MB. Fixed in the same PR (about 0.5 MB over 50 passes now). **Lesson:** work that scales with transcript length belongs in the catalogue (§4) and in CI (§8).
- **Freezes on a Linux machine, probably not AgentMux (#47818).** Fifteen stalls of 6 to 55 s in 8 hours, 9.5 to 17 hours into a session: the page's main thread and the host's UI thread stopped together (the page's 5 s RPC timeouts all fired in the same millisecond at the end), on a virtual machine whose swap file was 93% full on a 96%-full disk, with the kernel logging 26 s all-CPU lockups. It is a degradation that is not memory and not in the page; the log could not say what the stopped threads waited on. **Lessons:** a stall needs its cause recorded when it happens (the thread's state and wait channel, free memory, free disk); the product should say "this machine is low on disk and swap" before the desktop says "not responding".
- **Same builds, other machines.** A Mac ran 41 hours with no stall of 2 s or more and its main renderer about doubled (10 MB/h). A Windows machine had 5 stalls in 25 hours, all but two being real layout work inside one handler during the jam. So a decline has at least three families: **grows with age** (memory), **does more work with age** (layout, searches, rebuilds), and **waits on the machine** (swap, disk, keyring).

## 3. What "does not degrade" means

For each of these, the guarantee is: measured at the same load, the value at session age A is within a stated margin of the value in the first hours of the same instance, for any A up to a year. Margins start as the numbers below and are set from the first weeks of data (§9, phase 2); a budget no one has measured is a guess, so the first release of the record states them as proposed.

**What a person feels** (each measured in the page, in the field, as a percentile per hour):

| Measure | Where | Proposed margin |
|---|---|---|
| Key to character on screen (typing) | slow-interaction / event timing in the page | p95 within 20% of the first 6 h; never over 100 ms at p95 |
| Click to response | same | same |
| Scroll smoothness: frames over 50 ms per minute while scrolling | the always-on frame-time sample (#7458) | within 20% |
| Tab switch and window switch | the `tab-switch` line | p50 within 20%; never over 400 ms at p50 |
| Open a pane | pane mount time | within 20% |
| An agent's output appears (token received to painted) | the flush budget (#7468) | p95 within 20% |
| Resume of a window or app (reload, upgrade) | boot marks | within 20% |
| Main-thread stalls | long-task and long-frame lines | count of 1 s+ stalls per hour does not rise; none over 5 s from AgentMux's own work |

**Resources** (each sampled per process, with the session age):

| Resource | What "does not degrade" means |
|---|---|
| Memory per process and per window | after a collection, the retained size per mounted pane and per transcript row is flat; the total stays under the process budget (§5); a window resets (recycle) before it is over |
| CPU | idle CPU and CPU per streamed token at the same load are flat |
| Disk | AgentMux's own data (history, logs, indexes, caches, telemetry) stays under its budget and grows only with the person's content, never with time alone |
| Handles, sockets, threads, timers, child processes | counts return to their baseline when the work that opened them ends; no count only goes up |
| Store and index sizes and their query times | query time depends on the query, not on the store's age (compaction, rotation) |
| Update, start and resume time | flat |

Load is the confounder: a window with ten more panes uses more. So every size and time series is read against content counters (mounted panes, transcript rows, tabs, windows, agents streaming) and the flag fires on per-pane and per-row figures, not on totals.

## 4. How it degrades: the catalogue, and what exists today

| Class | Examples | Known today |
|---|---|---|
| Growth without bound | page heap, DOM nodes, stores and caches, the folded-text cache of an open search, logs, the history store, scrollback, per-pane state | renderer retained memory grows with age and with open content (#47768); the folded search cache holds a lower-case copy of every row while a bar is open; log and history growth are not measured |
| Work that scales with age or transcript length | flush cost, layout (the 28 s forced layout in one handler), search refresh, list rebuilds, polls that read whole lists | flush has a budget (#7468); the search pass was O(rows) with allocation per row (fixed #7556); pollers (swarm, kanban, line-counts) run on timers on every machine |
| Accumulation | listeners, timers, sockets, zombie children, bindings abandoned after a throw (a stale `<Show>` read was logged nine times in one night) | not counted anywhere |
| Fragmentation and file growth | sqlite and index files, the build cache, logs, transcripts | a "low disk" warning exists; no growth series |
| Stalls on things outside the page | the host's UI thread, disk, the keyring, the network, an update download | logged as a long frame only; the cause is not recorded (#47818) |
| State only a restart clears | a window over budget that cannot recycle; an upgrade that leaves something behind | the recycle exists and was disabled by idle state (#47824) |

## 5. Boundaries

A budget is a number and a rule for what happens at it. Starting numbers are proposals until phase 2 data exists.

- **Per process:** host, srv, each renderer by role (app window, pool window, browser pane, browser frame). A renderer's budget is a multiple of its own baseline (today: at least 2x and 600 MB) and an absolute ceiling; the host and srv have absolute ceilings.
- **Per pane kind and per store:** an agent pane holds a bounded number of rows in the page (a window over the transcript, older rows on disk); a terminal pane's scrollback is capped; a browser pane hibernates when unused; each cache has a maximum and an eviction rule (today's open search folds every row and keeps it for as long as the bar is open: it needs a cap or a re-fold on demand).
- **What holds a thing to its budget**, in order: eviction and windowing (cheap, invisible); hibernation (a pane sleeps, its state kept); compaction and rotation (stores, logs); **a recycle of the renderer or pane with its state carried across** (the existing mechanism, hardened as in §2); last, a clear note to the person.
- **At the limit:** shed the least valuable first (an idle pane's folded text, an old pane's rows, a caches' cold entries), tell the person once in the status bar ("Reset a window to free memory"), and never kill without carrying state and never lose what the person wrote. Anything that cannot be carried (unsaved text, a held message) is shown to the person as the reason, not waited on silently.
- **Time boundaries:** no call on the host's UI thread may block on disk, the keyring, the network, or a child; every such call moves off the thread or has a deadline and a fallback. A page's main thread never runs more than a slice of work per frame (searches, rebuilds and polls are sliced or incremental). The check is a CI rule where a rule can be written (§8) and a recorded stall otherwise.
- **The safety net's own rule:** nothing idle may block a recycle. Every reason that can decline one is carried across or has a limit (task #47824, the table in its PR). A new blocker needs to say which.

## 6. Telemetry: kept on the machine, for weeks

**Requirement: nothing leaves the computer.** No upload, no anonymous counters. The record is read by the person, by agents on the machine through the app API, and by export on request.

**What is recorded** (a sample every 5 minutes, on the host, from what each process already reports):
- per process: role, pid, RSS, private bytes, CPU time, handles (Windows) or fds (Unix), threads, and for renderers the JS heap used/total, heap after the host's last forced collection, DOM node count, mounted panes, and transcript rows held;
- per window: the page's frame-time percentiles, interaction-latency percentiles (the table in §3), long-task and long-frame counts by size bucket, tab-switch percentile;
- stalls: each of 2 s or more as an event, with the thread's state and wait channel, the process's size and swap, free memory and free disk (the probe of #47826), and what ran in it (the script attribution the page already logs);
- stores and files: size and growth of the history store, indexes, logs, transcripts, the build cache and the record itself; query times of the busiest queries;
- counters of what can accumulate: listeners, timers, sockets, children, caches' entries and bytes;
- machine context: free RAM, swap used, free disk, pressure level, so a decline caused by the machine is not blamed on the app.
Every sample carries the session's age, the instance, the version and the time of the last upgrade.

**Retention and size.** Raw samples for 14 days, rolled up hourly (min, median, p95, max) for a year, so a 10-process, 20-window instance is about 3 KB a sample: about 0.9 MB a day raw (under 15 MB for the 14 days) and about 0.1 MB a day rolled up (under 40 MB a year). The record has its own budget (§3, disk) and rotates oldest-first.

**Cost, to be proven small before it ships:** one sample is a few reads of counters the page and host already hold; the page reports its figures on the existing 5-minute path (no new timers); the host writes one append per sample off the UI thread. The build shows the cost with the same method used for the frame sample: CPU and allocation per sample, and the record's overhead as a share of idle CPU under 0.1%.

**Storage.** Owned by srv (it survives a host restart and an upgrade): an append-only segmented file per day with the rollups in a small table, or a table in an existing store; the choice is made in the build (§10). Written in a form an agent can attach to a bug (a JSON export per day).

**How it is read:** (1) a panel in the existing health or memory view: a chart per series against the session's age, with content counters beside it; (2) an `App` action returning a series or a window of samples, for agents; (3) `export` for a day or a range, with no text, titles or paths; (4) the log lines that exist today stay as they are.

## 7. Detection

The record is useless if someone has to look. A small set of rules runs in srv on the rollups:

- **Slope against age.** For each size or time series, per process and per pane kind, normalised by content (per mounted pane, per row), the slope over the last 24 h (Theil-Sen, robust to the sawtooth) is compared with the same instance's first 6 hours and with the previous version at the same age. A rise over the margin for two consecutive days, or an absolute breach, is a flag.
- **Retained vs garbage.** A rise in retained-after-collection is a leak candidate; a rise in the gap between before and after is a churn candidate. They are flagged separately because they have different owners.
- **Stall events.** More than three stalls of 5 s or more in an hour, or any over 20 s, is a flag, with the recorded thread state attached, and the machine's free memory and disk beside it (so it can say "this machine").
- **Recycles that never run.** More than five declines in a row for one window, or none in the last 24 h for a window over budget, is a flag with the reason and the holding pane (the count of #7556).
- **What a flag does.** A quiet note in the status bar ("Memory in one window is growing faster than its content: details"), a snapshot of what grew or slowed (the top series and the last stalls) stored with the record, and a message to the machine's steward agent. It does not interrupt the person and does not kill anything.
- **False positives:** each rule has a floor (a change smaller than the sampling noise never flags) and a content check (more mounted panes explains more memory).

## 8. How it is proven

**CI, in minutes** (no lab, nothing driven):
- a bounded-structure check: a unit test per store and cache that adds N times its limit and asserts the size stops at the limit (extends the existing store and cache tests);
- no unbounded growth in a loop: repeat the open-and-close of a pane, a search and a tab switch many times in a test page and assert the counters (listeners, timers, nodes) return to baseline;
- no slow call on a main thread: a lint on synchronous disk, keyring, network and process calls in the host's UI-thread code, and a test that the page's pollers are sliced;
- a pass-cost test for every O(rows) pass: time and allocation per row stay under a budget at 10,000 and 30,000 rows (the measurement behind #7556, kept as a test);
- the recycle's blockers: a test that each registered blocker has a limit or a carrier (the table in #7556 as a check).

**What only a soak shows**, and needs the operator's yes through AgentY before it runs (a lab): a long run under the real mix. What I would ask for: one run of 48 hours on a CI runner (never the operator's machine, never the real mouse), scripted load of N agents streaming, M tabs, one long transcript, a search open and closed, a recycle every 8 hours; sampling with the new record; success is every series in §3 within its margin and flat after content is normalised. It answers whether the budgets hold; the decision it informs is whether the margins are right and whether any leak remains after #7556. Not requested now.

## 9. Phases

0. **Make today's declines readable (in flight).** #47823 (five-minute renderer line: JS heap, collection gap, DOM nodes, mounted panes, rows, open search, window label on the cache line, blocker holder in a decline); #47826 (stall probe: thread state, wait channel, memory); #47824 / PR #7556 (the recycle is not blockable by idle state; declines are counted; the open search's pass allocates nothing per row).
1. **The record.** The sample on the host and srv, the daily file with hourly rollups, the budget for the record, the cost proof, the export and the `App` action. First useful: any later decline is findable. Smallest: per-process size and CPU, per-window stalls and percentiles, store sizes, machine context.
2. **See it.** The panel; the first weeks of data fix the margins; the proposed numbers in §3 and §5 are replaced by measured ones in a short follow-up edit of this spec.
3. **Flags.** The rules in §7 and the status bar note; the steward message.
4. **Hold the budgets.** One item per class of §4, taken in the order the record shows: the open search's folded cache cap, the rows an agent pane holds, scrollback caps, log and history rotation, accumulation counters with a baseline check, the host's UI-thread rule. Each is its own small PR with its CI check from §8.
5. **Prove it.** The soak in §8, with the operator's yes.

Each phase is one or a few PRs, merged on muxreview approval; none depends on the later ones.

## 10. Decisions

Approved by the operator on 2026-11-16. Their words, relayed by AgentY: "yes, zero degradation fix is good". The answers below are AgentY's reading of that approval against the recommended answer to each question.

1. **Margins.** The proposed starting margins in §3 stand (20% at p95, hard caps for typing and tab switch) until measured numbers replace them after phase 2.
2. **Retention.** 14 days raw and hourly rollups for a year, under 40 MB.
3. **Flags.** A flag may put a quiet note in the status bar.
4. **Restarts.** AgentMux restarts no process other than a renderer to hold a budget: a flag and a note only, until the record shows more is needed.
5. **Soak.** The soak (§8) is not approved now. Ask when phase 3 is merged; it stays a lab that needs the operator's yes then.

The record stays on the machine: no upload, ever, without a new decision.

For the build (decided in #47820, not now): the record's storage form (a segmented file or a table in srv's store), whether the hourly rollup is computed on write or by a job, and the exact content counters per pane kind.

## 11. What this spec does not do

It does not change a budget before there is data for it, does not add any upload, and does not run a lab. It does not settle the Linux freezes of section 2: those are a machine problem with an AgentMux part (the stall probe and the UI-thread rule).
