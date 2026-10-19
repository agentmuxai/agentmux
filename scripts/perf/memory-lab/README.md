# Memory lab

Measures a production build's memory under a repeatable agent load, so a change
can show what it saved. The method is §2 of
[`REPORT_RENDERER_MEMORY_ROOT_CAUSES_2026_10_12.md`](../../../docs/reports/REPORT_RENDERER_MEMORY_ROOT_CAUSES_2026_10_12.md),
made into scripts. The budget it serves is 500 MB for the app's own processes.

## Measure a build

Run the lab from a checkout of the repo, after `npm ci` (the CDP client needs the
`ws` package; a copied-out `scripts/perf` folder has no `node_modules` and the
lab says so and stops) and with a Rust toolchain (memtree is built on every run).


1. Build a portable of the branch: `task package` (a **production** build; a
   `task dev` build runs Vite and SolidJS in dev mode and inflates every number).
2. Run the lab on it:

   ```bash
   node scripts/perf/memory-lab/run.mjs --portable <the portable folder> --label my-branch
   ```

   On macOS `--portable` takes the `AgentMux.app` (or the folder holding it) and
   runs its `Contents/MacOS/agentmux-launcher`; on Linux the extracted tarball's
   folder (its `agentmux.sh`). For an AppImage or any other executable, pass it
   with `--exe <path>` instead of `--portable`.

   Options: `--panes 4|12` (4 agents in one tab, or 12 over 4 tabs as 4/2/4/2;
   default 4), `--rounds 10`, `--gap-s 15` (seconds between rounds), `--every 5`
   (a curve point every N rounds), `--settle-s 20`, `--samples 3`, `--seed 1`,
   `--port 9333` (the instance's CDP port; 9222 is refused, and so is a port
   something already answers on: pick another when two labs run at once),
   `--attempts 2` (see "A run that isn't the load"), `--out <dir>`
   (default `memory-lab-out/`), `--keep` (leave the instance running),
   `--memtree <path>` (default: built from `crates/procstats` every run; cargo makes it a no-op when current).

   It takes about 3 minutes for 4 agents and 10 rounds, and needs nothing else
   running on the CDP port.

3. To see what a change did, run the same options on `main`'s portable and the
   branch's, then:

   ```bash
   node scripts/perf/memory-lab/compare.mjs memory-lab-out/main/result.json memory-lab-out/my-branch/result.json
   ```

   It refuses two runs with a different layout, rounds, round spacing, seed
   or operating system: the same conversation on the same OS is what makes the
   numbers comparable.

## Before and after a change: memory, CPU and time

A change that saves memory must not cost speed, so every run reports three things
and `compare.mjs` puts them in one table:

- **Memory:** private working set per process group, forced-GC median (as above).
- **CPU seconds** per group (renderer, gpu, srv, host, utility) spent over the
  workload: the highest CPU time seen for every process, summed, so a recycled
  renderer keeps what it spent before it was replaced. Counted from the
  round-0 sample to the end, so it includes the settle and the forced-GC final
  (the same for both sides). A process that starts and exits between two
  samples is missed.
- **Round time:** milliseconds from sending a round until every agent is idle
  again, the median over the rounds (`result.wall`). It is a coarse signal of
  how long the build takes to play the same turns, not user-perceived latency.
  A round starts when the previous one has finished (and not before the gap), so
  a slow build is no longer overlapped with the next round.

How to compare two builds (or the same build with and without a change):

1. **One commit for both sides.** Build "before" and "after" from the same
   commit, differing only in the change under test. Two builds from different
   commits differ in everything that landed between them.
2. **At least 4 runs of each side, interleaved** (A B A B A B A B), same options,
   so drift in the machine's state hits both sides alike. One run of each
   cannot tell a change from noise: CPU time and round time move with the
   machine's load.
3. `node scripts/perf/memory-lab/compare.mjs --base a1,a2,a3,a4 --head b1,b2,b3,b4`
   (paths to the `result.json` files; two positional files still work). It
   prints each side's median with its lowest-highest, the change in percent,
   and flags a measure past its margin. The default margins are per metric,
   from the measured noise of identical runs on a runner: srv bytes written
   +3% (it varies 0.2-0.4% between runs); app, srv and renderer CPU +10%; GPU
   CPU and the other groups +20%; round time +20%; memory report-only unless a
   `memoryPct` is given (it varies 13-30% on a runner). `--margins <file>` reads
   them from a JSON file (`budget.json`'s `margins`, or the margins themselves;
   a CPU object without a `default` is filled from the defaults); `--cpu-margin`
   / `--wall-margin` set one number for all CPU or for the round time. A flag
   also needs the two sides' lowest-highest ranges not to overlap. `--json
   <file>` writes the comparison as the nightly history line's `measures`
   (`app_mb`, `srv_written_mb`, ..., each `{a, b, change_pct, flag}`, `a` being
   the base) and the names that flagged. The flags are report-only and the
   margins are a starting point: the operator sets the real ones.
   A run whose slowest page command (`result.slowestEvalMs`) took over 10 s is
   named in the output too: the window was unresponsive for that long during a
   round.

Results made with a different workload version (`result.workloadVersion`; the
pacing of the rounds) are refused: the two are different loads.

### Measuring an srv change without building the app

**Build the replacement srv from the same commit as the portable, with only the
change under test differing.** The srv and the frontend share a protocol, but an
srv from a different commit can still launch and then leave the frontend unable
to create agents or play turns (seen on Linux with an srv built from `main`
swapped into a portable built 12 commits earlier). Pass `--srv-commit <sha>` and
the lab compares it with the commit in the portable's folder name
(`agentmux-0.59.32+g<sha>...`) and warns when they differ; a run that then fails
to create its agents or play its turns says the srv was replaced. The usual
recipe: build the portable from a pinned commit C (its own srv is the "before"),
build srv from C plus the change (the "after"), and swap only the srv.

`--srv <path to an agentmux-srv executable>` (with `--portable`) runs a copy of
the portable whose `agentmux-srv` is replaced by yours; the host, the
renderer and the packaging stay as they were, so only srv differs. The
portable itself is untouched; the copy lives in the label's output folder
(about 500 MB on Windows). `result.json` records what was put in
(`srvReplaced`: file, source path, size, a hash). A portable has exactly one
`agentmux-srv*` file or the lab refuses. Build srv for the portable's
platform, and from a version the portable's host can talk to (the same
protocol); a mismatched one fails to start, which the lab reports as a failed
launch.

## What it does

- Starts the portable **isolated**: its own data folder
  (`AGENTMUX_HOME_OVERRIDE`, under the output folder, emptied at the start of
  every run) and its own CDP port. Any `AGENTMUX_*` variable of your shell is
  dropped. It never touches your own `~/.agentmux`. It refuses a CDP port that
  already answers, and checks that the instance it drives is the one it
  started (the app only prefers its port, and takes another when it is busy).
- The agents are the scripted Claude stand-in
  (`scripts/ui-screenshots/fake-claude`), so the panes run the real pipeline
  (srv, the translator, the stores, the renderer) with no model, no account and
  no tokens. The conversation is generated (`lib/conversation.mjs`): tool-heavy
  turns of about 140 KB (Bash with 40-300 lines of ANSI output, Reads of 80-400
  lines, Greps, Edits, TodoWrites, a long markdown reply). The seed fixes it.
- Each round, every agent plays one turn. The OS numbers are sampled as it goes
  (the curve).
- After the last round the agents settle, then it takes **the final**: a
  Chromium memory-infra dump over CDP with a forced garbage collection (each
  process's allocator roots: v8, blink_gc, partition_alloc, malloc, cc, gpu,
  ...), per-page JS heap and DOM counters (read after the GC), and OS samples. The median of the
  final's OS samples is the steady value reported.

## Measuring a window recycle

`--recycle-after <round>` runs the window recycle command (the palette's
"Reload Window (Free Memory)", `--recycle-command` for another) in the main
window after that round, from the palette as a user would. The build must have
the command (#5086): on one without it the palette finds nothing, and the lab
reports `recycled: false`. Two results compare only with the same recycle
(round and command), or none in both: a final that follows a recycle is a
different measurement. Before it, the lab
puts a draft in one agent's composer and scrolls another agent's transcript up a
little. It then samples (with a forced GC) just before, and at 5, 30 and 60
seconds after: the app and renderer totals, the number of renderer processes, and
the main window's document count (it counts every document in the renderer, so
it reads 4 on a normal window: what matters is that it doesn't grow, which it
would if the old page were held in the back/forward cache). The scroll is armed
with real mouse-wheel input, as a user scrolls: a scripted `scrollTop` leaves the
pane following the bottom, which is not what a user's scroll leaves. Last, it
reads the draft and the scroll position back. `result.json` has it all under `recycle`, with the saving
(before against the 60 s mark) and `state.pass`, which is false when a draft or a
scroll position was lost: a recycle that frees memory but loses state hasn't
helped anyone. If the command refuses (something a reload would lose), `recycle`
says `recycled: false` and the notice's words.

### Is the saving still there later?

A recycle frees memory at once; what matters is where the window is some minutes
later. Two ways to find out, each against a control that does everything but
press the command (`--recycle-command none`: the same arming, the same samples):

- **Keep working:** run, say, `--rounds 20 --recycle-after 5` and the same with
  `--recycle-command none`; the curve (`--every 3`) shows whether the recycled
  window climbs back to the other's level.
- **Go idle** (what an idle-window governor does): `--recycle-after 5
  --hold-minutes 10` plays 5 rounds, recycles, stops the agents and samples the
  window once a minute for 10 minutes, without forcing a GC. `result.hold` has
  the samples.

Each mark after the recycle also carries what the page holds (DOM nodes, JS heap,
layout objects, listeners, tool blocks and the text inside them), so a restored
page can be set against a live one at the same point: a restore that mounts tool
bodies a live page had unloaded shows in `toolTextKB`.

## Passing an app setting to the instance

`--setting NAME=value` (repeat it for several) puts an app setting in the instance's `settings.json`, for an experiment whose
switch is a setting (for example `--setting agent:transcriptsinglecopy=true`). An environment variable cannot do this: it does
not get through the launcher to srv. The value is read as `true`/`false`, a number, JSON (`[..]`, `{..}`, `"..."`) or plain
text. Which folder a build reads settings from depends on its channel (a local build has its own, a release uses the shared
one), and that is known only once it has run, so the lab starts the instance on the empty data folder, writes the file in every
place a build may look (the `channels` folder and each channel's `config` folder), stops it and starts it again on the same
folder: srv reads the setting at its own start, never after. That costs about 20 s a run.

`result.settings` records what was asked, the files written, and whether the app's full config shows each value (`true`: that
value; `false`: another one; `null`: the key isn't in its config). `compare.mjs` notes when the two sides were given different settings and refuses a side whose runs were not
all given the same. Note that `seenInAppConfig` says the setting was loaded into the app's config, not that the build uses it: the config holds any key, so a build that does not know the setting reports it too (checked on a build from before `agent:transcriptsinglecopy` existed). Whether the effect is there is for the measurement to show. `ab.mjs` takes `--a-setting` and `--b-setting` for the two sides, and `--setting` after the lone `--` for both.

## Where does a slow answer wait? (`--ws-worker`)

When a send's answer reaches the page's JS seconds after srv wrote it (`toArrival` in `result.sends`), it can have waited in
srv's queue for that connection, on the socket, or for the page's main thread. From inside the page the last two look alike.
`--ws-worker` adds a probe that doesn't ride the main thread: a Web Worker (a Blob worker started from the page, so it has the
page's origin and reads its auth key) opens a second websocket, replays the 80-odd event subscriptions the page re-sends when
its own socket is closed and reopened, counts what arrives per 100 ms, and every 250 ms makes a cheap RPC (an `eventsub` for an
event nobody publishes) and times the answer on that socket.

`result.wsWorker` has both streams' totals, the worker's RPC latency, and for the slow sends (5 s or more, else the 8 slowest of
1 s or more) how far the page's stream was behind the worker's in the send's window and the worker's slowest RPC then. Read it
as: a worker RPC of a few ms while the page's stream is seconds behind says srv answers promptly and the lateness is on the
page's side; a slow worker RPC says srv's egress or the socket. The lag compares cumulative message counts of two streams that
differ by a few messages (the page also receives its own RPC answers), so it is good to about 100 ms: the constant offset between them is taken once, in the idle seconds after the worker opens (the lab waits 3 s before the first round for this), not at each send, so a backlog the page already had when a send was made counts. The second subscriber makes
srv do a little more work: use it to find a cause, not for a figure to compare between builds. It needs the window to stay as it
is (not with `--recycle-after`). The forced reconnect that captures the subscriptions can take seconds on a window with many
agents just created (srv completes the close handshake slowly); the lab waits up to a minute for it.

`sends[].eventAgeMs` (how old the message event was when the page's handler ran) is always about 0: the browser creates the
event just before dispatching it, so it cannot show queueing. Use `respArrival` and `toArrival`.

While opening agents (`--open-agents`), the page's slow frames during the opens are reported apart from the workload's, in
`result.opens.frames` (the scripts that filled them, with their bundle positions, and the forced layout) and a summary line.

## Is every transcript intact? (`--verify-transcripts`)

With `--open-agents`, `--verify-transcripts` reads every agent's transcript through `agent:open_tail` (its output stream's line
count and newest line, which stream it is, a hash of its lines and the number of tool calls in it) once the turns and the
heavy turns are over, and again after the agents have been reopened and gone idle. A transcript only ever grows, so the second
reading must begin with exactly the lines of the first (the hash), be on the same stream (`g:` the agent's own, `b:` the block's),
end on a finished turn (`"type":"result"`), and hold at least the tool calls the scripted turns made. A problem makes the run
fail after it has written `result.transcripts` and the summary. A transcript is read whole in pages of `blockfile:read_range` (`agent:open_tail` is capped at 1 MiB by srv), up to 128 MB; above that its line count, newest
line and stream are still checked, not its hash or tool calls.

The line count is **not** expected to stay the same: with the lab's fake CLI a reopen starts the CLI again, srv sends its startup
message and the fake CLI, which remembers nothing, plays the next turn it has (about 450 lines). A real agent would answer
that message too.

## The heavy turns' write volume

`result.heavyIo` (with `--heavy-turns`) is what srv and the other processes wrote while the first agent played its heavy turns, from
just before the first to just after the last: `writtenMB` per group like `io.writtenMB`, and `storageWrittenMB` (the part that
reached storage; none on Windows). `io` covers the regular rounds only. `compare.mjs` shows both for srv, report only, and the
nightly history keeps them as `heavy_srv_written_mb` and `heavy_srv_storage_written_mb`.

## Opening an agent again

`--open-agents N` closes the first N agents' panes after everything else has been measured, then opens each again with the
same request the My Agents list makes (`agent.open`), and records how long it took to get to the first screen:
`result.opens` (and a line in the summary). The pane already logs one `[agent-open]` line per open (the phases since it
mounted: `first_screen` on a build with the first-screen open, `painted` before it, `revealed`, `quiet`) and an
`[agent-open-rest]` line when the older history has finished loading; the lab reads them in the page. What it adds is the
start of the request, which is before the pane exists, so `firstScreenMs` is from the request, not from the mount, and the
longest main-thread task (the Long Tasks API, 50 ms or more) while the open waited and while it ran.

`--heavy-turns K --heavy-kb S` first has the first agent play K extra turns of about S KB each (one turn is many tool
results; the fake CLI doesn't stream those, so a big turn is quick to play). That agent's open is reported as `heavy`, the
others as `regular`: `--heavy-turns 1 --heavy-kb 600` is one long turn that still fits the 1 MB first slice,
`--heavy-kb 2048` one that does not, and `--heavy-turns 80` a conversation of tens of MB. The extra turns are played after
the memory, CPU, I/O and long-task figures are taken, so those are the same with or without these options; the regular
turns of the scripted conversation are unchanged. A build that logs no `first_screen` phase (before the first-screen open)
gets `painted` instead, and the lab does not wait for a rest line from it.

`compare.mjs` shows the median first screen of regular and heavy opens and the worst main-thread task, report only; the
nightly history keeps them as `open_first_screen_ms`, `open_heavy_first_screen_ms` and `open_longest_task_ms`.

## The nightly check

`.github/workflows/perf-regression.yml` runs every day after the nightly artifacts build. It takes the latest build
that still has its Windows portable (B) and the newest older one with a different commit (A), runs the lab on them
interleaved (A B A B ..., 4 runs a side, 12 agents, 10 rounds) with `ab.mjs`, and prints the comparison (medians, ranges
and the figures that moved by more than their margin in `budget.json`) with the list of what was merged between the two
builds in the job summary. With no earlier build to compare with it measures B alone. It is report only: it fails on no
number and is not a required check. `node scripts/perf/memory-lab/nightly.mjs plan` picks the builds (it needs `gh`);
`lib/nightlyplan.mjs` holds the rules.

`.github/workflows/memory-soak.yml` stays as a manual run of one build, and `gate.mjs` checks its result against
`budget.json`. Every line there is `null` and the mode is `report`, so it cannot fail on a number until the operator
sets them; see
[`REPORT_MEMORY_SOAK_GATE_2026_10_18.md`](../../../docs/reports/REPORT_MEMORY_SOAK_GATE_2026_10_18.md).
Locally: `node scripts/perf/memory-lab/gate.mjs memory-lab-out/<label>/result.json`
(it says the result is not the gated workload unless it was run with
`--panes 12 --rounds 10 --seed 1`).

## Passing a setting to the instance

The lab drops every `AGENTMUX_*` variable of your shell before starting the instance, so a switch with that prefix never
arrives. Pass one on purpose with `--env NAME=value` (repeat it for several): it is given to the instance after the
lab's own, and cannot override `AGENTMUX_HOME_OVERRIDE`, `AGENTMUX_CDP_PORT` or `FAKE_CLAUDE_SCRIPT`. `result.instanceEnv`
records what was passed, what the lab set and which `AGENTMUX_*` names of your shell were dropped, and `compare.mjs` says
when its two sides were given different `--env` values (two conditions that differ only by a switch look identical in every
other line, and a switch that never arrived makes a condition run as the baseline). The values are recorded as given.

## Background disk load

A lab run on an idle disk does not show what a busy machine does: srv's transcript store can be held for seconds while
other work (agents' builds) saturates the volume (#5141). `--disk-load <MB/s>` (or `max`) runs a writer that streams
large files, synced chunk by chunk, to the same volume as the lab's data folder from the start of the workload until the
last agent turn is over (it stops before the settle and the forced-GC sample, so memory is measured the same way). Pass
the same flags on both sides of an A/B: `compare.mjs` refuses runs with a different load or cap, and `result.diskLoad`
records the rate and what was written. It writes at most `--disk-load-cap-gb` (default 20) in a run: `max` on a fast disk
writes hundreds of MB/s, and a stress run is wear on an SSD. A run whose writer reaches the cap before the workload ends
is rejected (the rest of it had no load): lower the rate or raise the cap. A fast NVMe may not be stressed by one writer; a SATA SSD or a
hard disk is where it matters. `--out` picks the volume. `result.volume` records how full that volume was at the start and
the end: an SSD slows badly as it fills, so a stall on a nearly full disk is a different finding from one on an empty disk.

## A run that isn't the load it was asked for

A run whose agents never played reports a plausible, far too low number (an
idle window), and one with extra agents a far too high one. So the lab checks,
and refuses (then starts over, up to `--attempts` times) a run that has:

- a different number of agent panes, agents made or agent definitions than the
  layout asked for (agent creation has been seen to make every agent twice
  under load);
- an agent whose transcript shows no tool rows after the first round (it never
  played);
- turns still being played when the last round's wait is over (a slow build is
  waited for, up to 5 minutes, instead of measured with fewer turns played);
- a transcript store (`shared/agents/transcripts`, database and log) that grew
  by less than 80% of what the played turns weigh.

Each refused attempt leaves `invalid-run-<n>.txt` in the run's folder.

## What it reports

Private working set and private commit, in MB, per process group, and the app
total (the number the budget is about):

| group | what |
|---|---|
| renderer | the window renderers (`--type=renderer`) |
| gpu | the GPU process |
| srv | the background service |
| host | the launcher and the CEF host |
| utility | Chromium's utility processes (network, storage, audio) |
| other | crashpad and the like |
| panes' children | the agents' own CLIs and shells (here the fake Claude's `node` process); **not** counted in the app total |

The metric is each OS's own (`agentmux_procstats::MEMORY_METRIC`): Windows
private working set, macOS `phys_footprint`, Linux anonymous resident plus swap.
Only compare runs from the same OS. On macOS the totals are per-process
footprints summed, an upper bound (a region shared between processes is counted
in each); `footprint`'s own total over the app's processes, which counts it
once, is kept in `result.json` under `final.footprint`.

`result.json` has the whole run (the curve, the final's per-process numbers and
allocator roots, the pages); `summary.md` is the table.

## What fills the page's slow frames

`result.pageFrames` (and one line in `summary.md`) says what filled the page's slow frames, so a stall can be
told apart as script or as the rendering step, and a script can be named. It uses the Long Animation Frames API (a
frame of 50 ms or more; Chromium 123 and later, an older one records nothing and says so) and observes only: it
changes no behaviour in the app. `lib/frames.mjs` has the page script and the summary.

- The frames' time split into script and rendering (style and layout), and how much of the script time was
  **forced** style and layout: a script reading layout after writing the DOM.
- The longest script of each frame, and the script that forced the most layout, as the invoker and its position in
  the page's bundle. The production bundle is minified, so look the position up in the build's `index-*.js`. Only the
  script that forced the most in each frame is named, so `forcedAttributedMs` is part of the forced total, not all of it.
- How long the page's websocket handler took per message size, and the biggest messages with their handling time:
  a big message is one task that everything behind it waits for.
- Panes flushed per frame, from a counter in the app's stream scheduler that the lab switches on
  (`globalThis.__labFrameCounter`). A build without the counter says "not available", so old portables still run.

Frames in the lab's own forced-GC windows are left out and counted. The page keeps at most 5,000 frames; past that
they are counted, not kept, and `truncated` is set.

## Reading the numbers

- Compare **the final**, taken after a forced GC. The curve is not forced:
  it includes garbage the GC hasn't collected yet, so it runs well above the
  final (4 agents: about 780 MB of renderers in the curve, about 500 MB in the
  final).
- The workload is synthetic and compresses hours of agent work into minutes, so
  the absolute sizes are higher than a real hour would give. What carries over
  is which process and which allocator a change moves, and by how much against
  the same load.
- Tested on Windows. The scripts are cross-platform, but the portable's
  executable path (`lib/driver.mjs` `portableExe`) and the macOS and Linux
  numbers haven't been checked.

## Files

| file | what |
|---|---|
| `run.mjs` | the lab |
| `compare.mjs` | two runs side by side |
| `lib/driver.mjs` | starts the instance, drives it over CDP (account, tabs, agents, messages) |
| `lib/sample.mjs` | one sample: `memtree` OS numbers, the memory-infra dump, page stats |
| `lib/summary.mjs` | grouping, steady values, comparison (pure; tested) |
| `lib/conversation.mjs` | the scripted conversation |
| `memory-lab.test.mjs` | `npx vitest run scripts/perf/memory-lab` |
