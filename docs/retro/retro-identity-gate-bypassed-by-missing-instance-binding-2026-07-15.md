# Retro — tonight's fail-closed spawn gate never runs for the agents actually running in production

**Date:** 2026-07-15
**Trigger:** Live user report, minutes after the previous auth-isolation retro
merged: *"there are no logged in accounts in the armory, but I am still able
to communicate to the Lzop agent."* Verified directly against the running
databases — not reproduced from a synthetic test, found live.
**Severity:** High. This is not a new bug — it means **PR #2164 (tonight's
"fail spawn on missing oauth account" gate) does not gate the agents that
are actually running right now**, in either of the two live AgentMux
instances inspected. The gate is real code, correctly implemented for the
data shape it assumes; that shape is not what real agents have.
**Related:** `docs/retro/retro-auth-isolation-invariant-silently-orphaned-2026-07-14.md`
(same resolver file, same failure pattern, written six hours earlier —
its own closing line applies verbatim here too: *"a written invariant is
worthless if nothing re-checks it when the ground moves."*)

---

## 1. What was verified live (not theoretical)

Direct SQLite inspection of both currently-running AgentMux instances
(dev:main channel at `~/.agentmux/dev/main/bdbef7b72912a6f3`, v0.53.5, built
from tonight's `bf1b4e64` with PR #2164 included; and the v0.53.3 portable
at `~/.agentmux/channels/local-main-b28b7a-5bc5caca/versions/0.53.3`):

- **`db_accounts` is empty in the shared store** (`~/.agentmux/shared/store.db`)
  — zero rows. This is the "no logged-in accounts in the Armory" the user
  saw, and it's accurate; there really are none.
- **Every agent-shaped block in both instances is still running.** A block
  named "Lzop" (`f3b242ca-9a3a-47ba-a30d-c5836cc85068`, `agentProvider:
  "claude"`) in the dev:main channel, and this very conversation's own pane
  (`30e0638f-edd0-417a-9ea7-a744b31d8646`) in the v0.53.3 channel — **2 for
  2** of the agent-shaped blocks actually observed running had **zero**
  matching row in `db_agent_instances`, and for Lzop specifically, no
  matching row in `db_agents` either (`SELECT ... FROM db_agents WHERE
  id='9b1b8afe-447c-4461-9619-09c701c4fa86' AND is_template=0` → empty).
- Lzop's block carries a **frozen** `cmd:env.CLAUDE_CONFIG_DIR` =
  `C:\Users\area54\.agentmux\shared\providers\claude` — the old shared
  isolated dir from the pre-Phase-4 Default-bundle era (see the prior
  retro). That directory's `.credentials.json` is still being actively
  refreshed (last write: this morning) by whatever else uses it — meaning
  Lzop keeps working not because it was gated-and-passed, but because
  nothing ever asked it to prove anything, and the directory it was frozen
  to point at happens to still hold a live login for unrelated reasons.

## 2. Root cause — two escape hatches inside the gate function itself, both untouched by tonight's PR #2164

`agentmux-srv/src/identity/resolver.rs:596`,
`inject_identity_env_with_broker`, runs in four steps. **PR #2164 only
changed steps 3 and 4** (the `gate_oauth_failure` closure and its call
sites). Steps 1 and 2 were left exactly as they were, and both are
unconditional early-returns that skip the gate entirely:

**Step 1** (`resolver.rs:592-604`, calling
`backend/storage/agents.rs:1738` `instance_get_active_for_block`):

```rust
let instance = match wstore.instance_get_active_for_block(block_id) {
    Ok(Some(i)) => i,
    Ok(None) => {
        // Block has no agent instance row — nothing to inject, and no
        // gating either: quick-launch panes that never went through the
        // launch modal are outside the managed-credentials contract.
        return Ok(());
    }
    ...
```

`instance_get_active_for_block` itself has two tiers: it first tries
`db_agents WHERE id = <block's agentId> AND is_template = 0` (the modern,
consolidated table — Phase 3b.3), and only falls back to the legacy
`db_agent_instances` table if that misses. **Lzop misses both tiers** — its
`agentId` doesn't correspond to any row in either table — so Step 1 returns
`None` and the whole function returns `Ok(())` before the layer-3 gate is
ever reached.

**Step 2** (`resolver.rs:606-624`), reached only when Step 1 *does* find a
`db_agents` row:

```rust
if instance.identity_id.is_empty() || instance.identity_id == "blank" {
    // Empty or legacy "blank" sentinel → ambient creds (no
    // injection). ...
    // Deliberately NOT gated by layer 3: this sentinel predates the
    // managed-account model and was an explicit "ambient creds"
    // choice at launch time, not a silent fallback.
    ...
    return Ok(());
}
```

This is the same shape of bug, one level deeper: `db_agents.identity_id` is
a real column, populated by exactly one write path
(`agents_dual_write_instance_create`, `backend/storage/dual_write.rs:206-260`),
which only fires when something calls `instance_create` — and that is
**disconnected from** the account-linking system Steps 3/4 actually gate
on (`db_agent_identity_links`, written by `LinkAgentIdentityCommand`,
`agent_handlers/identity.rs:518-539`, which never touches `db_agents`).
An agent can have a real, live, correctly-bound account in
`db_agent_identity_links` and still sail past Step 2 ungated, because that
table and the `identity_id` sentinel Step 2 checks are two different,
never-reconciled signals.

## 3. Why this wasn't caught before shipping PR #2164 tonight

Every test added for PR #2164 (`spawn_blocked_when_bound_oauth_account_missing_and_flag_false`,
`spawn_blocked_when_oauth_def_provider_has_no_binding_and_flag_false`, etc.)
constructs its fixture by calling `store.instance_create(&inst)` /
`store.agent_def_insert` directly in the test setup — i.e., every test
*starts* from a state where Step 1 and Step 2 already pass, and only then
exercises Steps 3/4. This is not a criticism of the tests' logic (they
correctly prove what they set out to prove) — it's that **nothing in the
test suite constructs the fixture shape real running agents actually have**:
a block with agent-shaped meta and no instance/db_agents row at all, or one
with a row but a blank `identity_id`. The two production call sites that
create such blocks —

- **App API `agent.open`** (`server/app_api/agent_open.rs`) — opens a pane
  and sets meta; never calls `instance_create` at all. This is the path
  external tools and orchestrators use, per `README.md:61`.
- **The canonical frontend launch modal** (`frontend/app/view/agent/agent-model.ts:277-629`) —
  *does* call `RpcApi.CreateAgentInstanceCommand`, but only **after** the
  actual spawn (`ControllerResyncCommand`, line 541), wrapped in a
  try/catch that logs and continues on failure (`:589-594`, "a failure here
  doesn't abort the launch, the agent already started"). Best-effort,
  not a precondition.

— were never exercised by anything added tonight, so the gate's own
internal escape hatches were invisible to the very PR that was supposed to
close exactly this hole.

## 4. What this means about "the norm," not "the edge case"

Across the ~90 call sites referencing `instance_create`/`db_agent_instances`
in the codebase, essentially all except two production call sites are test
fixtures. Of the two real ones, one deliberately creates a bindingless stub
(`agent_handlers/agent_define.rs`'s `make_stub_idempotent`, `block_id: ""`,
`identity_id: ""`) and the other is the best-effort post-spawn call above.
Combined with the live evidence (2 for 2 running agent-shaped blocks
missing the row), the honest conclusion is: **the "managed-credentials
contract" the Step-1 comment refers to is not a boundary most real agents
are inside of — it's closer to the exception.** Tonight's gate is
correctly built for a data shape that doesn't describe how agents actually
come to exist in this system today.

## 5. What needs to change (not implemented here — see the companion spec)

This retro documents the finding; `docs/specs/SPEC_UNIVERSAL_IDENTITY_GATEWAY_2026_07_15.md`
proposes the fix. Summary of the direction: stop asking the gate to trust
two disconnected identity signals (`db_agents.identity_id` sentinel vs.
`db_agent_identity_links` rows) reached through a resolvability chain
(block → agentId → db_agents-or-db_agent_instances) that real spawn paths
don't reliably populate. Move the credential decision to a single point
every spawn passes through unconditionally, keyed directly on the binding
table Steps 3/4 already trust, not on an intermediate row that may or may
not exist.

## 6. The pattern, stated plainly, for whoever reads this next

This is the **second** silent-orphaning finding on this exact file in one
day. The first (`retro-auth-isolation-invariant-silently-orphaned-2026-07-14.md`)
was about a *removed* isolation mechanism. This one is about a gate that
was *added* correctly but only for a precondition that doesn't hold in
production. Both are the same lesson from two different directions: this
resolver function has accumulated enough independent, uncoordinated
decision points (four steps, two of them unconditional early-exits) that
no single PR touching it can reason about the whole thing at once anymore.
That is itself the argument for collapsing it into one gateway rather than
patching step 5.
