# Report: A terminal CLI for the Agent App API — what Wave Terminal's `wsh` actually is, why AgentMux already tried and killed this once, and what's changed since

**Date:** 2026-09-16
**Status:** proposed
**Author:** Vmer
**Repo state:** `agentmux` main @ `68ad59a` (v0.56.0); `wavetermdev/waveterm` main cloned fresh into this workspace for direct source inspection (not from memory or the marketing docs alone)

---

## 0. TL;DR

- **Wave Terminal's `wsh`** is a 20+ subcommand Go CLI (`cobra`-based) that talks to the running Wave app over a custom RPC protocol (`wshrpc`, over WebSocket/domain sockets), authenticated by a token exchanged at shell-init time. Its product thesis: the terminal is a bidirectional control surface — shell commands drive UI blocks, and UI state flows back into the shell.
- **AgentMux inherited this CLI literally** (`agentmux-wsh`, forked verbatim from Wave Terminal in the 2026-04-03 rename) and then **retired it entirely on 2026-04-12** (`docs/specs/archive/SPEC_RETIRE_WSH_2026_04_12.md`, shipped) after a full-repo grep found **zero real invocations anywhere** — no docs, no scripts, no agent tools, nothing. The verdict then: none of Wave Terminal's five founding use cases for `wsh` map onto AgentMux's product, because AgentMux's center of gravity is agent panes, not user-driven terminals.
- **That verdict is five months stale in one specific way.** In 2026-04, AgentMux had no Agent App API at all. Today it has a large one — MCP tools, a REST surface, and a WebSocket JSON-RPC layer covering agent lifecycle, panes, sessions, identity, memory, MCP/skill/bundle management, and a shared work queue (`agentmux-docs/src/content/docs/internals/agent-app-api.md`). And, organically, three small Node CLIs have already appeared to expose *pieces* of it to a terminal: `muxlog` (log viewer), `muxspect` (live state introspection), `muxopen` (launch an agent into a pane — added 2026-09-06 specifically because *nothing on the command line could start an agent*, `docs/reports/REPORT_AGENT_OPEN_API_GAP_2026_09_06.md`).
- **So the honest question isn't "should we bring `wsh` back."** It's "AgentMux has been re-deriving `wsh`'s shape one tool at a time (`muxlog`, `muxspect`, `muxopen`) — should that continue ad hoc, or converge on one consistent entry point?" This report argues: **converge**, but keep the proven architecture each of the three already uses (thin Node client, `$AGENTMUX_LOCAL_URL`/`$AGENTMUX_AUTH_KEY` from the pane env, hits the same REST/WS routes the frontend and MCP tools use) rather than importing `wsh`'s Go/cobra/custom-RPC machinery.
- **Should it work through the Agent App API?** Yes, unambiguously, and it already would have no other sane option — see §3.
- **Naming:** the existing family already established a convention (`muxlog`, `muxspect`, `muxopen`, alongside `muxbus`/`muxcode`/`muxqueue` elsewhere in the product). Recommend **`muxctl`** as the collision-checked, convention-consistent name for a unifying entry point — see §5 for why, and why the more obvious `wsh`-lineage name (`muxsh`) and the shorter `amux` are worse choices.

---

## 1. What `wsh` actually is (verified from source, not the marketing docs)

Cloned `wavetermdev/waveterm` fresh (`agentmux/waveterm/`, this report's workspace) rather than relying on recall or the public docs site alone, because the docs site (fetched: `docs.waveterm.dev/wsh-reference`) lists the subcommand surface but not the transport/auth internals.

### 1.1 Command surface (from `docs.waveterm.dev/wsh-reference`, cross-checked against `cmd/wsh/cmd/*.go`)

`view`, `edit`/`editor`, `getmeta`/`setmeta`, `ai`, `editconfig`, `setbg`, `badge`, `run`, `deleteblock`, `ssh`, `wsl`, `web`, `notify`, `conn` (status/reinstall/disconnect/connect/ensure), `setconfig`, `file` (cat/write/append/rm/info/cp/mv/ls — including across SSH-remote hosts), `launch`, `getvar`/`setvar`, `termscrollback`, `wavepath`, `blocks`, `secret`.

### 1.2 Transport and auth (from `cmd/wsh/cmd/wshcmd-root.go`, `wshcmd-token.go`, `pkg/wshrpc/`)

- `wsh` is a `cobra.Command` tree (`Use: "wsh"`) that builds a `wshutil.WshRpc` client and talks a custom RPC protocol, `wshrpc`, implemented over WebSocket (and Unix domain sockets for local connections) — this is Wave's own RPC layer, not a generic REST API.
- Authentication is a **token exchange**, not a static key: `wsh token <token> <shell-type>` (hidden subcommand, called by shell-init) trades a one-time token for an env-var script (`EncodeEnvVarsForShell`) plus an init script, which is what actually wires up the RPC client's credentials for that shell session.
- `wsh` is deployed **to remote hosts** on first SSH/WSL connect (`wsh conn reinstall`/`update`) specifically so a remote pane gets the same rich control surface as a local one — this is the one piece of `wsh`'s design that has no AgentMux analogue at all, because AgentMux has no SSH/WSL pane story (confirmed independently in §2).

### 1.3 The product thesis behind the surface

Wave Terminal's founding pitch (documented in the AgentMux retirement spec, §1, and consistent with what's in `aiprompts/waveai-architecture.md` in the cloned source) is *"shell output flows out as structured data, and shell commands flow in as control operations"* — the terminal itself is the product, and `wsh` is how it talks back to the rest of the app. Every one of the ~20 subcommands is a specific instance of that one idea: `view`/`web`/`launch`/`editor` open UI blocks from a shell command; `setbg`/`badge`/`setmeta` decorate a specific block from the command line; `ssh`/`wsl`/`conn` extend that same capability to remote hosts; `ai` is a one-off LLM query from the shell (Wave Terminal added AI as a bolt-on feature to an already-existing terminal product, not the reverse).

**Source:** [wsh overview](https://docs.waveterm.dev/wsh), [wsh reference](https://docs.waveterm.dev/wsh-reference), [Wave Terminal GitHub](https://github.com/wavetermdev/waveterm).

---

## 2. AgentMux already answered this question once — and the answer needs updating, not reversing

`docs/specs/archive/SPEC_RETIRE_WSH_2026_04_12.md` (Status: SHIPPED) did the actual work already: it mapped all five of Wave Terminal's `wsh` use cases (terminal-as-automation-API, power-user workspace scripting, remote SSH/WSL control, OSC escape-code protocol, poor-man's-extension-API) against AgentMux's product and found **none of them mapped**, backed by a full-repo grep showing zero real invocations of `wsh <subcommand>` anywhere outside the crate's own orphaned test suite. It was deleted: the crate, the packaging step, the CEF-side deploy logic, four shell-integration branches, two dead RPC constants — roughly 400 lines and ~1.19 MiB per portable build. A second, narrower spec the same month (`SPEC_RENAME_WSH_TO_RPC_2026_04_17.md`) separately renamed the *frontend's own* WebSocket RPC client files away from `wsh*` naming, because that layer was never the CLI at all — just internally-named after it.

**That verdict was correct for 2026-04, and the reasoning holds today: none of Wave Terminal's original five use cases have become true of AgentMux since.** AgentMux still has no SSH/WSL pane story, doesn't hijack OSC escape codes, and doesn't need `wsh ai` (the whole product is agents). Reviving `wsh` wholesale would be reviving dead weight for the same reasons it was cut.

**What's actually changed is narrower and specific: AgentMux now has a real, agent-facing App API worth putting a terminal front door on — which didn't exist in April.** The retirement spec's own grep found *zero* agent tools, zero MCP surface, zero App-API-anything in the repo at the time. Today (`agentmux-docs/.../agent-app-api.md`, `docs/specs/SPEC_AGENT_APP_API_MCP_BINDINGS_2026_06_28.md`) there's a substantial, actively-growing API: MCP tools (`WhoAmI`, `Layout`, `SetName`, `OpenEditor`, `Shell*`, `DiscoverAgents`, `SendMessage`, `Work*`, `Loop`/`Cron*`, `Memory*`/`Identity*`), a documented REST surface mapping to most of those, and a much larger WebSocket JSON-RPC catalog underneath (`agent.*`, `pane.*`, `session:*`, `identity.*`, `memory.*`, `mcp.*`, `skill.*`, `bundle.*`, `widget.*`) that the frontend uses but nothing at the terminal reaches directly.

And — independently of this report, before it started — AgentMux has already begun re-deriving `wsh`'s actual shape, piece by piece:

| Tool | What it does | Added |
|---|---|---|
| `muxlog` | Discover/render/follow AgentMux's own logs across every running instance | earlier (exact date not in this report's scope) |
| `muxspect` | Live process/turn-state introspection, work-queue inspection, layout doctor, migration doctor | earlier |
| `muxopen` | Launch an agent into a pane from a terminal, no GUI required | 2026-09-06, `docs/reports/REPORT_AGENT_OPEN_API_GAP_2026_09_06.md` |

`muxopen`'s own source comment states the pattern explicitly: *"until this existed nothing on the command line could START an agent — automation could stop cross-channel but not launch anywhere."* That is, word for word, the same gap-finding logic that would justify the next tool, and the one after that.

---

## 3. Should a CLI like this work through the Agent App API? Yes — and there's really only one architecturally sound way to do it

`agentmux-docs/.../agent-app-api.md` already draws the relevant distinction, and it resolves this question directly:

> "It helps to keep two layers separate: **MCP tools** — the curated, agent-facing verbs that `agentmux-mcp` advertises... **The app-API RPC surface** — the full set of JSON-RPC commands the backend registers over its WebSocket transport... MCP tools are a thin, curated slice of this larger surface."

A terminal CLI invoked by a human typing in a pane (or by a script, or by an agent's own `Bash`/`Shell` tool call) is **not** an MCP client — it has no LLM harness translating tool-call JSON into a stdio MCP session. It's exactly the second category the docs already name: *"scripts or tooling that run with explicit access to the auth key."* `muxlog`, `muxspect`, and `muxopen` already establish the working pattern for this, and it's the same pattern this report recommends extending rather than replacing:

```
node ~/.agentmux/shell/<tool>.mjs <args>
  reads $AGENTMUX_LOCAL_URL + $AGENTMUX_AUTH_KEY from the pane's own environment
  → POST/GET against the documented /api/v1/* (or /agentmux/*) REST routes
  → same app-API handler the frontend and agentmux-mcp ultimately call
```

This is deliberately **not** a new transport, not a new auth scheme, and not MCP. It's the exact "trusted subprocess with explicit access to the auth key" case the docs describe, verified working three times over already (`muxopen.mjs`'s own header comment: *"authenticated exactly the way muxspect and agentmux-mcp already are... No new IPC, no new auth scheme"*). `wsh`'s own token-exchange model is a reasonable design for Wave Terminal's threat model (a CLI that must also work against a **remote** SSH host, where env-var injection at spawn time isn't available) — but AgentMux has no remote-pane story, so the simpler, already-proven "read it from the local pane's own env" model is strictly sufficient and shouldn't be second-guessed into something more complex.

**One real gap exists that none of `muxlog`/`muxspect`/`muxopen` cover, and it's the closest thing to `wsh`'s actual marketing-demo use case:** nothing on the command line can open a **pane** (editor, browser, terminal) the way `OpenEditor`/`pane.open` already do for MCP callers. `wsh view`/`wsh editor`/`wsh web`/`wsh launch` map almost one-to-one onto AgentMux's existing `POST /api/v1/pane/open` route — the REST endpoint and the app-API handler already exist; there is simply no terminal-side client for it today, exactly the same shape of gap `muxopen` closed for `agent.open` ten days before this report.

---

## 4. Recommendation

**Don't import `wsh`.** Its command surface is shaped by a product thesis (terminal-as-control-surface, remote-host parity, OSC hijacking, a bolt-on AI query) that AgentMux's own 2026-04-12 retirement correctly found doesn't apply, and nothing about that finding has changed.

**Do close the specific, real gap: a terminal-side client for `pane.open`/`OpenEditor`**, following the exact `muxopen` pattern (thin `.mjs` core, deployed to `~/.agentmux/shell/`, shell-function delegators in `bash.sh`/`zsh.sh`/`fish.fish`/`pwsh.ps1`, `$AGENTMUX_LOCAL_URL`+`$AGENTMUX_AUTH_KEY` auth, no new transport). That alone would give a human or script in a terminal pane the single most-demoed `wsh` gesture ("open a file/URL as a block right next to me") without reviving anything else.

**Separately — and this is a judgment call for the repo owner, not something this report resolves — decide whether the `muxlog`/`muxspect`/`muxopen`(/`muxpane`?) family should stay as independent single-purpose binaries, or converge under one dispatcher** (`muxctl log ...`, `muxctl spect ...`, `muxctl open ...`, `muxctl pane open ...`) the way `wsh` itself was one binary with subcommands. Arguments either way:

- **Keep them separate (status quo):** each tool already has its own focused `--help`, its own exit-code contract, and its own shell-function name that's presumably muscle-memory for whoever's been using them since. Zero migration cost.
- **Converge under one entry point:** one thing to document, one thing to discover (`muxctl help` lists everything instead of needing to already know three-to-N tool names exist), and it's the shape that scales if the App API gap-filling continues past a fourth or fifth tool. `wsh` itself is evidence this shape works at 20+ subcommands; AgentMux's own list of not-yet-terminal-reachable App API groups (`bundle.*`, `mcp.*`, `skill.*`, `identity.*` beyond what's MCP-bound, `session:*`) is long enough that "one binary, subcommands" may age better than "one binary per verb."

This report doesn't pick between those two — it's a product-shape decision, not a technical one, and the technical foundation (thin REST/WS client, pane-env auth) is identical either way.

---

## 5. Naming — collision research

Checked candidates against live web search (GitHub, npm, Go package index), not assumption. AgentMux already has an internal convention worth respecting: `mux`-prefixed shell tools (`muxlog`, `muxspect`, `muxopen`) plus `muxbus` (the messaging layer) and `muxcode` (the sibling agentic-CLI repo) elsewhere in the product. Any new name should fit that family unless there's a strong reason not to.

| Candidate | Collision finding | Verdict |
|---|---|---|
| `wsh` | **Already taken — literally Wave Terminal's own binary.** Reusing it would be actively misleading (same name, different, incompatible protocol) even setting aside that it's not even AgentMux's own prior name choice to begin with (Wave Terminal owns it upstream). | **Reject.** |
| `amux` | **Bad.** Multiple real, actively-maintained projects already use this exact name in an overlapping problem space: [`weill-labs/amux`](https://github.com/weill-labs/amux) ("Terminal multiplexer with a first-class agent API for shared human+agent workflows" — nearly identical positioning), [`mixpeek/amux`](https://github.com/mixpeek/amux) ("control plane for AI coding agents... parallel Claude Code, Codex, Gemini workers... single Rust binary" — describes almost exactly AgentMux's own category), plus at least two more (`hewigovens/amux`, `mattmorganpdx/amux`). A user searching for this name will find *other people's* AI-agent orchestration tools first. | **Reject — the worst option checked.** |
| `muxsh` | Minor collision: one small, low-traffic GitHub repo ([`PhosCity/muxsh`](https://github.com/PhosCity/muxsh), a bash wrapper around an anime-subtitling tool called SubKt). Not a published package, not on anyone's default PATH, negligible real-world collision risk — but it does mean the name isn't literally unclaimed. Closest in spirit to `wsh`'s own naming (keeps the `sh` suffix), which could read as implying a shell/REPL rather than a command dispatcher (as `wsh` itself was not a shell either, despite the name — a pre-existing, minor naming imprecision, not unique to this candidate). | **Acceptable, not first choice.** |
| `muxctl` | Minor collision: two obscure Go packages (`ngicks/cmdman`'s internal `muxctl` package; `pyrex41/shenmux`'s `muxctl` command) and an unrelated `usbmuxctl` (USB hardware mux control). None are widely installed, none would plausibly be on a target user's PATH. Follows the well-established Unix `-ctl` convention for "this is a control/dispatcher CLI for a running service" (`kubectl`, `systemctl`, `usbmuxctl` itself) — signals the right thing about what the tool does, doesn't imply "shell." | **Recommended.** |

**Recommendation: `muxctl`.** It fits the existing `mux`-prefix family, has the cleanest collision profile of the four real candidates checked, and its `-ctl` suffix correctly signals "control/dispatch CLI for a running AgentMux instance" rather than implying a shell replacement. `muxsh` is a reasonable, low-risk fallback if `-sh` naming continuity with the retired `wsh` lineage is considered more valuable than the `-ctl` convention. `amux` should be avoided outright regardless of which structural recommendation (§4) is chosen — the collision there isn't cosmetic, it's with tools in the *same product category*.

**Sources:**
- [wsh overview | Wave Terminal Documentation](https://docs.waveterm.dev/wsh)
- [wsh reference | Wave Terminal Documentation](https://docs.waveterm.dev/wsh-reference)
- [Wave Terminal — GitHub](https://github.com/wavetermdev/waveterm)
- [GitHub - PhosCity/muxsh](https://github.com/PhosCity/muxsh)
- [GitHub - weill-labs/amux](https://github.com/weill-labs/amux)
- [GitHub - mixpeek/amux](https://github.com/mixpeek/amux)
- [muxctl package — ngicks/cmdman](https://pkg.go.dev/github.com/ngicks/cmdman/pkg/muxctl)

---

## 6. Open questions for the repo owner

1. **Scope of the first cut.** This report recommends closing exactly one gap first (`pane.open`/`OpenEditor` from the terminal, `muxopen`-style) rather than a broad new subcommand surface. Agreed, or is there a different specific gap that matters more right now?
2. **One dispatcher vs. several single-purpose tools** (§4) — no strong technical reason either way; this is about what's easiest to discover and document going forward.
3. **Naming** — `muxctl` vs. `muxsh` vs. something else entirely; `amux` is the one option this report actively argues against.
4. **Is there any interest in the one `wsh` capability AgentMux structurally can't have today** — remote-host (SSH/WSL) deployment of the CLI — as a forward-looking reason to design the auth/transport layer a bit more generally now, or is "AgentMux has no SSH-pane story" (confirmed twice, in the original retirement spec and unchanged since) reason enough to not build for it speculatively?
