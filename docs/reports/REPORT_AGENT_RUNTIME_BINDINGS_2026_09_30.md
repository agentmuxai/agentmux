# REPORT: every binding between the runtime menu and what the agent runs

**Date:** 2026-09-30
**Status:** analysis — complete inventory of the model / effort / permission-mode bindings; gap G0 fixed in this change, the rest open (§4)
**Author:** Agento
**Prompted by:** the owner, after an agent resumed into Opus 5.5 while its menu read Sonnet 5.5.
Follows `docs/retro/RETRO_RESUMED_AGENT_SPAWNS_WITHOUT_RUNTIME_FLAGS_2026_09_30.md` (root cause) and
`docs/reports/REPORT_AGENT_RUNTIME_STATE_RECONCILIATION_2026_09_30.md` (the end-state design). Those two say
*why* and *what to build*; this one is the complete list of *where*, so the match can be held hard.

## 1. The invariant

For every agent pane, at every moment, **what the menu shows is what the CLI process was given**. Three
records have to agree:

| Record | Holds | Read by |
|---|---|---|
| `agent:runtime` (block meta) | what the user chose: `{permissionMode, model, effort}` | the Runtime dropup, `/runtime`, `/model`, `/effort`, `/permission-mode` |
| `cmd:args` (block meta) | the CLI argv | every spawn, at spawn time only |
| the running process's argv | what the CLI is actually using | nothing — never observed |

The first two are written by different code at different times. The third is only as right as `cmd:args` was the
moment it spawned. **srv never reads `agent:runtime`**; it treats `cmd:args` as opaque and replays it. So the
frontend is the only thing that puts `--model`, `--effort` or a mode flag into the argv, and every spawn that
does not go through the frontend's rebuild runs on the CLI's own defaults.

## 2. Evidence grades

Each finding below carries one:

- **ran** — reproduced by running the code (`buildRuntimeArgs` over every catalog provider).
- **log** — seen in an srv log of a real agent.
- **read** — from reading the code; not run.
- **unverified** — depends on how a third-party CLI behaves; nobody has run it.

## 3. The bindings

### 3.1 Writers of `agent:runtime`

| # | Writer | Where | Writes |
|---|---|---|---|
| W1 | Launch, `launchAgentDefinition` | `frontend/app/view/agent/agent-model.ts` (`resolveInitialRuntimeConfig`) | `{bypass, <override ‖ provider default ‖ sonnet>, high}` |
| W2 | `applyRuntimeChange` | `runtime-apply.ts` | the merged config; callers are the `/model` `/effort` `/permission-mode` `/bypass` `/plan` commands and the Runtime dropup |
| W3 | Dropup auto-migration | `components/AgentRuntimeDropup.tsx` | replaces a stored model id that left the overlaid list; goes through W2 and so restarts the process (G13) |

Nothing in `crates/` writes or reads `agent:runtime`. No other frontend code writes it. Fork, quick-fork,
continue, reattach, split and "new session" all go through W1.

### 3.2 Writers of `cmd:args`

| # | Writer | Where | Carries `--model`/`--effort`? |
|---|---|---|---|
| A1 | Launch | `agent-model.ts` | **now yes** (was no — G0) |
| A2 | Per-send rebuild | `hooks/useAgentCommands.ts` | yes |
| A3 | Runtime change + `forcerestart` | `runtime-apply.ts` | yes |
| A4 | srv `agent.open` (MCP `OpenAgent`, layouts) | `crates/srv/src/server/app_api/agent_open.rs` | **no** (G3) |
| A5 | `/btw` side-question block | `agent_handlers/side_question.rs` | copies the source pane's (G11) |
| A6 | `open:agent` command | `store/command-registry.ts` | `[]` — a bare picker, not a launch |
| A7 | Legacy `launchAgent()` | `agent-model.ts` | no callers; dead code |

A1, A2 and A3 now call one function, `buildPaneArgs` (`buildRuntimeArgs.ts`): catalog base args for the
controller and mode, `agent:runtime` applied on top, then the agent's `provider_flags`. `--fork-session` is
the one launch-only flag and is appended by the caller.

### 3.3 What displays runtime state, and from where

| Reader | Value comes from |
|---|---|
| Runtime dropup trigger + panel (`AgentRuntimeDropup.tsx`), Claude panes only | `getRuntimeConfig(meta)`: `agent:runtime` with a per-field fallback to `DEFAULT_RUNTIME_CONFIG` (`bypass`/`sonnet`/`high`) |
| `/runtime`, `/model`, `/effort`, `/permission-mode` | the same `getRuntimeConfig` |
| Template-create modal model select | provider catalog, gated by `providerSupportsModelFlag` (Claude, Codex) |
| Context-window meter | `message_start.message.model` from the raw stream — the **only** place a CLI-reported model is read, used to pick a window size, never shown or compared |

There is no status line, header chip, swarm row or stats popover that shows model, effort or mode. **Nothing
shows what the process was actually given.**

### 3.4 What srv does with `cmd:args`

| Path | Reads `cmd:args` | Adds model/effort? |
|---|---|---|
| `agentinput` RPC / `run_agent_turn` (also jekt, cron, work-queue delivery) | fresh from block meta each turn | no |
| Persistent spawn, eager resume, config-restart respawn | block meta at spawn | no |
| `resync_controller` with `forcerestart` | defers to turn end when mid-turn | only if the frontend wrote `cmd:args` first |
| App API `agent.send` | block meta (missing → `[]`) | no; no frontend rebuild on this path |
| Subprocess host turn, container turn | block meta | no |
| Codex app-server (dormant: catalog still says subprocess) | block meta | reads `agent:model`, which nothing writes (G12) |
| ACP (openclaw, pi, copilot) | block meta | no model handling at all |

### 3.5 Spawns the menu does not govern (by design)

| Spawn | Model |
|---|---|
| Ambient helper (titles, activity, continuity summary) — `app_api/session.rs` | hard-coded `claude-haiku-4-5-20251001` |
| Drone workflow Agent block — `agents/runner.rs` | never passes `--model`/`--effort`/mode; CLI default |
| Subagents the CLI spawns | chosen by the CLI per subagent (`subagent:spawned` carries each one's model) |

Listed so nobody expects the strip to govern them.

## 4. Gaps

Severity is about the user being misled or money being spent unseen. "Fix" is a direction, not a commitment.

| # | Gap | Grade | Fix |
|---|---|---|---|
| **G0** | **Launch wrote `agent:runtime` next to a `cmd:args` with no `--model`/`--effort`.** A continuation spawns at launch, before any send, and a running persistent process never re-reads `cmd:args`, so it ran on the CLI default (Opus 5.5 / medium) while the strip read Sonnet / high. | **log** (this agent's own 21:04:03 spawn had neither flag; the flags appear only on spawns after a runtime change) | **Fixed here.** Launch uses `buildPaneArgs`. |
| **G1** | **Permission flag gets the wrong vocabulary.** `buildRuntimeArgs` special-cases only kimi/gemini/qwen (`--yolo`) and codex. Every other provider falls into the Claude branch on **every send**: `muxcode`, `openclaw` (`acp`), `copilot` (`--acp`), `pi` (`--json`) gain `--dangerously-skip-permissions`; `antigravity`'s own `--yolo` is stripped and replaced by it. | **ran** for what is emitted; **unverified** whether each CLI rejects the flag | Decide per CLI; antigravity is a Gemini-style CLI that declares `--yolo` itself. Pinned by a ratchet test. |
| **G2** | **Antigravity lists models and applies none.** The catalog has a model list, `providerSupportsModelFlag` says no, yet `resolveInitialRuntimeConfig` stores `gemini-3.6-flash` and `/model` replies "applies to next turn". The template modal hides the picker; the slash command does not. | **read** + ratchet test | Wire `--model` for it, or make `/model` and the stored default honour the same gate. |
| **G3** | **srv-opened panes have no runtime at all.** `agent_open.rs` writes `cmd:args` (catalog + `provider_flags`) and no `agent:runtime`, no `agent:provider_flags`. A resumable pane is eagerly resumed at open → CLI default. The strip shows the fallback `Bypass · Sonnet · high` regardless. The first UI send then rebuilds `cmd:args` without the define-time `provider_flags` (incl. any `--model`). Same mechanism as G0, other entry point. | **read** (same mechanism as the G0 log) | srv fills `--model`/`--effort` from `agent:runtime` at spawn (retro §5 item 2), and `agent.open` seeds `agent:runtime` and `agent:provider_flags`. |
| **G4** | **Effort is shown for Haiku but not applied** (`--effort` 400s on Haiku 4.5). The guard is an exact match on the alias `haiku`; a concrete Haiku id would still receive it. | **read** | Per-model capability (`SPEC_MODEL_EFFORT_CAPABILITY_VALIDATION_2026_07_02.md`); hide or disable the row. |
| **G5** | **Permission modes are mostly cosmetic on persistent Claude.** `bypass` becomes `--permission-mode default`, and srv's control channel auto-allows every tool except AskUserQuestion, so `bypass`, `default`, `acceptEdits`, `auto` behave the same for tool approval. Only `plan` is enforced. The dropup labels ("Default (prompt all)", "Auto (AI classifier)") promise more. | **read** | Relabel to what happens, or implement the modes in srv. |
| **G6** | **srv-originated turns skip the per-send rebuild.** `agent.send`, jekt and cron use `cmd:args` as it stands. After a runtime change on a subprocess or container pane (which rewrites nothing until the next UI send) they run the old args. | **read** | srv derives the flags at spawn (same fix as G3). |
| **G7** | **A running persistent process ignores later `cmd:args` changes.** The per-send rewrite lands in meta and nothing restarts the process; only `runtime-apply.ts` does. G0's fix closes the launch case, not the general one. | **read** | The reconciler in the reconciliation report §3. |
| **G8** | **`provider_flags` can carry its own `--model`/permission flag.** `agent.define` writes `--model <m>` into `provider_flags`; it is appended *after* the runtime flags, giving two `--model`. If the CLI takes the last one, the strip and `/model` silently lose to the definition. Nothing dedupes. | **read**; last-wins is **unverified** | Strip them from `provider_flags` in `buildPaneArgs`, or surface the definition's model as the starting `agent:runtime`. |
| **G9** | **Model / effort / mode are not remembered across launches.** Fork, continue, reattach, quick-fork and new-session launches pass no model override, so `agent:runtime` resets to the catalog default and the previous pane's choice is lost. (Layout save/restore drops it too.) | **read** | Persist on the agent instance row; seed W1 from it. |
| **G10** | **The effective model is never observed.** srv does not parse `init.model`, `message.model` or `modelUsage`, sends no `get_settings`/`set_model`, and keeps no record of the spawned argv (only a log line). | **read** | Reconciliation report §3–4. |
| **G11** | **`/btw` copies the source pane's `cmd:args`.** That inherits `--model`/`--effort` that may differ from the running process, and on a persistent Claude pane also `--input-format stream-json` / `--permission-prompt-tool stdio` while the turn writes raw text. | **read**; the stream-json part **unverified** | Run the container-style heal over it. |
| **G12** | **Codex app-server reads `agent:model`, which nothing writes.** Dormant (the catalog still says subprocess); the model picker would not reach it if enabled. | **read** | Write it from `agent:runtime` when the controller is enabled. |
| **G13** | **The Dropup silently restarts the agent.** Its auto-migration of a superseded model id calls `applyRuntimeChange` with no user action. | **read** | Migrate the stored value without a restart, or ask. |
| **G14** | **Only Claude panes have a runtime menu.** Codex's model is reachable through `/model`, `/runtime` and the create modal only; `/effort` and `/permission-mode` succeed for providers that ignore them. | **read** | Per-provider capability table drives both the menu and the commands. |
| **G15** | **Environment and settings can override the flag's absence.** srv strips no `ANTHROPIC_MODEL`/`CLAUDE_CODE_EFFORT_LEVEL`, and a `model` in the config dir's or workdir's `settings.json` applies when no `--model` is passed. With the flag present the flag wins. | **read** | Only matters where G0/G3 leave the flag off. |
| **G16** | **Catalog drift.** The Rust `ProviderConfig` has no model list; the live overlay and `providers.models` are Claude-only and fetched with the account-global token, not the agent's bound identity, so labels can differ from what that identity's CLI resolves. `model_catalog.rs` still says the RPC is "not yet implemented"; comments still cite the removed `AgentControlBar`. | **read** | Covered by the catalog specs; comment cleanup is free. |

## 5. What this change adds

**Code**
- `buildPaneArgs` (`buildRuntimeArgs.ts`): the one composition, used by launch, the per-send rebuild and a
  runtime change. Launch now resolves `agent:runtime` *before* building the args and builds both from the same
  value.
- Behaviour change worth knowing: a **continued or resumed** agent now starts on the model its strip shows
  (Sonnet / high by default) instead of the CLI default (Opus / medium). That is the fix, and it changes cost.

**Tests** (`pane-args-parity.test.ts`, 94 tests, all over the live provider catalog)
1. For every provider × host/container: no runtime flag repeats, the rebuild is idempotent, `--fork-session`
   never leaks into a rebuild, codex's stdin marker stays last.
2. Claude: `--model` and `--effort` equal `agent:runtime` for every catalog model × effort × mode; a persistent
   agent is never handed the bypass flag; a fresh launch's args match the runtime it writes to meta.
3. Every provider that lists models either gets `--model` or is in `KNOWN_MODELS_NOT_APPLIED` (G2).
4. Ratchets for G1 (`KNOWN_STRAY_BYPASS_FLAG`, `KNOWN_YOLO_DROPPED`): the test fails when a gap is fixed without
   removing its entry, and when a new one appears without being added.
5. Structural guard: nothing outside `buildRuntimeArgs.ts` may call `buildRuntimeArgs()` or `selectLaunchArgs()`.
   `launchAgentDefinition` itself is too large to run in a unit test, so this is what stops the launch site
   from quietly going back to building its own args. Checked by mutation: reverting the fix at the launch site
   fails it.

## 6. What still needs tests, in order of value

1. **srv eager-resume argv** (`persistent/tests/eager_resume.rs`): with `agent:runtime` set and `cmd:args`
   lacking `--model`/`--effort`, the spawned argv has them. Needs the srv fill-in first (G3, retro §5 item 2);
   then this is the test that holds the whole class shut from the server side. The existing stub-process
   harness there already records argv.
2. **`agent.open` parity** (`agent_open.rs` tests): the pane it creates has `agent:runtime` and a `cmd:args`
   that agree (G3).
3. **Reconciler tests**, as listed in the reconciliation report §4 (fake CLI that ignores `--model`; a CLI that
   never answers `get_settings`; a continuation spawn; a multi-model `modelUsage`; the bypass → default
   exclusion).
4. **One live check per release**: spawn a continued agent, read its argv from the srv spawn log, and ask the
   CLI which model it is. The argv check is the cheap half and could run in the existing smoke job.

## 7. Open verification items

Things this report could not settle from code alone:

- Does the CLI take the **last** of two `--model` flags? (G8)
- Do `muxcode`, `openclaw`, `copilot` and `pi` reject or ignore `--dangerously-skip-permissions`? Does the
  antigravity CLI honour it, or only `--yolo`? (G1)
- Does `/btw` on a persistent Claude pane work at all with the inherited stream-json flags? (G11)
- CLI 2.1.283+ reportedly emits `system`/`init` only after the first user message; confirm before relying on it
  as a readback source (reconciliation report §2.2).
