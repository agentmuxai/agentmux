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
muxlog help         # full usage
```

`muxlog` always defaults to the **most-recently-active** instance. With several
instances running (dev + portables + different versions), that's almost always
the one you mean — and `muxlog ls` shows the rest.

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
| `--since <ts>` | only lines at/after ISO `<ts>` (e.g. `2026-06-15T23:30`) |
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
| `muxlog ls` | every instance's logs: target, version, source (`shared` / `dev:<branch>` / `channel:…`), age, size, path |
| `muxlog mem` (alias `doctor`) | system **commit-free** + derived pressure level (the OOM-relevant ceiling, not physical RAM) + the count and footprint of live AgentMux processes — makes multi-instance commit pressure visible before the cliff (`SPEC_MEMORY_PRESSURE_SUPERVISION_2026_06_16` §5.G) |
| `muxlog errors` | ERROR + WARN across the active host and sidecar |
| `muxlog bridge` | the startup handshake — `Loading URL`, `Injected IPC …`, `backend-ready`, `window.api`, `Bootstrap failed` — correlated in time, so a reconnect loop is obvious at a glance |
| `muxlog swarm` | subagent/swarm lifecycle — spawn, `display_name` resolution (`subagent.GenerateName`), status transitions (`reconcile_stale_subagents`' active→abandoned pass), and the `parent_block_id`/`session_id`/`workflow_id` each event carries — filters the sidecar log to `subagent_watcher.rs`'s tracing target so a duplicate-group or stuck-status report is diagnosable from logs alone (srv-side only; there's no host-side subagent logging to combine in) |

---

## How discovery works (and why it's better)

AgentMux logs live in **three** root trees, not one:

```
~/.agentmux/logs/                                   shared: sidecar, launcher, some host
~/.agentmux/dev/<branch>/<hash>/logs/               task dev — keyed on the git branch
~/.agentmux/channels/local-*/versions/<v>/.../logs/ portable / per-build instances
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

Implementation: `agentmux-srv/src/backend/shellintegration/muxlog.mjs` (core) and
the per-shell `muxlog` delegators in the same directory.
