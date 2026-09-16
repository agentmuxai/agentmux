# `muxsh` — open editor/browser panes from the terminal

**Status:** living — reference doc kept current alongside the tool, same as `MUXLOG.md`/`MUXSPECT.md`.

`muxsh` is shipped in every AgentMux terminal (bash / zsh / pwsh / fish), same
family as `muxlog`/`muxspect`/`muxopen`. It's AgentMux's own successor to
Wave Terminal's `wsh` CLI — not a revival of it (AgentMux retired the
inherited `wsh` entirely in 2026-04 after finding zero real usage; see
`docs/specs/archive/SPEC_RETIRE_WSH_2026_04_12.md`), but a narrow tool for
the one gap that turned out to be real: nothing on the command line could
open a pane. See `docs/reports/REPORT_WSH_STYLE_CLI_FOR_AGENT_APP_API_2026_09_16.md`
for the full research behind this, and `docs/specs/SPEC_MUXSH_CLI_2026_09_16.md`
for the design.

> One implementation: the shells delegate to a small Node core
> (`muxsh.mjs`, deployed next to `muxlog.mjs`/`muxspect.mjs`/`muxopen.mjs`).
> Run it directly with `node ~/.agentmux/shell/muxsh.mjs …` if the `muxsh`
> function isn't loaded (e.g. inside a tool-spawned `bash -c`).

---

## Quick start

```bash
muxsh open <file>              # open a file in an editor pane
muxsh web <url>                # open a URL in a browser pane
muxsh help                     # full usage
```

Both are thin wrappers over `POST /api/v1/pane/open` — the same route the
frontend and the `OpenEditor` MCP tool use, authenticated the same way
`muxlog`/`muxspect`/`muxopen` already are: `$AGENTMUX_LOCAL_URL` +
`$AGENTMUX_AUTH_KEY`, inherited from the pane environment. No new IPC, no
new auth scheme.

## `open` — options

| Option | Effect |
|---|---|
| `--title <t>` | pane/tab title (default: file name) |
| `--split <right\|left\|down\|up>` | split direction relative to the calling pane (default: `right`) |
| `--collapse-tree` | open with the file-tree sidebar collapsed |
| `--floating` | open in a floating window instead of a docked split |
| `--no-focus` | open without focusing the new pane |

```bash
muxsh open ~/notes.md
muxsh open ./src/main.rs --split down --title "main.rs"
muxsh open ./report.pdf --floating
```

## `web` — options

| Option | Effect |
|---|---|
| `--title <t>` | pane/tab title |
| `--floating` | open in a floating window instead of a docked split |
| `--no-focus` | open without focusing the new pane |

```bash
muxsh web https://grafana.internal/d/api-latency
muxsh web https://docs.agentmux.ai --floating
```

`--collapse-tree` is editor-only and is rejected with an error if passed to
`muxsh web`, rather than silently ignored. `--split` works for both.

Splitting relative to the calling pane requires `$AGENTMUX_BLOCKID` (set in
every AgentMux terminal pane). Without it, the new pane is inserted at the
tab root instead — the server's placement logic only splits when it has a
reference block id, regardless of `--split` — so `muxsh` only sends
`split_direction` when it actually knows the calling pane's block id.

## What this doesn't do (yet)

Only `editor` and `browser` panes are exposed today, even though the
underlying route also accepts `term`/`sysinfo`/`help`/`media` — see
`SPEC_MUXSH_CLI_2026_09_16.md` §5 for why the first cut is scoped this
narrowly, and how to extend it.

Each call always opens a **new** pane — unlike `muxopen`, which is
idempotent per agent, `muxsh` has no dedup concept: files and URLs aren't
identity-scoped the way agents are.

Same single-instance scope as the rest of this tool family: `muxsh` opens
into the instance the calling pane belongs to. Cross-instance opening is
not implemented here.

Implementation: `agentmux-srv/src/backend/shellintegration/muxsh.mjs` (core)
and the per-shell `muxsh` delegators in the same directory.
