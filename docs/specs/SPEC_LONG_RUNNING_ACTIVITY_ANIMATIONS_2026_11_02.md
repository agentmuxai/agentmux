# Spec: one ASCII animation per kind of long-running work

**Status:** active. Approved by the operator on 2026-11-04 ("proceed with best recommendations"); built under #22325. Author: AgentY. Builder: Agent1 (UI kit, motion).
**Asked for (operator, 2026-11-02):** "lets also give an alternate animation for each type of long-running process. reserve the rotating slash for the main progress. the long running should have a unique ascii animation that falls into the category of operation it is, design a set of buckets and the ascii animation it goes with".

## 1. The decision

Which buckets of long-running work get their own animation, what each animation is, and how a running operation is sorted into a bucket. The rotating slash stays the one sign that an agent's turn is in progress.

## 2. What exists today

- `frontend/app/element/AsciiSpinner.tsx`: frames `| / - \`, one shared 120 ms timer for every spinner on screen (so dozens never strobe out of phase), a static `*` when the motion policy reduces `status`.
- It is used for everything that runs: the agent pane's Working row (`working-row.tsx`), the preparing row (`AgentPreparingRow.tsx`), each running tool in the activity list (`ActivityRow.tsx`) and the Swarm's status chip (`swarm-status-chip.tsx`). So a 20-minute `cargo build`, a `git push` and the turn itself all look the same.

## 3. Rules every animation follows

1. **The slash is reserved.** `| / - \` means one thing: the agent's turn is in progress (Working row, preparing row, Swarm chip). No bucket uses `|`, `/`, `-` or `\` as its moving part.
2. **ASCII only, monospace, fixed width.** Every bucket animation is exactly **3 cells** wide, so a row never shifts as frames change. The slash stays 1 cell.
3. **One clock.** All animations run off the existing shared 120 ms ticker. A bucket's frame period is a multiple of it (120, 240, 360 or 480 ms), and its frame is `floor(tick / (period / 120)) % frames.length`, so identical buckets on screen move in step.
4. **Reduced motion keeps the meaning.** When the motion policy reduces `status`, each bucket shows its own still glyph (below), not a shared `*`, so the kind of work is still readable.
5. **Words carry it too.** The glyph is `aria-hidden`; the row keeps its text, and the tooltip reads the bucket name and elapsed time ("Building, 4m 12s").
6. **Only long-running work.** A tool call shows its bucket animation once it has run for 3 s; before that it shows nothing extra, so quick calls don't flicker.

## 4. The buckets

Frames are shown between backticks, one per step; a space is a blank cell.

| # | Bucket | Covers | Animation (3 cells) | Period | Still | Why it reads that way |
|---|---|---|---|---|---|---|
| 0 | **Turn in progress** (reserved) | the agent's turn, preparing, the Swarm chip | `\|` `/` `-` `\` (1 cell, unchanged) | 120 ms | `*` | the app's one "working" sign |
| 1 | **Build and compile** | `cargo build/check`, `tsc`, `vite build`, `task build`, bundling, LTO | `+x+` `x+x` | 240 ms | `+x+` | two gears meshing |
| 2 | **Tests and checks** | `cargo nextest/test`, `vitest`, `npm test`, `check-*` scripts, lint, gates | `v  ` `vv ` `vvv` `   ` | 240 ms | `vvv` | ticks piling up, as test runners print them |
| 3a | **Sending** | `git push`, uploads, `gh-agent pr create`, publishing a file, sending a message with files | `>  ` ` > ` `  >` | 120 ms | ` > ` | a packet leaving |
| 3b | **Receiving** | `git clone/fetch/pull`, downloads, web fetch, a browser page loading | `  <` ` < ` `<  ` | 120 ms | ` < ` | a packet arriving |
| 4 | **Install and set up** | `npm install`, `cargo fetch`, `pip install`, toolchain or CLI install, container image pull, container start | `[ ]` `[.]` `[o]` `[#]` | 240 ms | `[#]` | a box filling |
| 5 | **Search and scan** | Grep, Glob, `rg`/`find`, history search, indexing, an audit scan | `o  ` ` o ` `  o` ` o ` | 120 ms | ` o ` | a scanner sweeping back and forth |
| 6 | **Deploy and release** | `deploy infra`, release workflows, `cdk`, landing deploy, store submissions | ` . ` ` : ` ` i ` ` ! ` ` ^ ` `   ` | 240 ms | ` ^ ` | something rising and launching |
| 7 | **Delegated work** | a subagent (Agent tool), a worker's task, a background workflow | `-o-` `>o<` `-o-` `<o>` | 240 ms | `-o-` | a hub handing work out and taking it back |
| 8 | **Compacting and summarising** | context compaction, continuity summaries, any model call outside the turn | `< >` `<.>` `<:>` `<#>` | 240 ms | `<#>` | something being squeezed |
| 9 | **Waiting on someone** | CI, muxreview or Codex, a person's approval or answer, another agent's reply, an external service | `( )` `(.)` `(o)` `(O)` `(o)` `(.)` | 360 ms | `(o)` | a slow breath: holding, not stuck |
| 10 | **Held or asleep** | quota hold, a scheduled start, a hibernated pane waking, an idle hold | `z  ` `zZ ` `zZz` `   ` | 480 ms | `zZz` | sleeping |
| 11 | **Other long-running work** (fallback) | anything unclassified that has run over 3 s | `.  ` `.. ` `...` `   ` | 360 ms | `...` | plain ellipsis |

No two buckets share a moving glyph, so each one is recognisable at a glance and in a screenshot.

## 5. How work is sorted into a bucket

One pure function, `activityBucket(activity) -> Bucket`, in `frontend/app/element/` next to the spinner, with a table of rules tested case by case. First match wins:

1. **The source says it.** Events that already know their kind map directly: compaction and continuity (8), subagent/worker/task (7), quota hold, schedule, hibernation (10), approval prompts, review and CI waits (9), browser navigation (3b), container start or image pull (4).
2. **Tool name.** Grep, Glob, history search (5); WebFetch (3b); Agent/Task (7).
3. **Bash command, by its first real program and subcommand** (after `cd … &&`, env assignments and wrappers like `timeout`, `secrets escalate … --`): build words (1), test and check words (2), `git push` (3a), `git clone/fetch/pull`, `curl`/`wget` (3b), install words (4), `rg`/`find`/`grep` (5), deploy and release words (6). The word lists live in the one table, not scattered.
4. **Otherwise** 11.

A bucket never changes while the same operation runs, so the animation doesn't jump.

## 6. Where it shows

- **Agent pane, activity list (`ActivityRow`)**: the running tool's row, after 3 s. Main use.
- **Agent pane, Working row**: keeps the slash; next to it, the current long-running operation's bucket animation and its name ("Building 4m"), so the row says both "the turn is running" and "it's waiting on a build".
- **Swarm status chip**: keeps the slash for "working"; when the agent's current operation has run over 30 s, the chip shows that bucket's animation instead, so a fleet view separates "building", "waiting on CI" and "held for quota". Its tooltip names the bucket.
- **Status bar background tasks**, if present: the same mapping.

## 7. Build plan (one PR, Agent1)

1. `ActivityAnimation` component and the bucket table (frames, period, still, label) beside `AsciiSpinner`, on the same shared ticker; `AsciiSpinner` itself unchanged.
2. `activityBucket()` with the rules of section 5 and a test per rule, plus a test that no bucket uses a slash glyph and every frame is 3 cells.
3. Wire it into `ActivityRow`, the Working row and the Swarm chip as in section 6; reduced-motion stills; tooltips.
4. Screenshots of each bucket, moving and still, on the task.

## 8. Open questions for the operator

1. Are 3-cell animations acceptable in the Swarm chip, or should the chip use a 1-cell version (the still glyph's middle character)?
2. Do waiting on CI or review (9) and held for quota (10) also need a colour, or only the animation?
3. Should the 3 s threshold be a setting?
