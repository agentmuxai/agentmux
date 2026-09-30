# `muxlog` — the AgentMux log viewer

`muxlog` is shipped in every AgentMux terminal (bash / zsh / pwsh / fish). It
**discovers, renders, and follows** the AgentMux logs for you — across every
running instance — so debugging never starts with a file hunt.

> One implementation: the shells delegate to a small Node core
> (`muxlog.mjs`, deployed next to the shell rcfiles). Run it directly from any
> subshell or script with `node ~/.agentmux/shell/muxlog.mjs …` if the `muxlog`
> function isn't loaded (e.g. inside a tool-spawned `bash -c`).

---

## Quick start

```bash
muxlog ls           # what logs exist, newest first, with version + age
muxlog              # follow the most-recently-active host log
muxlog srv          # follow the active sidecar log
muxlog bridge       # startup-handshake trace — debug "Can't reconnect" loops
muxlog errors       # just the ERROR/WARN lines across host + sidecar
muxlog swarm        # subagent/swarm lifecycle trace — spawn/name/status, debug duplicate groups
muxlog swarm -d <id> # same, filtered to one dispatch + a match-count verdict
muxlog auth         # provider auth/identity trace — login, OAuth dir wiring, unlink/logout
muxlog phases       # merged turn-phase timeline for one pane — defaults to your own ($AGENTMUX_BLOCKID)
muxlog admission AgentA  # why won't this agent run here — across ALL instances, with a verdict
muxlog opens        # how long agent opens took — one row per open, then p50/p95
muxlog help         # full usage
```

`muxlog` defaults to **your own running instance** when it can tell what that
is — `$AGENTMUX_CHANNEL` is already set in every agent pane's environment, so
a recipe run from inside an agent pane resolves to that instance's own log
first, not a same-version sibling's. Falls back to the
**most-recently-active** instance across the whole machine only when nothing
matches your own channel (e.g. `launcher`, which has no per-channel log at
all) or `$AGENTMUX_CHANNEL` isn't set (a human running `muxlog` outside any
agent pane). `-i <substr>` always overrides both. `muxlog ls` shows every
instance so you can sanity-check which one a bare `muxlog swarm`/`errors`/
etc. actually resolved to.

---

## Targets

| Target | Log |
|--------|-----|
| `host` (default) | CEF host — windows, IPC bridge, `[fe]` frontend lines, heartbeat |
| `srv` | sidecar — RPC, blocks, shells, config |
| `launcher` | launcher — process/DLL/startup diagnostics (portable & installed only) |
| `fe` | the host log, pre-filtered to frontend `[fe]` lines |
| `all` | host (alias for the active host log; combined view is a roadmap item) |

## Actions

| Action | Meaning |
|--------|---------|
| `tail` (default) | print the last `-n` lines, then **follow** (like `tail -f`) |
| `cat` | the whole log, rendered |
| `grep <regex>` | lines whose **message** matches `<regex>` (not the whole JSON) |

```bash
muxlog host                 # follow host
muxlog srv cat              # dump the sidecar log
muxlog host grep "window\.api"
```

## Options (any position)

| Option | Effect |
|--------|--------|
| `-i <substr>` | pick the instance whose log path / branch / version contains `<substr>` |
| `-n <N>` | history lines before following (default 200) |
| `-a` | include agent-transcript noise (the sidecar's `…→ blockfile` lines), excluded by default |
| `--grep <re>` | filter on the message field only |
| `--level a,b` | only these levels (`error,warn,info,debug`) |
| `--target <s>` | only lines whose tracing target contains `<s>` |
| `--since <ts>` | only lines at/after ISO `<ts>` (e.g. `2026-06-15T23:30`). Logs roll daily by UTC date (`….log.<YYYY-MM-DD>`); with `--since` the resolved log is widened to its older daily files that reach the window, so an incident just before UTC midnight isn't missed. Without it, only the newest file is read, as before |
| `--until <ts>` | only lines at/before ISO `<ts>`; widens to daily files the same way |
| `--agent <a>` | only lines about agent `<a>`: a name in any case (`AgentA`, `agenta`) or its uid. Matches the structured `agent`/`agent_id`/`uid`/`definition_id` fields, the message and `why` text, and the `block_id` of the agent's own panes (so `persistent process spawned` lines, which carry only a block id, are included). Name ↔ uid is resolved from the lines themselves |
| `--instances all\|live\|<s1,s2>` | merge several instances into **one chronological timeline**, each line tagged `v<version>/<channel>` (e.g. `v0.57.6/7a8245ae`). `all`: every instance whose logs reach the window, running or not; `live`: skip ones whose liveness probe says `dead`; otherwise `-i`-style substrings. One-shot (no follow). The header lists only instances that contributed lines |
| `--raw` | emit the original NDJSON (don't render) |
| `--verbose` | append the structured fields after the message |

```bash
muxlog srv -i fix-shell grep "shell\.spawn"   # a specific dev branch's sidecar
muxlog host --level error,warn                 # only problems, then follow
muxlog srv --since 2026-06-15T23:30 cat        # a time window
```

## Recipes

| Recipe | What it shows |
|--------|---------------|
| `muxlog ls` | every instance's logs: target, version, source (`shared` / `dev:<branch>` / `channel:…`), **LIVE** (`live`/`dead`/`?` — a real check, not inferred from log mtime: probes the instance's own `ipc-port-*` file with a `GET /health` request, checking both the `data` and `cef-cache` sibling dirs since `agentmux-cef` writes the port file to `cef-cache` for `task dev` instances and to `data` for portable/installed builds; `?` means genuinely unknown, e.g. neither sibling dir found, never a liveness verdict of its own), age, size, path |
| `muxlog mem` (alias `doctor`) | system **commit-free** + derived pressure level (the OOM-relevant ceiling, not physical RAM) + the count and footprint of live AgentMux processes — makes multi-instance commit pressure visible before the cliff (`SPEC_MEMORY_PRESSURE_SUPERVISION_2026_06_16` §5.G) |
| `muxlog errors` | ERROR + WARN across the active host and sidecar |
| `muxlog bridge` | the startup handshake — `Loading URL`, `Injected IPC …`, `backend-ready`, `window.api`, `Bootstrap failed` — correlated in time, so a reconnect loop is obvious at a glance |
| `muxlog swarm` | subagent/swarm lifecycle — spawn, `display_name` resolution (`subagent.GenerateName`), status transitions (`reconcile_stale_subagents`' active→abandoned pass), and the `parent_block_id`/`session_id`/`workflow_id` each event carries — filters the sidecar log to `subagent_watcher.rs`'s tracing target so a duplicate-group or stuck-status report is diagnosable from logs alone (srv-side only; there's no host-side subagent logging to combine in). `-d <dispatch_id>` filters to just that one dispatch and prints an explicit verdict line (`N lines mention '<id>'`, or a "0 matches" explanation with next steps) instead of silently printing nothing — checks the RAW line (message OR any structured field, e.g. `dispatch_id`), not just the rendered message `--grep` would. Productizes the manual "did this ever get processed here" correlation `docs/reports/REPORT_MUXSPECT_MUXLOG_CROSS_CHANNEL_INSPECTION_2026_08_22.md`'s own investigation had to do by hand |
| `muxlog auth` | provider auth / identity lifecycle — the login flow (`auth.start` / `auth.spawn` child start+exit / `auth.cancel`), OAuth config-dir wiring, `auth success (direct-account)` persistence, `CheckCliAuth` + the one-time `claude auth:` credential import, and the logout side (`identity.unlink:` provider unlinks, `identity.delete:` account + keychain removal). Auth events span multiple srv modules, so this filters on the **message** vocabulary rather than a tracing target — pass your own `--grep` to override, or combine `--since`/`--level`/`-i` as usual. Ideal for repeated login/logout stress runs |
| `muxlog phases [<block-id>]` | **Merged, chronological turn-phase timeline for one agent pane** — combines the frontend's `[wave-turn]` transition log (host) with the backend's `[health] turn_active flip` log (srv) into a single, correctly-ordered stream, instead of the two separate files you'd otherwise have to cross-reference by hand. Defaults to your own pane via `$AGENTMUX_BLOCKID` (already set in every agent's shell env) — pass an explicit block id to look at a different pane. Host and srv logs are resolved by actually checking which log **contains** this pane's lines (not just picking "most recently active"), so this stays correct even with several instances — or several retained dev builds of the same branch — running at once. `[health]` lines show their `active`/`was_active`/`exit_code` fields inline (the srv side's whole reason for being in the timeline — those never live in the message text). A `watchdog: tick #N` heartbeat line appears every ~60s as direct proof the recovery watchdog is alive, not just inferred from silence. Every generic option (`--grep`, `--level`, `--target`, `-a`) composes on top of the recipe's own per-pane filter, same as `swarm`/`auth`/`bridge`; `--raw` emits the original NDJSON for matched lines only. See `docs/specs/SPEC_AGENT_TURN_PHASE_TIMELINE_LOGGING_2026_08_18.md` |
| `muxlog admission [<agent>]` | **Why won't this agent run here.** The agent's admission, take-over and lifecycle lines (`agent_admission.*`, `registry: … changed hands`, `self-quit`, `persistent process spawned`, `turn_active flip (process exited)`, muxbus registration/lease lines) from **every instance**, merged into one timeline, with `holder`/`released`/`epoch`/`block_id`/`why`/`to` shown inline. Ends with a per-instance verdict (last admission state, who the relay names as holder, whether that instance still has a live pane for the agent) and, when it recognizes the pattern, a **diagnosis** that names the fix PRs and how to recover. Implies `srv`, `--instances all`, `--agent <agent>` (default `$AGENTMUX_AGENT_ID`) and `--since` 24 h ago (override with your own). See `docs/specs/SPEC_MUXLOG_AGENT_ADMISSION_TIMELINE_2026_09_27.md` |
| `muxlog opens [<agent>]` | **How long agent opens take.** One row per `[agent-open]` line — the frontend logs one per open (`frontend/app/view/agent/open-trace.ts`) — from **every instance**: when, instance, agent, `source` (`my-agents` = a My Agents click, `mount` = a pane that mounted without one, e.g. startup restore), `outcome` (`quiet`, `unsettled`, `failed`, `timeout`, `closed`, `superseded`), and ms since the click/mount for ResolveCli (with `cli_source`: `installed` means npm ran), the first history row painted, the reveal and quiet (no frame gap > 50 ms for 1.5 s). The clock stops at reveal + quiet, so a login or account prompt the pane shows afterwards is never counted as latency; `auth` records the phase the pane landed in. Ends with p50/p95 to first row and to quiet, per source. Defaults to `--instances all` and `--since` 24 h ago; `-n` caps the rows. See `docs/specs/SPEC_AGENT_OPEN_LATENCY_2026_09_27.md` §4.6 |

### Worked example: a stuck agent whose Take over does nothing

```bash
muxlog admission AgentA --since 2026-09-26T23:50
```

It prints both instances' lines interleaved (the 0.57.6 instance quitting AgentA, the 0.57.8 instance being fenced, its take-over, and the 0.57.6 instance answering `released=0`), then:

```
verdict:
  v0.57.8/6addbd3a   denied at 2026-09-26T23:58:17Z (relay names: this computer, channel local-main-b28b7a-7a8245ae, v0.57.6); no live pane
  v0.57.6/7a8245ae   no admission lines; no live pane

diagnosis: stale relay lease: the relay names v0.57.6/7a8245ae as holder, but v0.57.6/7a8245ae has had no live pane for the agent since 23:52:19, and a take-over there answered released: 0 … Recover: quit or restart v0.57.6/7a8245ae …
```

`agenta` or the agent's uid give the same result.

### Worked example: are agent opens slow?

```bash
muxlog opens            # every agent, last 24 h
muxlog opens Lazo --since 2026-09-27T04:00
```

```
when           instance           agent            source    outcome      cli cli_source    painted revealed  quiet  total auth
09-27 04:01:57 v0.58.0/7a8245ae   Lazo             my-agents quiet        180 installed         812      815   1341   1341 authenticated

summary:
  my-agents 1 open (quiet 1); first row p50 812 p95 812; quiet p50 1341 p95 1341; 1 installed the CLI
```

Read a slow row left to right: a large `cli` with `cli_source=installed` is an npm install on the open path (F1); a gap between `revealed` and `quiet` is main-thread work after the pane appeared (F3/F4); `auth=first-login` means the pane ended at a login prompt, which the clock doesn't count. `outcome=unsettled` means the page never went quiet within 15 s (often a hidden window: `hidden=1`).

---

## How discovery works (and why it's better)

AgentMux logs live in **three** root trees, not one:

```
~/.agentmux/logs/                                  shared: launcher, and sidecar
                                                    when AGENTMUX_LOG_DIR isn't set
~/.agentmux/dev/<branch>/<hash>/logs/              task dev — keyed on the git branch
~/.agentmux/channels/<channel>/versions/<v>/logs/  portable / per-build instances —
                                                    both host AND sidecar, as of the
                                                    sidecar honoring AGENTMUX_LOG_DIR
```

The old `muxlog` followed a single version-pinned pointer
(`current-host-v<version>.path`) in the shared dir only. With many instances that
pointer routinely resolved a **stale** instance — and it was blind to the
`dev/<branch>` logs entirely. `muxlog ls` / the most-recently-active default fix
both: every root is scanned, results are ranked by modification time, and you can
always pin a specific one with `-i`.

## Notes

- **Rendering** turns each NDJSON line into `HH:MM:SS  LEVEL  target  message`.
  Tracing targets are shortened (`agentmux_cef::commands::backend` → `cef:backend`).
  Use `--raw` for the original JSON, `--verbose` to see the structured fields.
- **Transcript noise is excluded by default.** The sidecar log is mostly agent
  conversation (`subprocess stdout → blockfile`); `muxlog` drops it so searches
  hit real events. Pass `-a` to include it.
- **Follow is efficient** on huge logs — a plain `tail` reads only the end of the
  file; a `grep`/recipe/filter scans the whole history so no match is missed.
- **Fallback:** if `node` isn't on `PATH`, `muxlog` degrades to the legacy
  pointer-based `tail`/`cat`/`grep` so logs are never wholly inaccessible.

Implementation: `crates/srv/src/backend/shellintegration/muxlog.mjs` (core) and
the per-shell `muxlog` delegators in the same directory.
