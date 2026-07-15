# SPEC — universal identity gateway: close the gate's own internal bypasses

**Date:** 2026-07-15
**Status:** Proposed — investigation and live verification complete, design
not yet implemented
**Governing incident:** `docs/retro/retro-identity-gate-bypassed-by-missing-instance-binding-2026-07-15.md`
**Related:** `docs/retro/retro-auth-isolation-invariant-silently-orphaned-2026-07-14.md`,
`SPEC_PROVIDER_ISOLATION_2026_06_20.md` (INV-A), `SPEC_ACCOUNT_DELETE_DEAUTH_LAYERS_2_4_2026_07_14.md`
(PR #2164, the gate this spec extends)

---

## 1. The good news first: there is already only one mechanical spawn gateway

Before proposing changes, it's worth being precise about what's *not* broken.
A full sweep of every place a provider CLI subprocess could plausibly be
launched (`agentmux-srv/src/backend/blockcontroller/`,
`agentmux-srv/src/server/agent_handlers/`, `agentmux-srv/src/server/app_api/`)
found **exactly two call sites, ever**, that invoke credential injection:

| Call site | File:line |
|---|---|
| `agentinput` RPC handler | `agentmux-srv/src/server/agent_handlers/input.rs:168` |
| `agent.send` RPC handler | `agentmux-srv/src/server/app_api/agent_io.rs:165` |

Both call `identity::resolver::inject_identity_env_async` and both correctly
propagate a returned `SpawnGateError` as a blocking spawn failure (verified
during tonight's PR #2164 work). The **container-agent spawn path**
(`spawn_container_turn`, reached from inside `agentinput` at
`input.rs:373-413`) is downstream of the *same* single call — it reuses the
`env_vars` map `inject_identity_env_async` already produced, not a separate
credential path. `agent.open` (`server/app_api/agent_open.rs`) never spawns
a process directly at all — it opens the pane; the actual CLI process is
lazily spawned on first `agentinput`/`agent.send`, which does gate. The MCP
`Shell` tool spawns a *terminal* sub-block inside an already-open agent's
own pane, not a new provider-CLI credential context.

**So this is not a "scattered spawn paths" problem** — the architecture
already funnels every real spawn through one function. The problem is
entirely **inside that one function**: two unconditional early-returns that
were correct when written and have since drifted out of sync with how
agents actually get created.

## 2. The two escape hatches (see the retro for full detail)

`agentmux-srv/src/identity/resolver.rs:596`, `inject_identity_env_with_broker`:

- **Step 1** (`:592-604`): if `instance_get_active_for_block(block_id)`
  returns `None` — no matching row in `db_agents` (primary) or
  `db_agent_instances` (legacy fallback) — return `Ok(())`, no gate.
- **Step 2** (`:606-624`): if a row *is* found but its `identity_id` column
  is empty or the literal string `"blank"` — return `Ok(())`, no gate.

Both are reached in practice. Both are older than PR #2164's Steps 3/4 gate
and were never revisited when that gate was added.

## 3. Why patching Steps 1/2 in place is not enough

The tempting fix — "make Step 1 also return `Err` instead of `Ok(())`" —
would immediately break every agent in the system, including this
session's own pane, since **2 of 2 live-observed running agents hit exactly
this path today**. That's not a reason to leave it; it's a reason the fix
has to also close the upstream gap (real spawns not reliably producing a
`db_agents`/`identity_id` row), not just flip the downstream default.

The deeper issue: Steps 1/2 and Steps 3/4 currently trust **two different,
never-reconciled identity signals**:

- Steps 1/2 trust `db_agents.identity_id`, populated by exactly one
  production write path (`agents_dual_write_instance_create`,
  `backend/storage/dual_write.rs:206-260`), itself only triggered by
  `instance_create`, itself only called from two production sites — one a
  deliberately-bindingless stub (`agent_handlers/agent_define.rs`'s
  `make_stub_idempotent`), one a best-effort call **after** the spawn
  already happened, swallowed on failure
  (`frontend/app/view/agent/agent-model.ts:556-594`).
- Steps 3/4 trust `db_agent_identity_links` (written by
  `LinkAgentIdentityCommand`, `agent_handlers/identity.rs:518-539`) — the
  table that's actually current, actually maintained, and actually what the
  Armory UI reads and writes.

An agent can be fully, correctly bound in `db_agent_identity_links` and
still never reach the code that checks it, because Step 1/2 gatekeep on a
different, effectively-vestigial signal first.

## 4. Proposed fix

### 4.1 Collapse to one identity signal

Stop routing through `db_agents.identity_id` / `db_agent_instances` at all
for the purpose of the credential gate. Resolve directly from the block's
`agentId` (already read at `agents.rs:1747-1751`) against
`db_agent_identity_links` / the agent definition's own provider — the same
data Steps 3/4 already use. Concretely: merge what are currently four
sequential steps with two different data sources into a single resolution
that only ever asks "does this `agentId` have a definition, and does that
definition have (or lack, with explicit opt-in) working credentials for the
provider it needs" — one signal, one path, no intermediate row whose
absence silently means "skip."

### 4.2 Make the gate's default fail closed at every tier, not just tier 3/4

Once 4.1 removes the two-signal split, "no instance row" stops being a
meaningful state to special-case — there's no longer a `db_agents` row to
be missing. What replaces Step 1's early-return should be: no agent
definition for this block's `agentId` → that's the same shape as "unknown
provider," which should log clearly and either block (if the block's own
`agentProvider` meta names an oauth-class provider) or proceed (if it's
provider-agnostic, e.g. a plain terminal block). The important change is
that **there must be no code path where the answer to "does this spawn have
provider X's credentials" is simply never computed.**

### 4.3 A boot-time / CI invariant, not just a spawn-time one

Per both retros' shared conclusion — a spec alone has already failed twice
today to prevent a regression on this exact file — the fix needs a
standing check, not just corrected logic:

- **A live consistency check** (could run at srv startup, or as a periodic
  background sweep, or exposed via `muxlog`/a diagnostics RPC): count
  agent-shaped blocks (`meta.agentProvider` set) whose resolution produces
  neither a bind nor an explicit ambient opt-in nor a blocked state — i.e.,
  blocks that are *silently* running ungated right now. Tonight's
  live numbers (2 for 2) should be **zero** once this ships; the check
  makes "zero" an assertion instead of a hope.
- **A test fixture shaped like production**, not like today's test setup.
  Every current resolver test calls `instance_create`/`agent_def_insert`
  directly, which is precisely the shape real agents *don't* reliably have.
  Add a test that constructs a block the way `agent.open` + first
  `agentinput` actually would (meta set, no instance row, no forced
  identity_id) and asserts the gate still produces a real verdict — block,
  ambient-with-opt-in, or resolved — never a silent, ungated pass-through.

## 5. Full inventory — "every place login may happen," answered

Per the user's request to identify all such places, for the record:

| Surface | What it does | Gated by the spawn-time credential check? |
|---|---|---|
| `agentinput` RPC | Real per-turn spawn/resume | Yes (the one real gate; has the escape hatches above) |
| `agent.send` RPC | Same, alternate entry point | Yes (same function) |
| Container agent turn | Downstream of `agentinput`'s env | Yes, inherits `agentinput`'s outcome |
| `agent.open` RPC | Opens the pane, no process yet | N/A — no spawn happens here |
| Armory "Connect" (`auth.start`/`auth.spawn`) | Interactive OAuth login, **creates** an account row | N/A — this is account creation, not spawn-time credential use; out of scope for this gate |
| `CheckCliAuth` | Validates whether the CLI reports a working login in a given dir | N/A — read-only status probe, not a spawn |
| MCP `Shell` | Spawns a terminal sub-block inside an existing agent's pane | N/A — not a provider-CLI credential context |
| MCP `SetName`/`WhoAmI` | Metadata only | N/A |

The practical conclusion: there is nothing to "gather into" a new gateway —
the gateway already exists and already receives every real spawn. The work
is entirely in making that gateway's own decision tree honest about the
data it actually has, per §4.

## 6. Definition of done

- Steps 1/2 of `inject_identity_env_with_broker` no longer read
  `db_agents.identity_id` / `db_agent_instances` for gating purposes.
- A block with agent-shaped meta and no `db_agents`/`db_agent_instances`
  row (today's Lzop shape) produces a real gate verdict — blocked or
  explicitly-ambient — never a silent `Ok(())`.
- A resolver test constructs exactly this shape (not
  `instance_create`-seeded) and asserts on it.
- A live/diagnostic count of "agent-shaped blocks with no computed
  credential verdict" is either exposed or added as a startup log line,
  so this class of drift is visible going forward instead of requiring a
  live incident to surface it.
