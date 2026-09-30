# Retro: a resumed agent spawned without its model, effort and permission flags

**Date:** 2026-09-30 (UTC; the evening of 2026-09-29 Pacific)
**Found by:** the owner, on AgentX's pane in a local v0.58.3 portable build
**Investigated by:** AgentX (narko)
**Status:** retro — root cause identified; fix 1 (launch applies the runtime) implemented, fixes 2–5 open (see §5). Every binding and remaining gap: `docs/reports/REPORT_AGENT_RUNTIME_BINDINGS_2026_09_30.md`

## 1. What happened

AgentX's pane strip read **`Bypass · Sonnet 5.5 · high`**, but the agent was
actually running on **Opus 5.5 at medium effort**. Of the three settings in
the strip, model and effort had not been applied to the running process;
permission mode had (row below) — its own translation from `bypass` to
`default` is by design, not part of this bug.

What the pane showed against what ran:

| Setting | Strip (block meta `agent:runtime`) | Running process (pid 54748) |
|---|---|---|
| Model | `sonnet` ("Sonnet 5.5") | no `--model` → Claude Code 2.1.280's own default, **Opus 5.5** (`claude-opus-5-5`) |
| Effort | `high` | no `--effort` → the CLI default, **medium** (`CLAUDE_EFFORT=medium` in the CLI's tool env) |
| Permission | `bypass` | `--permission-mode default` (by design: a control-protocol agent never gets bypass; srv's auto-allow provides it; `buildRuntimeArgs.ts:127-136`) |

Agents normally run at the pane's default of Sonnet / high. Here the strip still
showed that selection, so nothing on screen suggested the agent was on a
different, costlier model.

## 2. Evidence

The srv log for this instance (`agentmuxsrv-v0.58.3.log.2026-09-30`, block
`2379834a-f9ab-4e6a-9061-07eb71d6cb5a`), in order:

| Time (UTC) | Event |
|---|---|
| 02:53:57.548 | `ControllerResync forcerestart=true`, the launch commit from `launchAgentDefinition` |
| 02:53:57.837 | a second `ControllerResync` (force=false) → "existing controller — skipping spawn" |
| 02:53:57.892 | "prior session … not eager-resuming it … another spawn already in flight". So the first resync's spawn is the one in progress. |
| 02:53:58.247 | resume gate: the conversation belongs to another channel's login (`local-agent1-release-patch-bump-…`), so it is relocated to fork from |
| 02:53:59.259 | **persistent process spawned**, args: `--input-format stream-json --output-format stream-json --verbose --include-partial-messages --permission-prompt-tool stdio --permission-mode default --resume 7b94a635-… --fork-session` |
| 02:54:04.224 | first `AgentInput` (the owner's "u there"). The per-send rebuild now writes `--model sonnet --effort high` into `cmd:args`. The process is already running and never re-reads them. |

The spawned argv is exactly the Claude provider's `persistentLaunchArgs`
(`frontend/app/view/agent/providers/catalog.ts:150`), plus the resume and fork
flags. By the time of the investigation the block's `cmd:args` did contain
`--model sonnet --effort high`, written at the first send, after the spawn.

## 3. Root cause

Three things combine.

1. **The launch path writes `cmd:args` without the runtime flags.**
   `launchAgentDefinition` (`frontend/app/view/agent/agent-model.ts:517-530`)
   builds `cliArgs` from `selectLaunchArgs(provider, mode)` + `provider_flags` +
   `--fork-session`. It never calls `buildRuntimeArgs`, even though a few lines
   later (`:794-822`) it writes `agent:runtime` (the model/effort/mode the strip
   displays) into the same meta commit. So from the moment of launch, the
   block's two records of the runtime config disagree.

2. **The flags are only added when a message is sent.** `useAgentCommands.ts:1526-1534`
   rebuilds `cmd:args` with `buildRuntimeArgs(...)` on every send. For a fresh
   launch this hides bug 1: the controller spawns lazily on the first message,
   so the rebuild lands first and the process gets `--model sonnet --effort high`.
   That's why agents normally come up on Sonnet.

3. **A continuation spawns before any send.** When a launch continues a prior
   session (here, a conversation relocated from another channel and resumed with
   `--fork-session`), the launch's `forcerestart` resync spawns the persistent
   CLI immediately (the "continuation's eager resume" in the `commitLaunch`
   comment). It reads the `cmd:args` from bug 1, without the flags. The per-send
   rebuild then updates `cmd:args`, but a persistent process doesn't re-read
   them. Only `runtime-apply.ts` (a `/model`, `/effort` or `/mode` change)
   respawns with `forcerestart`, and a plain send doesn't.

The result: every continued or resumed launch runs on Claude Code's own
defaults for its whole life, while the strip shows the selected settings. Here
that meant Opus 5.5 at medium effort. That's likely a cost increase as well as
a mismatch, and it happens silently.

## 4. The "Sonnet 5.5" label (a separate, smaller issue)

- The curated catalog labels the `sonnet` row **"Sonnet 5"**
  (`providers/catalog.ts:174`).
- At startup, `providers.models` fetches Anthropic's Models API
  (`agentmux-srv/src/backend/model_catalog.rs`).
- `setProviderModels` (`providers/model-overlay.ts:89-128`) then relabels each
  curated row with the newest model in its family. Anthropic shipped Sonnet 5.5
  on 2026-09-28, so the row reads "Sonnet 5.5".
- The value it passes is still the alias `sonnet`, which the Claude Code CLI
  resolves itself. **Checked after this retro was first written:** once this
  pane's own selection was correctly applied (`--model sonnet --effort xhigh`
  confirmed in the spawned process's command line), the harness's own live
  model self-report read `claude-sonnet-5` — CLI `2.1.280`'s `sonnet` alias
  resolved to Sonnet 5, not Sonnet 5.5.
- So the label advertised a model the pinned CLI didn't select. This is the
  same kind of mismatch the overlay's own comment warns about for concrete-id rows.

We want Sonnet 5.5 as the default. Fixed in
`docs/spec-claude-code-versioning.md`'s `2.1.285` bump: that CLI's embedded
model catalog lists `claude-sonnet-5-5`, and the curated `sonnet` label was
updated to match.

## 5. Fix plan

1. **Apply the runtime config at launch.** *Done:* `launchAgentDefinition` now resolves `agent:runtime` first and builds `cmd:args` with `buildPaneArgs` — the same function the per-send rebuild and `applyRuntimeChange` use — so the three cannot drift. `agent_open.rs` (the MCP `OpenAgent` path) is **not** fixed; it is gap G3 in the bindings report and needs fix 2. In `launchAgentDefinition`, build
   `cmd:args` with the same helpers the per-send path uses
   (`withProviderFlags(buildRuntimeArgs(base, runtimeConfig, provider.id), flags)`),
   then append the one-shot `--fork-session`. The meta commit then carries
   `cmd:args` and `agent:runtime` that agree. Audit `launchAgent`
   (`agent-model.ts:277-357`) and srv's `agent_open.rs:528`
   (`resolve_cli_args` + `provider_flags`, used by the MCP `OpenAgent` path) for
   the same gap. Neither applies `agent:runtime` today.
2. **Make the srv spawn authoritative.** When the persistent controller spawns
   (including eager resume), derive `--model` / `--effort` from `agent:runtime`
   if `cmd:args` lack them, or at least log a warning that they're missing.
   Then a missing frontend rebuild can't silently fall back to CLI defaults.
3. **Respawn when the per-send rebuild changes the args of a running process.**
   If the rebuilt `cmd:args` differ from the running process's argv, go through
   the same respawn as `runtime-apply.ts` (at a turn boundary), rather than only
   writing meta.
4. **The strip should show what's actually running.** Show the model and effort
   the live process was given, e.g. from the spawn argv or the CLI's `init`
   event, which reports the resolved model. If they differ from the selection,
   flag the difference in the strip.
5. **Sonnet 5.5.** Update the curated `sonnet` label and CLI pin once we've
   confirmed which CLI version resolves `sonnet` to Sonnet 5.5. Keep the default
   on the `sonnet` alias.

## 6. Verification for the fix

*Unit coverage for fix 1 is in `frontend/app/view/agent/pane-args-parity.test.ts`: every catalog provider, plus a guard that nothing else composes `cmd:args`. The srv and live checks below are still to do.*

- **Unit:** a `launchAgentDefinition` continuation launch (with
  `continueSid`/fork) commits `cmd:args` containing `--model <runtime.model>` and
  `--effort <runtime.effort>`. `--fork-session` stays out of the persisted args
  used on later sends.
- **srv:** an eager-resume spawn whose `cmd:args` lack `--model` gets it from
  `agent:runtime`, or logs the warning.
- **Live:**
  1. Continue an agent into a fresh channel, the path that failed here.
  2. Check the spawned process's command line
     (`Get-CimInstance Win32_Process … CommandLine`) for `--model sonnet --effort high`.
  3. Check that the CLI's `init` event reports a Sonnet model.
  4. Ask the agent which model it's running.

## 7. How it was found

- The owner saw "Sonnet 5.5" in the strip and asked AgentX which model it was
  running. AgentX answered Opus 5.5.
- AgentX checked its own process: no `--model` or `--effort` in the command
  line, nothing in `settings.json`, and no cached org default.
- The spawn log line and the block's current meta showed the gap between launch
  and first send.
