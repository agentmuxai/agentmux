# Memory lab

Measures a production build's memory under a repeatable agent load, so a change
can show what it saved. The method is §2 of
[`REPORT_RENDERER_MEMORY_ROOT_CAUSES_2026_10_12.md`](../../../docs/reports/REPORT_RENDERER_MEMORY_ROOT_CAUSES_2026_10_12.md),
made into scripts. The budget it serves is 500 MB for the app's own processes.

## Measure a build

1. Build a portable of the branch: `task package` (a **production** build; a
   `task dev` build runs Vite and SolidJS in dev mode and inflates every number).
2. Run the lab on it:

   ```bash
   node scripts/perf/memory-lab/run.mjs --portable <the portable folder> --label my-branch
   ```

   Options: `--panes 4|12` (4 agents in one tab, or 12 over 4 tabs as 4/2/4/2;
   default 4), `--rounds 10`, `--gap-s 15` (seconds between rounds), `--every 5`
   (a curve point every N rounds), `--settle-s 20`, `--samples 3`, `--seed 1`,
   `--port 9333` (the instance's CDP port; 9222 is refused, and so is a port
   something already answers on: pick another when two labs run at once),
   `--attempts 2` (see "A run that isn't the load"), `--out <dir>`
   (default `memory-lab-out/`), `--keep` (leave the instance running),
   `--memtree <path>` (default: built from `crates/procstats` on first use).

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
