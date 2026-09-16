# SPEC: The full `muxsh` collection — a consistent, wsh-familiar CLI over the Agent App API

**Author:** Vmer
**Date:** 2026-09-16
**Status:** active — Phases 1, 2a, and 2b are implemented (Phase 2b's real scope corrected during implementation, see §5.5/§5.6 — two commands originally planned there turned out not to be buildable as designed). Phase 3 not started.
**Related:** `docs/specs/SPEC_MUXSH_CLI_2026_09_16.md` (Phase 1 — shipped, PR #3255, `muxsh open`/`muxsh web`), `docs/reports/REPORT_WSH_STYLE_CLI_FOR_AGENT_APP_API_2026_09_16.md` (the research this builds on), `docs/specs/archive/SPEC_RETIRE_WSH_2026_04_12.md` (why AgentMux doesn't import `wsh`'s literal command set), `docs/reports/REPORT_AGENT_OPEN_API_GAP_2026_09_06.md` (`muxopen`), `agentmux-docs/.../internals/agent-app-api.md`

---

## 0. TL;DR

The previous spec deliberately shipped a narrow first cut (`open`/`web`) and left family-consolidation as an open question. This spec answers it: **`muxsh` becomes the unified, noun-verb dispatcher for AgentMux's Agent App API** — the collection old Wave Terminal `wsh` users would recognize the shape of, without importing capabilities AgentMux doesn't have (SSH/WSL, OSC hijacking, a bolt-on AI query).

Three things make this different from just "add more subcommands":

1. **A house style, defined once, applied to every subcommand.** This is what "consistent API" actually means in practice — one grammar, one set of global flags, one output-mode convention, one exit-code contract, one auth-error message shape. §2 is the part of this spec every future `muxsh` subcommand must be reviewed against.
2. **DRY bindings, not a fourth hand-written copy of every route's field names.** Every route already has two independent Rust implementations (the REST handler and the MCP tool binding); Phase 1 added a third by hand in Node. §2.8 proposes a shared Node client library (removes duplication *within* the CLI family) plus a checked-in manifest + contract tests on both the Rust and Node sides (keeps the CLI honest against the real Rust structs across the language boundary) — done *before* scaling to six more subcommands, not after.
3. **An honest per-command mapping**, not a wishlist. Every one of `wsh`'s ~20 subcommands gets a real verdict: *maps cleanly onto an existing App API route (build now)*, *maps onto a capability that needs backend work first (build later, and say what)*, or *doesn't apply to AgentMux's product at all (don't build, and say why — same rigor the 2026-04-12 retirement spec applied)*.

Net result, by phase:

| Phase | What | Backend work needed? |
|---|---|---|
| **1 — shipped** | `muxsh open`, `muxsh web` | No (PR #3255) |
| **2a — shipped** | `lib/muxclient.mjs`; the App API manifest + Rust/Node contract tests, covering `pane/open` | No — refactor + tests only, §2.8 (PR #3263) |
| **2b — shipped** | `muxsh view`, `muxsh edit`, `muxsh pane list`, `muxsh run`, `muxsh config edit/path`, `muxsh agent list/send` | No — every one is a thin wrapper over an already-existing, verified-unsigned route, built DRY on top of 2a. Manifest extended to cover `shell.{create,status,stop}` and `agent.send` too. |
| **3 — needs backend work first** | `muxsh getmeta`/`setmeta`, `muxsh termscrollback`, `muxsh notify`, `muxsh pane close` (§5.5 — real route exists but is agent-signed, not `X-AuthKey`-gated like the rest; needs a new unsigned self-close route) | Yes — see §5 for exactly what |
| **Not building** | `ssh`/`wsl`/`conn`, `file`, `launch`, `ai`, `setbg`/`badge`, `setconfig`, `secret list` (§5.6 — not a missing route, a conceptual mismatch: identity accounts are agent-scoped, and a terminal pane isn't an agent) | N/A — structurally doesn't map, see §6 |

---

## 1. Why this is different from reviving `wsh`

The 2026-04-12 retirement spec's core finding still stands and this spec doesn't relitigate it: importing `wsh`'s ~400 lines of Go, its OSC-escape-code protocol, its SSH/WSL remote-deployment machinery, and its token-exchange auth model would be reviving dead weight, because none of that machinery maps to AgentMux's actual product.

What *is* being revived is narrower and more honest: **`wsh`'s command *vocabulary and grammar* — not its implementation — as a design reference for AgentMux's own, independently-justified CLI surface**, built entirely on infrastructure that already exists (the Agent App API, the `muxlog`/`muxspect`/`muxopen`/`muxsh` deployment pattern). Every command below is justified on its own AgentMux-specific merits in §3/§4, not on "`wsh` had one."

---

## 2. The `muxsh` house style (read this before adding any subcommand)

This is the actual mechanism for "old wsh users feel at home with a consistent API." A user who learns `muxsh pane open` should be able to *guess* the shape of `muxsh agent send` correctly, without reading its help text first.

### 2.1 Grammar: `muxsh <noun> <verb> [args] [flags]`

`wsh` itself was mostly flat (`wsh view`, `wsh web`, `wsh file cp`) because Wave Terminal has one real resource type — the block. AgentMux's domain has several distinct resource types (panes, agents, the work queue, config, secrets) that already have separate MCP tool families (`pane.*` vs `agent.*` vs `identity.*` vs `Work*`) — collapsing them into flat verbs the way `wsh file cp/mv/rm` did would create naming collisions almost immediately (`muxopen`'s own verb is "open an **agent**"; this spec's Phase 1 verb "open" already means "open a **pane/file**" — those cannot both be bare `muxsh open` in one dispatcher). Noun-verb avoids this permanently, and mirrors the App API's own namespacing (`agent.*`, `pane.*`, `identity.*`) that a reader of `agent-app-api.md` already has to learn anyway — the CLI grammar and the API grammar teach each other.

Nouns, fixed set for this spec (extend deliberately, not ad hoc):

| Noun | Backs |
|---|---|
| `pane` | `pane.*` / `POST /api/v1/pane/*` |
| `agent` | `agent.*` / `/api/v1/agent/*`, `/agentmux/discovery`, `/agentmux/reactive/inject` |
| `run` | *(verb-only, no noun — see §4.3)* `/api/v1/shell/*` |
| `config` | local env-var/file paths, no network call |
| `secret` | `identity.*` — **not shipped this pass, see §5.6**: agent-scoped, not terminal-scoped; kept in this table as the noun a future `--agent <name>` form would use, not a promise of `muxsh secret list` as originally drafted |

**Backward-compatible exception, deliberate, not an inconsistency:** `muxsh open <file>` and `muxsh web <url>` (Phase 1, already shipped) stay as top-level shortcuts for `muxsh pane open`/`muxsh pane web` — they're the two most `wsh`-iconic gestures (`wsh view`/`wsh editor`/`wsh web` — "type a command, a UI block appears," the exact marketing gesture cited in the research report §1.1) and they're already in production use. Document them explicitly as aliases, not as evidence the grammar is inconsistent.

### 2.2 Global flags — every subcommand that opens a pane supports the same set, spelled the same way

Already established by Phase 1 (`muxsh open`/`web`), now made a hard requirement for every future pane-opening subcommand:

| Flag | Meaning |
|---|---|
| `--title <t>` | pane/tab title |
| `--split <right\|left\|down\|up>` | placement relative to the calling pane, defaults to `right` when `$AGENTMUX_BLOCKID` is known (see §2.5 — this exact default-and-guard pattern is now the template, not a one-off) |
| `--floating` | open in a floating window instead of a docked split |
| `--no-focus` | open without focusing the new pane |

A subcommand that doesn't support one of these (e.g. `--collapse-tree` is editor-view-only) must say so in its own `--help`, not silently ignore the flag — this was ReAgent's real finding against Phase 1's first draft (PR #3255) and is now a standing rule, not a one-time fix.

### 2.3 Output mode: `--json` everywhere that returns structured data

`muxspect` already established this (`list`/`describe`/`dock --json`). Every new read-oriented `muxsh` subcommand (`agent list`, `pane list`, `secret list`) must support `--json` for scripting from day one — it's cheaper to build in from the start than to retrofit once a script depends on the human-readable format.

### 2.4 Exit codes — fixed across the whole collection

| Code | Meaning |
|---|---|
| `0` | success |
| `1` | usage error or missing `$AGENTMUX_LOCAL_URL`/`$AGENTMUX_AUTH_KEY` |
| `2` | the server rejected the request, or is unreachable |

Already the contract for `muxopen`/`muxsh` Phase 1; this spec just makes it explicit as a rule for every future addition rather than something each author has to independently notice and copy.

### 2.5 The "only send what you can back up" rule

Phase 1's real bug (ReAgent, PR #3255): `--split <dir>` was sent to the server without the `split_reference_block_id` that makes it meaningful, so it silently did nothing. The general rule this generalizes to, for every future subcommand: **if a flag's effect depends on a piece of context (a block id, a tab id, an agent id) that isn't guaranteed to be present, read that context from the environment/a prior call yourself — don't forward a flag whose server-side effect is conditional on data you didn't send.** Every `muxsh` core reads `$AGENTMUX_BLOCKID`/`$AGENTMUX_TABID` from its own process env (already injected into every terminal pane, `blockcontroller/shell/lifecycle.rs`) for exactly this reason, the same source `OpenEditor`'s MCP tool already uses.

### 2.6 Auth and deployment — unchanged, restated as a hard requirement

Every subcommand: thin Node core in `agentmux-srv/src/backend/shellintegration/`, deployed to `~/.agentmux/shell/`, shell-function delegator in all four shells, `$AGENTMUX_LOCAL_URL` + `$AGENTMUX_AUTH_KEY` from the pane env, **no MCP, no new transport, no new auth scheme** — per the original research report §3, this is the one thing that must never vary between subcommands.

### 2.7 Namespacing inside the binary: subcommand files, one dispatcher

Implementation note so the "one binary" goal doesn't become one giant file: `muxsh.mjs` becomes a thin dispatcher (`muxsh pane ...` → import and call `./muxsh/pane.mjs`'s handler) over per-noun modules, each independently unit-tested the way `open`/`web` already are. This is a refactor of the *existing* Phase-1 file as part of Phase 2's first PR, not a rewrite of its logic.

### 2.8 DRY: one source of truth for bindings, not a fourth hand-written copy per route

**The problem, stated precisely.** Every App API route Phase 1 touched already has *two* independent, hand-maintained representations before this spec adds a third: the Rust REST handler + its `rpc_types::CommandXxxData`/`XxxResult` struct (the actual source of truth), and `agentmux-mcp`'s Rust `TOOLS` JSON-schema entry + `match` arm (`SPEC_AGENT_APP_API_MCP_BINDINGS_2026_06_28.md` §3 — "each tool is (a) a JSON-schema entry... and (b) a match arm that builds a request"). Phase 1's `muxsh.mjs` added a *third*, hand-written in Node, with its own independent knowledge of field names (`view`, `split_reference_block_id`, `tree_expanded`, ...). Scaling to six more subcommands (§4) without addressing this means six more independently-typed copies of field names that only stay correct by human review — exactly the failure mode Phase 1 already hit once (ReAgent, PR #3255: a field silently not sent). This gets worse, not better, as the collection grows, which is the whole reason this section exists before §4's build list, not after.

**Cross-language constraint that has to be designed around, not wished away:** `agentmux-mcp`/`agentmux-srv` are Rust; every `muxsh`/`muxlog`/`muxspect`/`muxopen` core is Node. These cannot literally `import` the same compiled module. "Same bindings" therefore has to mean *generated from, or contract-tested against, one checked-in source of truth* — not "one file both languages load directly." (Precedent for taking generation seriously here already exists in this codebase's own lineage: `SPEC_RENAME_WSH_TO_RPC_2026_04_17.md` describes the frontend's `rpc-api.ts` as "generated-style" typed wrappers over the Go/Rust RPC types — this spec proposes the same discipline for the CLI/MCP boundary, not a novel pattern for the repo.)

**Two tiers, deliberately separated because they have very different cost/risk:**

**Tier 1 — within the Node CLI family itself. Cheap, no new infrastructure, do this first.** `muxlog.mjs`/`muxspect.mjs`/`muxopen.mjs`/`muxsh.mjs` each independently re-implement the same ~20 lines today: read `$AGENTMUX_LOCAL_URL`/`$AGENTMUX_AUTH_KEY`, validate they're set, the `fetch` + `X-AuthKey` header + JSON-parse-with-fallback + non-`ok` error rendering, the `fail()`/exit-code convention (§2.4). Extract this once into `agentmux-srv/src/backend/shellintegration/lib/muxclient.mjs`, a small shared module every core imports. This is a pure refactor of already-shipped code (no behavior change, covered by each core's existing tests) and should land as **Phase 2's first PR, before any of the six new subcommands in §4** — so every new subcommand is built DRY from day one instead of copying the boilerplate a fifth and sixth time.

**Tier 2 — across the Rust/Node boundary. The real ask, and the harder one; propose a contract, not a rewrite.** Rather than generating Node code from Rust (a bigger investment than this CLI's current scale justifies, and a bigger single PR to get right), propose a **checked-in JSON manifest + two contract tests**, one per language:

1. A manifest file (e.g. `docs/specs/app-api-manifest.json`, or `agentmux-common/app-api-manifest.json` if it should ship in a crate other tooling can read) listing, for every route this spec's subcommands touch: the REST path, method, request field names, and response field names — hand-authored once per route, from the real `rpc_types` structs, not invented.
2. A Rust test (alongside the existing `rpc_types`/`app_api` tests) that asserts each manifest entry's field names match the real `CommandXxxData`/`XxxResult` struct's `serde` field names — catching the case where a Rust field is renamed and the manifest wasn't updated.
3. A Node test (`muxsh.contract.test.mjs`, same `vitest` setup as every other `muxsh` test) that asserts each subcommand's `buildRequestBody()` only emits field names present in the manifest for that route — catching the case where `muxsh` drifts from what the manifest (and therefore the real Rust struct) says is valid.

Neither test can literally prevent both sides from being wrong in the *same* way, but both together mean a route rename, a field rename, or a new required field shows up as a **CI failure in both the Rust and the Node test suite**, not a silent runtime mismatch discovered by a user (or by ReAgent, after the fact, the way Phase 1's bug actually was found). This is deliberately positioned as upgradeable, not a dead end: if the collection keeps growing past what §4/§5 already cover, the same manifest is exactly the artifact a future real code-generator (Tier 2.5, not proposed here) would consume — starting with the manifest now doesn't foreclose that, it's a prerequisite for it.

**Scope for this spec:** stand up the Tier 2 manifest + both contract tests covering *only* the routes Phase 1 already shipped (`pane/open`) as part of the same foundational PR that does the Tier 1 extraction — prove the mechanism on one route before scaling it to the six new ones in §4. Extending the manifest to cover each new route as it's built (§4.1–4.7) then becomes a small, mechanical addition to each subcommand's own PR, not a separate effort.

---

## 3. Full `wsh` inventory → AgentMux mapping

Every `wsh` subcommand (from `docs.waveterm.dev/wsh-reference`, cross-checked against `cmd/wsh/cmd/*.go` in the cloned `wavetermdev/waveterm` source per the original research report §1.1), with an honest verdict.

| `wsh` command | AgentMux equivalent route | Verdict |
|---|---|---|
| `view` | `POST /api/v1/pane/open` (view auto-detected) | **Build, Phase 2** — new: `muxsh view` (§4.1) |
| `edit` / `editor` | `POST /api/v1/pane/open` (`view=editor`) | **Build, Phase 2** — `muxsh edit` (§4.1); `muxsh open` already covers this since Phase 1 |
| `web` | `POST /api/v1/pane/open` (`view=browser`) | **Shipped, Phase 1** |
| `launch` | *(OS-level open-with-default-app)* | **Not building** — §6.3 |
| `deleteblock` | `POST /api/v1/agent/pane/close` (backs `ClosePane`, `server/ui_handlers.rs`) — **verified at implementation time to require an agent-signed `UiAutomationAuth`, not plain `X-AuthKey`** | **Phase 3** — §5.5, not Phase 2 as originally scoped; the real unsigned mechanism (`DeleteBlock` WS RPC) has no REST route yet |
| `blocks` | `GET /api/v1/tabs?block_id=<id>` (backs `Layout("tabs")`) | **Build, Phase 2** — `muxsh pane list` (§4.2) |
| `getmeta` / `setmeta` | `blockfile:read_state`/`write_state` — **WebSocket RPC only today, no REST route** | **Phase 3** — needs a new REST route first, §5.1 |
| `run` | `POST /api/v1/shell/create` + `/status` + `/stop` (backs `Shell`/`ShellStatus`/`ShellStop`) | **Build, Phase 2** — `muxsh run` (§4.3) |
| `ai` | *(no one-shot-query primitive; AgentMux's whole product is persistent agents)* | **Not building** — §6.1 |
| `notify` | *unverified — a `PushNotification` MCP tool name exists but this research pass did not confirm its backing route or whether it does OS-level desktop notification* | **Phase 3, pending investigation**, §5.3 |
| `editconfig` | *(no route — opens a local file)* | **Build, Phase 2** — `muxsh config edit`, thin alias over `muxsh open` (§4.4) |
| `setconfig` | *(no equivalent)* | **Not building** — §6.4 |
| `setbg` | *(no such feature in AgentMux's block model)* | **Not building** — §6.2 |
| `badge` | *(no such feature)* | **Not building** — §6.2 |
| `wavepath` | *(env vars already injected — `AGENTMUX_DATA_DIR`, `AGENTMUX_CONFIG_DIR`, `AGENTMUX_LOG_DIR`, `AGENTMUX_SHARED_DIR`, confirmed present in every pane's env this session)* | **Build, Phase 2** — `muxsh config path` (§4.4) |
| `getvar` / `setvar` | *(no generic workspace-scoped KV primitive exists)* | **Phase 3, lowest priority**, §5.2 |
| `termscrollback` | *unclear whether a plain terminal pane's scrollback is persisted/reachable the way an agent block's `agent.output` is* | **Phase 3, pending investigation**, §5.4 |
| `secret` | `GET /api/v1/agent/identity/accounts?agent_id=<slug>` (backs `IdentityAccounts`) — **verified at implementation time to be agent-scoped, not caller-scoped**: `agent_id` comes from `$AGENTMUX_AGENT_ID`, injected only into agent spawn env, never a terminal pane's | **Not building this pass** — §5.6, a conceptual mismatch, not a missing route |
| `ssh` / `wsl` / `conn` | *(no SSH/WSL pane type)* | **Not building** — §6.5, same finding as the 2026-04-12 retirement, still true |
| `file` (cat/write/append/rm/info/cp/mv/ls) | *(local shell already has this; `muxsh` never targets a remote host)* | **Not building** — §6.6 |

---

## 4. Phase 2b — build now, zero backend work (each is a thin wrapper over an already-existing, already-shipped route)

Each of these lands on top of Phase 2a (§2.8): every subcommand below imports the shared `lib/muxclient.mjs` from day one (no new copy of the auth/fetch/error boilerplate) and adds its route to the App API manifest with the matching Rust-side field-name assertion, rather than being written and tested in isolation the way Phase 1 was.

### 4.1 `muxsh view <path-or-url>` / `muxsh edit <file>`

`view` is a smart dispatcher matching `wsh view`'s own "just figure out the right block" behavior: a URL (`^\w+://`) → `view=browser`; a media extension (`.png/.jpg/.gif/.mp4/.pdf/...`) → `view=media`; anything else → `view=editor`. `edit` always forces `view=editor` regardless of extension — the explicit form for when you specifically want the editor, not the smart guess (matches `wsh`'s own `view`-vs-`editor` split). Both take the full §2.2 flag set.

### 4.2 `muxsh pane list`

**Correction, at implementation time:** this section originally also scoped `muxsh pane close [block_id]` here as zero-backend-work. That was wrong, caught by re-verifying the actual route against source before writing code (same discipline as everything else in this spec) — see §5.5, which supersedes the `close` half of this section. `list` itself is unaffected and ships as originally planned.

`list` prints the calling pane's workspace's tabs/panes (`GET /api/v1/tabs?block_id=<id>`, using `$AGENTMUX_BLOCKID`), `--json` for scripting.

### 4.3 `muxsh run <cmd> [-- <args>]`

Thin wrapper over `Shell`/`ShellStatus`/`ShellStop` (`/api/v1/shell/*`). No noun prefix (`run`, not `shell run`) — deliberate exception to §2.1's grammar, because `wsh run` is one of the two or three most-cited `wsh` gestures and `muxsh shell run` reads worse for no real disambiguation benefit (there's no other "shell"-named noun in this collection to collide with). Returns the `shell_id`; `muxsh run --status <id>` / `muxsh run --stop <id>` cover the rest of the lifecycle without inventing a second noun.

### 4.4 `muxsh config edit` / `muxsh config path [data|config|logs|shared]`

`edit` opens `$AGENTMUX_CONFIG_DIR/settings.json` — via `muxsh open` internally, not a new route — matching AgentMux's existing "Settings pane has a footer button that opens the raw settings.json in the user's default editor" convention (`CLAUDE.md`'s Settings row), just reachable from a terminal now. `path` prints the requested directory (default `data`) by echoing the already-injected env var (`AGENTMUX_DATA_DIR`/`AGENTMUX_CONFIG_DIR`/`AGENTMUX_LOG_DIR`/`AGENTMUX_SHARED_DIR` — all confirmed present in a real pane's environment this session) — genuinely zero-risk, no network call at all, the CLI equivalent of `wsh wavepath`.

### 4.5 `muxsh agent list` / `muxsh agent send <name> <message>`

`list` wraps `GET /agentmux/discovery` (backs `DiscoverAgents`), `--json` supported. `send` wraps `POST /agentmux/reactive/inject` (backs `SendMessage`) — the terminal-side counterpart to an agent's own `SendMessage` MCP tool, useful for scripts/CI hooks that want to poke a running agent without themselves being one.

### 4.6 `muxsh agent open <name>` stays `muxopen`

Not duplicated under the new grammar. `muxopen` is already shipped, already has muscle memory, and its own name (a verb, "open [an agent]") predates this spec's noun-verb convention. Document it in `muxsh --help`'s "see also" rather than reimplement it as `muxsh agent open` — one implementation, two ways to typo-check yourself.

### 4.7 `muxsh secret list` — REMOVED, see §5.6

This section's own text flagged its route as "not independently re-verified... at implementation time." It's now been checked, and the finding is a real, structural blocker, not a missing-route gap — see §5.6. Not shipped in this pass.

---

## 5. Phase 3 — real gaps, need backend work before a CLI can be built

### 5.1 `getmeta`/`setmeta`

`blockfile:read_state`/`write_state` exist and work, but only over the WebSocket JSON-RPC transport — no `/api/v1/*` REST route exists, unlike the read-only namespaces (`memory.*`, `mcp.*`, etc.) that already got one in `SPEC_AGENT_APP_API_MCP_BINDINGS_2026_06_28.md`. Needs: a REST route (`/api/v1/agent/blockfile/{read,write}_state` or similar, following that spec's own naming convention), server-side identity/block-scope stamping the same way §5 of that spec did for `memory.*`. Real work, not a CLI-only change — scope it as its own follow-up spec when picked up, don't fold it into a CLI PR.

### 5.2 `getvar`/`setvar`

No generic, workspace-scoped key-value primitive exists in AgentMux today — `memory.*` is agent-scoped (per-agent files), `blockfile:*` is pane-scoped. A faithful `wsh getvar`/`setvar` port would need a genuinely new storage primitive, which is real feature design, not a CLI wrapper. Lowest priority of the Phase 3 items — defer until a concrete use case justifies the new primitive, rather than build storage speculatively.

### 5.3 `notify`

A `PushNotification` MCP tool name was observed to exist in this session's tool listing, but this research pass did not confirm what it actually does (OS toast notification? in-app banner? something else?) or its REST backing. **Before scoping a `muxsh notify`, check `PushNotification`'s real implementation** — if it's already a working, REST-reachable OS-notification primitive, this could move to Phase 2 trivially; if not, it needs the same treatment as §5.1.

### 5.4 `termscrollback`

`agent.output`/`blockfile:read_range` give paginated persisted output for an **agent** block. Whether a plain **terminal** pane's scrollback is captured/persisted the same way is not confirmed by this research pass — needs a source check (`blockcontroller` terminal-pane path) before this can be scoped as "build" or "not applicable."

### 5.5 `pane close` — moved here from §4.2, real blocker found at implementation time

`ClosePane` (`POST /api/v1/agent/pane/close`, `server/ui_handlers.rs`) is **not** a plain `X-AuthKey`-gated route like every other one in §4 — its request type (`ClosePaneRequest`, `agentmux-common/src/api_types.rs`) `#[serde(flatten)]`s a `UiAutomationAuth { agent_id, ts_secs, sig }`: an HMAC-SHA256 signature computed with the *calling agent's own* `AGENTMUX_JEKT_KEY`. That key is injected only into `agentmux-mcp`'s spawn environment for a specific registered agent (the same per-agent signing key jekt sender-authentication uses) — a terminal pane's environment (`$AGENTMUX_LOCAL_URL`/`$AGENTMUX_AUTH_KEY`/`$AGENTMUX_BLOCKID`/`$AGENTMUX_TABID`) never includes it, structurally, by the same design that makes jekt sender spoofing impossible. `muxsh` cannot produce a valid signature for an identity it isn't.

This is deliberately **not** treated as "needs a REST route" the way §5.1 is — a REST route already exists. The actual gap is a WS-only, *unsigned* close path: `DeleteBlock` is a real WebSocket RPC command (`backend/rpc_types/block.rs`, `backend/service.rs`) the frontend itself uses for the ordinary "click X on a tab" case, which — being the frontend's own already-authenticated session, not a claim to be a specific agent — needs no `UiAutomationAuth` signature at all. There is currently no REST route exposing `DeleteBlock`'s simpler, unsigned contract.

Two real paths forward, neither a CLI-only change:
- Add a REST route over `DeleteBlock` for self-close only (defaulting the target to the caller's own `$AGENTMUX_BLOCKID`, no agent-identity claim involved, same trust tier as `pane.open`) — the natural analogue to how `ClosePane` already documents "omitted `block_id`" as the self-only case, just without requiring a signature to prove what's already true (a terminal closing its own pane needs no proof of identity to close a pane it's already running in).
- Or accept `ClosePane`'s existing signing requirement and give `muxsh` a way to hold a real per-agent jekt key — which would mean `muxsh` impersonating an agent identity, a materially bigger and more sensitive change than anything else in this spec, and not recommended without a much stronger justification than CLI convenience.

Recommend the first option if this is picked up. Not scoped further here — real backend design work, same as §5.1.

### 5.6 `secret list` — moved here from §4.7, real blocker found at implementation time

The route backing `IdentityAccounts` (`agentmux-mcp/src/main.rs`) is `GET /api/v1/agent/identity/accounts?agent_id=<slug>` — and that `agent_id` isn't optional plumbing, it's the entire point of the route: identity accounts are linked *to a specific registered agent definition*, not to "whoever is currently typing in a pane." The MCP tool resolves it via `agent_slug()`, sourced from `$AGENTMUX_AGENT_ID` — injected into an **agent's** spawn environment, not a terminal pane's (terminal panes get `$AGENTMUX_BLOCKID`/`$AGENTMUX_TABID`, never `$AGENTMUX_AGENT_ID` — confirmed by grepping every existing `muxsh`/`muxspect`/`muxopen` core, none of which ever reference it).

This is a **conceptual mismatch, not a missing-capability gap** — closer to §6's "doesn't apply" category than a Phase-3 "needs backend work" item, which is why it's listed here rather than promised as a future build. A terminal pane isn't an agent and has no identity-account bindings of its own to list; "which agent's accounts?" has no non-arbitrary answer from a bare shell. If this capability is wanted from a terminal, the honest shape is different from what §4.7 originally proposed — e.g. `muxsh secret list --agent <name>`, explicitly listing a *named* agent's accounts rather than an implicit "mine" — which is a different, smaller design than what was speculatively scoped here. Not built in this pass; revisit only if a concrete cross-agent use case shows up.

---

## 6. Explicitly not building, and why (same rigor as the 2026-04-12 retirement, not a lesser standard for new additions)

### 6.1 `ai`

`wsh ai` is a one-off LLM query bolted onto a product whose center of gravity is the terminal. AgentMux's center of gravity is the *agent* — a one-shot, contextless query contradicts the product's own thesis (the original retirement spec's exact reasoning, restated for a new command rather than an old one). The honest equivalent, if the need is real, is `muxsh agent send` to an agent that already has context — not a new stateless-query endpoint.

### 6.2 `setbg` / `badge`

No per-block background or badge/indicator customization exists anywhere in AgentMux's block model (`widgets.json`, the pane-types table, nothing). This is a missing *product feature*, not a missing CLI wrapper — out of scope for a CLI spec. Would need its own design spec if ever pursued.

### 6.3 `launch`

OS-level "open with the system default handler" is exactly what `open` (macOS)/`start` (Windows)/`xdg-open` (Linux) already do, on every machine, today. A `muxsh` wrapper would add a dependency for zero capability gain.

### 6.4 `setconfig`

Single-key config mutation from the CLI. `muxsh config edit` (opens the whole file) already covers the real need at far lower design cost (no key-path/type-validation surface to build and maintain); revisit only if a specific automation need for single-key sets shows up.

### 6.5 `ssh` / `wsl` / `conn`

AgentMux has no SSH/WSL pane type and no remote-connection story — confirmed unchanged since the 2026-04-12 retirement (nothing in this session's research found evidence otherwise). This is the one piece of `wsh`'s surface that's architectural, not incidental: `wsh`'s auth model (token exchange, deployable to a remote host) exists specifically to serve this use case, and `muxsh`'s simpler local-env-var auth model (§2.6) is sufficient *because* this doesn't apply. Revisiting this is a prerequisite-question for a totally different, much larger feature (does AgentMux want remote panes at all), not a CLI addition.

### 6.6 `file`

`wsh file` (cat/write/append/rm/info/cp/mv/ls, including cross-host) exists because a Wave Terminal shell might *be* the remote host and `wsh` needed a portable way to touch files regardless of where the shell itself is running. `muxsh` always runs on the same machine as the AgentMux instance it talks to (§2.6) — every one of these operations is already exactly as available as the shell's own `cat`/`cp`/`mv`/`rm`/`ls`, with zero capability gap. Building this would be pure surface-area for redundant functionality — the clearest "doesn't map" case in the whole inventory, structurally identical to why the original retirement killed `wsh`'s SSH-file story.

---

## 7. Open questions for the repo owner

1. **Phase 2b as one PR or several?** Six independent additions (§4.1–4.7); each is small and independently testable, same as Phase 1. Recommend several small PRs over one large one (with Phase 2a, §2.8, landing first as its own PR regardless), for the same reviewability reasons `docs/specs/README.md`'s conventions generally favor — but sequencing is a judgment call.
2. **`muxsh secret list`'s exact REST route** needs confirming against source (the one item in §4 not independently re-verified this pass) before implementation starts.
3. **Priority order within Phase 3** (§5.1–5.4) — none are blocking Phase 2, but whoever scopes them next should decide which (if any) is worth the real backend work first. `getmeta`/`setmeta` seems the most clearly justified (closest `wsh` analogue with a real, if narrow, use case); `getvar`/`setvar` the least (no concrete use case identified yet).
4. **Should `view`'s type-detection list (media extensions) live in `muxsh.mjs` or be fetched from something the frontend already uses** (if AgentMux has a canonical "is this a media file" check somewhere in the frontend/pane-type resolution code) — worth checking before hand-rolling a second list that can drift from the real one.
5. **Where should the App API manifest (§2.8 Tier 2) live**, and does it belong to this CLI effort or is it a more general artifact `agentmux-mcp`'s own maintainers should own regardless of `muxsh`? The manifest's value isn't specific to the CLI — it would catch Rust-struct/MCP-tool drift too, which is a pre-existing risk this spec didn't create. Worth raising as a question rather than assuming `muxsh`'s implementer should also own the manifest's long-term maintenance.
