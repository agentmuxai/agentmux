# `muxsh` — a terminal CLI over the Agent App API

**Status:** living — reference doc kept current alongside the tool, same as `MUXLOG.md`/`MUXSPECT.md`.

`muxsh` is shipped in every AgentMux terminal (bash / zsh / pwsh / fish), same
family as `muxlog`/`muxspect`/`muxopen`. It's AgentMux's own successor to
Wave Terminal's `wsh` CLI — not a revival of it (AgentMux retired the
inherited `wsh` entirely in 2026-04 after finding zero real usage; see
`docs/specs/archive/SPEC_RETIRE_WSH_2026_04_12.md`), but a purpose-built
collection scoped per `docs/specs/SPEC_MUXSH_FULL_COLLECTION_2026_09_16.md` —
every subcommand maps to a real, `$AGENTMUX_AUTH_KEY`-gated App API route,
not an agent-identity-signed one (a distinction that ruled out a couple of
`wsh` commands that looked buildable on paper — see "What's not here" below).

> One implementation: the shells delegate to a small Node core
> (`muxsh.mjs`, deployed next to `muxlog.mjs`/`muxspect.mjs`/`muxopen.mjs`,
> importing shared auth/fetch plumbing from `lib/muxclient.mjs`). Run it
> directly with `node ~/.agentmux/shell/muxsh.mjs …` if the `muxsh` function
> isn't loaded (e.g. inside a tool-spawned `bash -c`).

---

## Quick start

```bash
muxsh open <file>              # open a file in an editor pane
muxsh web <url>                # open a URL in a browser pane
muxsh view <path-or-url>       # open the right pane type automatically
muxsh edit <file>              # like open, but always forces the editor view
muxsh pane list                # list tabs/panes in your workspace
muxsh run <cmd...>              # run a background shell
muxsh config edit               # open settings.json
muxsh config path               # print AgentMux's data dir
muxsh agent list                # list reachable agents
muxsh agent send <name> <msg>   # message a running agent
muxsh help                      # full usage
```

Every command is authenticated the same way `muxlog`/`muxspect`/`muxopen`
already are: `$AGENTMUX_LOCAL_URL` + `$AGENTMUX_AUTH_KEY`, inherited from the
pane environment. No new IPC, no new auth scheme.

## `open` / `web` / `view` / `edit`

Thin wrappers over `POST /api/v1/pane/open` — the same route the frontend and
the `OpenEditor` MCP tool use.

- **`open <file>`** — editor pane.
- **`web <url>`** — browser pane.
- **`view <path-or-url>`** — guesses the right pane type: a URL → browser, a
  media extension (`.png`/`.jpg`/`.gif`/`.mp4`/`.pdf`/...) → media, anything
  else → editor. The guess list isn't derived from a canonical AgentMux
  source (open question, see the full-collection spec §7.4) and may drift
  from what the frontend itself considers "media."
- **`edit <file>`** — like `open`, but never guesses; always the editor view.

| Option | Effect | Applies to |
|---|---|---|
| `--title <t>` | pane/tab title | all four |
| `--split <right\|left\|down\|up>` | split direction relative to the calling pane (default: `right`) | all four |
| `--collapse-tree` | collapse the file-tree sidebar | editor view only — rejected with an error on `web`/a URL-guessed `view` |
| `--floating` | open in a floating window instead of a docked split | all four |
| `--no-focus` | open without focusing the new pane | all four |

```bash
muxsh open ~/notes.md
muxsh open ./src/main.rs --split down --title "main.rs"
muxsh web https://grafana.internal/d/api-latency
muxsh view ./report.pdf          # -> media pane, no need to know that yourself
muxsh edit ./photo.png            # forces the editor view despite the extension
```

Splitting relative to the calling pane requires `$AGENTMUX_BLOCKID` (set in
every AgentMux terminal pane). Without it, the new pane is inserted at the
tab root instead — the server's placement logic only splits when it has a
reference block id, regardless of `--split` — so `muxsh` only sends
`split_direction` when it actually knows the calling pane's block id.

Each call always opens a **new** pane — unlike `muxopen`, which is
idempotent per agent, `muxsh` has no dedup concept: files and URLs aren't
identity-scoped the way agents are.

## `pane list`

```bash
muxsh pane list
muxsh pane list --json
```

Lists tabs/panes in the calling pane's own workspace (`GET /api/v1/tabs`).
Read-only.

## `run`

```bash
muxsh run <cmd...>               # start a background shell, prints its shell_id
muxsh run --status <shell_id>    # is it still running?
muxsh run --stop <shell_id>      # stop it
```

Thin wrapper over `POST /api/v1/shell/{create,status,stop}` — the same
routes the `Shell`/`ShellStatus`/`ShellStop` MCP tools use. All remaining
arguments after `run` are joined with spaces to form the command line (quote
it yourself if it needs shell-special characters preserved a particular way).

## `config edit` / `config path`

```bash
muxsh config edit                 # opens settings.json in an editor pane
muxsh config path                 # prints $AGENTMUX_DATA_DIR (default)
muxsh config path config          # prints $AGENTMUX_CONFIG_DIR
muxsh config path logs            # prints $AGENTMUX_LOG_DIR
muxsh config path shared          # prints $AGENTMUX_SHARED_DIR
```

`edit` is a thin wrapper over `open` pointed at `$AGENTMUX_CONFIG_DIR/settings.json`
— the terminal equivalent of the Settings pane's own "open settings.json"
footer button. `path` makes no network call at all; it just echoes an
already-injected environment variable.

## `agent list` / `agent send`

```bash
muxsh agent list
muxsh agent list --json
muxsh agent send Scouto "please rebase your PR branch"
```

`list` wraps `GET /agentmux/discovery`; `send` wraps
`POST /agentmux/reactive/inject` — the terminal-side counterpart to an
agent's own `SendMessage` MCP tool. All arguments after the agent name are
joined with spaces to form the message.

To *launch* an agent into a pane, use `muxopen <agent>` (a separate,
already-shipped tool) — not duplicated here as `muxsh agent open`.

## What's not here, and why

Two commands from the original design turned out not to be buildable as
scoped, caught by verifying the actual routes against source before writing
code (not assumed from the docs) — see
`docs/specs/SPEC_MUXSH_FULL_COLLECTION_2026_09_16.md` §5.5/§5.6 for the full
account:

- **`pane close`** — the real route (`ClosePane`) requires an HMAC signature
  from the calling *agent's own* `AGENTMUX_JEKT_KEY`, a credential only
  injected into `agentmux-mcp`'s spawn environment for a specific registered
  agent — never into a terminal pane's environment. `muxsh` structurally
  cannot produce a valid signature for an identity it isn't. The unsigned
  mechanism the frontend itself uses for "click X on a tab" (`DeleteBlock`,
  a WebSocket RPC) has no REST route yet.
- **`secret list`** — the real route (`IdentityAccounts`) is scoped to a
  specific registered *agent*, not to "whoever is running this terminal."
  There's no coherent default answer to "which agent's accounts" from a bare
  shell.

Same single-instance scope as the rest of this tool family: `muxsh` operates
on the instance the calling pane belongs to. Cross-instance operation is not
implemented here.

Implementation: `agentmux-srv/src/backend/shellintegration/muxsh.mjs` (core),
`lib/muxclient.mjs` (shared auth/fetch plumbing), and the per-shell `muxsh`
delegators in the same directory. Route field names are checked against
`docs/specs/app-api-manifest.json` from both the Rust and Node sides — see
that file and `SPEC_MUXSH_FULL_COLLECTION_2026_09_16.md` §2.8.
