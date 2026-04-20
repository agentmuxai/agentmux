# Portable Agent Working Dirs

**Date:** 2026-04-20
**Status:** Proposed
**Depends on:** [`portable-data-dir.md`](./portable-data-dir.md)

---

## Problem

A portable AgentMux (extracted ZIP with `runtime/` + `data/`) still writes
**per-agent state** to the user's home directory instead of its own `data/`.
Specifically, from a portable running at `C:\...\agentmux-0.33.286-x64-portable\`,
every Forge agent I launch gets:

- working directory `%USERPROFILE%\.agentmux\agents\<slug>\`
- `GH_CONFIG_DIR=%USERPROFILE%\.agentmux\config\gh-<slug>`

That leaks agent state out of the portable, collides between coexisting
portable instances, and breaks the "copy the folder, keep everything" promise
of a portable build.

The parent spec [`portable-data-dir.md`](./portable-data-dir.md) addresses the
**CEF host and sidecar** data roots (db, config, logs, CEF cache). It does
**not** address agent working directories — those are constructed by the
**frontend** (`frontend/app/view/agent/agent-model.ts`) and serialized into
block metadata as `cmd:cwd` / `cmd:env`, bypassing `AGENTMUX_DATA_HOME`.

---

## Root cause

`frontend/app/view/agent/agent-model.ts`:

- `agentmuxHome()` (~L413) unconditionally returns `${HOME}/.agentmux`. No
  portable awareness.
- `launchForgeAgent()` (~L257) sets
  `workDir = agent.working_directory || ${agentmuxHome()}/agents/${slug}`.
- Same function (~L290) sets
  `envVars["GH_CONFIG_DIR"] = ${agentmuxHome()}/config/gh-${slug}`.

The CEF host already resolves a portable `data_dir` and passes it to the
sidecar as `AGENTMUX_DATA_HOME` (per `portable-data-dir.md` §3 / current
`agentmux-cef/src/main.rs` portable detection). The frontend never receives or
uses it.

---

## Goal

Agent working directories and per-agent config paths for a portable instance
must live under the portable's `data/` folder, matching the parent spec's
layout:

```
<portable>/data/agents/<slug>/          ← cmd:cwd
<portable>/data/config/gh-<slug>/       ← GH_CONFIG_DIR
<portable>/data/config/auth/.../projects/<slug>/   ← Claude project dir
```

Installed builds keep `~/.agentmux/agents/...` (unchanged).

## Non-goals

- Auto-migrating existing agent dirs from `~/.agentmux/agents/` into the
  portable's `data/agents/`. The parent spec is explicit about no
  auto-migration; this spec inherits that.
- Changing the slug format, the `AGENTMUX_AGENT_ID` env var, or any public
  agent-identity contract.
- macOS / Linux portable layouts. Same approach applies, but Windows is the
  shipping portable target today.

---

## Design

### 1. Surface `data_home` to the frontend

The CEF host already computes a `data_dir` at startup (portable or installed).
Expose it to the webview via the existing bootstrap IPC so the frontend can
read it synchronously before any agent launch.

- Add a field to the endpoints/bootstrap payload returned by
  `get_backend_endpoints` (or equivalent startup IPC):
  `dataHome: string` — absolute OS path, forward-slash-normalized on Windows.
- Populate from the same variable that sets `AGENTMUX_DATA_HOME` for the
  sidecar, so there is a single source of truth.

### 2. Frontend uses `dataHome` for agent path construction

In `agent-model.ts`:

- Replace `agentmuxHome()` with a function that returns the `dataHome`
  received from the bootstrap IPC. Fall back to `${HOME}/.agentmux` only if
  the IPC hasn't delivered (defensive; should never happen in practice).
- `launchForgeAgent()` keeps the same construction; it reads the new
  `agentmuxHome()` transparently.

No other callers change — everything downstream (`cmd:cwd`, `GH_CONFIG_DIR`,
Claude project directory derived from `cmd:cwd` path) follows automatically.

### 3. Claude project directory

The Claude CLI stores per-project state under
`~/.claude/projects/<slug-of-cwd>/`. That path is owned by the Claude CLI, not
AgentMux, so we can't directly redirect it. Two options:

- **Option A (recommended):** set `HOME` in the spawned agent's `cmd:env` to
  the portable `data/` when in portable mode. Claude CLI then writes to
  `<portable>/data/.claude/projects/...` automatically.
- **Option B:** accept leakage for `~/.claude/` — document it as a known
  limit of the isolation.

Option A is cleaner and consistent with the portable promise, but changing
`HOME` has broad effects on any child process. Flag this as an open question
below.

---

## Changes required

| File | Change |
|------|--------|
| `agentmux-cef/src/main.rs` | Add `dataHome` to the bootstrap IPC payload (same value used for `AGENTMUX_DATA_HOME`). |
| `agentmux-cef/src/ipc.rs` | Serialize `dataHome` in `get_backend_endpoints` response. |
| `frontend/app/view/agent/agent-model.ts` | Rewrite `agentmuxHome()` to consume `dataHome` from bootstrap. No change to `launchForgeAgent()`'s call sites. |
| `frontend/app/store/...` | Wire the bootstrap's `dataHome` into whatever global config atom/signal the agent view reads. |
| `docs/specs/portable-data-dir.md` | Cross-reference this spec from the "What Does NOT Change" table (remove agents from scope or link here). |
| `README.md` | Agents section's slug claim (`Drives ~/.agentmux/agents/<slug>/`) gains a note: portable instances use `<portable>/data/agents/<slug>/`. |

No backend (Rust) handler changes. No rpc_types changes. No App API surface
changes.

---

## Verification

1. Extract the portable ZIP to a fresh folder. Launch. Open a Forge agent.
2. Confirm `<portable>/data/agents/<slug>/` is created and contains the
   agent's working state.
3. Confirm `%USERPROFILE%\.agentmux\agents\<slug>\` is **not** created.
4. `GH_CONFIG_DIR` env (inspect via `/cmd env` slash command or
   `agent.status` App API) points at `<portable>/data/config/gh-<slug>`.
5. Installed build (MSI or `task dev`) unchanged: `~/.agentmux/agents/<slug>/`
   still used.
6. Two portable instances side-by-side each get independent `data/agents/`.

A future harness test under `tools/tests/` could assert (1)-(3)
programmatically via the App API (`agent.open` → `agent.status` to read the
resolved `cmd:cwd`).

---

## Slug is per-instance, not globally unique

This is the motivating safety argument for the fix, worth stating explicitly
because the current layout assumes otherwise.

Per `agentmux-srv/src/backend/storage/wstore.rs:565-598`, `forge_insert`
auto-resolves slug collisions **within one Forge DB** by appending `-2`,
`-3`, etc. under a mutex-guarded uniqueness scan. A unit test
(`test_forge_insert_collision_resolves_at_runtime`, L1253) confirms this.

Each AgentMux instance — installed build, any number of coexisting portables
— has its own Forge DB. So two instances can independently produce an agent
with slug `agentx`. The current path layout
`~/.agentmux/agents/<slug>/` is **globally shared across instances on the
same machine**, so those two `agentx` agents clobber each other's working
directories, GitHub configs, Claude project state, and auth dirs.

The portable-data-dir layout (`<portable>/data/agents/<slug>/`) removes that
collision by making the data root per-instance. Installed builds remain on
`~/.agentmux/` but are the single instance of their kind (one install per
user account), so they're safe in practice. This spec is what closes the
multi-portable and portable-vs-installed coexistence gap.

## Open questions

- **`HOME` override for child processes (Option A vs B above).** Decide
  before implementing; affects whether Claude CLI and other per-user
  integrations are isolated to the portable or leak to the host home.
- **Multi-portable coexistence safety.** If the user runs two portables in
  parallel, each binds to its own `data/`. But shared external state (e.g.
  GitHub PAT stored by the `gh` CLI under `GH_CONFIG_DIR`) is per-portable;
  confirm that's the intended semantics, not "share auth across portables".
- **Dev mode.** `task dev` uses `~/.agentmux-dev` today; unchanged by this
  spec. Worth confirming agent dirs under dev mode also follow the new rule
  (they should, since `agentmuxHome()` will defer to whatever `dataHome` is
  surfaced).

---

## Sequence

1. Land `portable-data-dir.md` first (parent spec). Without it, the CEF host
   doesn't have a resolved portable `data_dir` to surface.
2. Add `dataHome` to the bootstrap IPC.
3. Update `agent-model.ts` to consume it.
4. Doc updates (README + parent spec cross-reference).
5. Optional: harness test under `tools/tests/` asserting portable isolation.
