# Report: global MCP servers/skills are always injected regardless of bind state

## Summary

`bound_to_agent` — the flag driving every "Bind"/"Unbind" toggle in the
Armory and Stash MCP Servers/Skills tabs — has no effect on an agent's
actual materialized runtime config. `agent_open.rs`'s config-generation
path injects **every global MCP server and every global skill into every
agent's config, unconditionally**, regardless of whether that specific
agent is bound to it. Binding/unbinding only changes `db_agent_mcp_ref`/
`db_agent_skills_ref` (which currently drives nothing in config
generation for global rows) and the UI's own "bound"/"not bound" badge.

Flagged by reagentx's re-review of PR #2329 (which made `mcp.catalog.unbind`/
`skill.catalog.unbind` reachable from the UI for the first time — see
"Why this wasn't caught sooner" below).

## Root cause

`agentmux-srv/src/server/app_api/agent_open.rs:558` and `:589`:

```rust
// v1 skills: globals are always injected; the agent's OWN ref-bound skills
// are authoritative when present, otherwise fall back to legacy
// db_agent_skills. ...
let visible_skills: Vec<Skill> = wstore.skill_list(&agent.id)
    .unwrap_or_default()
    .into_iter()
    .map(|item| item.skill)
    .collect(); // own refs + globals
```

`Store::skill_list`/`Store::mcp_server_list`'s query is:

```sql
WHERE s.is_global = 1
   OR s.id IN (SELECT skill_id FROM db_agent_skills_ref WHERE agent_id = ?1)
```

The `is_global = 1` branch is unconditional — it does not check whether
`agent_id` holds a ref to that specific global row. `bound_to_agent` (the
`EXISTS(...)` subquery in the SELECT list) is computed and returned, but
config generation immediately discards it (`.map(|item| item.skill)`,
per the code's own comment: *"this config-generation path doesn't need
it, so unwrap immediately"*).

So `bound_to_agent`'s only real consumer today is deciding **ref-based
vs. legacy-fallback mode** (`has_own_skill_refs` / `has_own_mcp_refs` —
whether *any* of the agent's own private rows exist, which switches
between "trust the ref tables" and "merge legacy blob + globals"). It was
never wired to gate *global* row inclusion.

## Why this wasn't caught sooner

Before PR #2329, `mcp.unbind`/`skill.unbind` were `check_s1`-gated
(agent-self-service only) and no `mcp.catalog.unbind`/`skill.catalog.unbind`
existed — there was no way to actually invoke unbind from any UI. Only
`mcp.catalog.bind`/`skill.catalog.bind` (added in PR #2317) were
UI-reachable. Binding a global item that's *already* unconditionally
injected has no observable effect either way, so this never looked broken
in practice: click "Bind," nothing changes, but nothing looked wrong
because the item was already there.

Unbind is the first UI-reachable action whose absence of effect is
actually visible — a user clicks "Unbind," the badge flips to "not
bound," and the server/skill keeps being injected into the agent's next
generated config regardless.

## Current mitigation (landed in PR #2329)

Added an inline caveat to `AgentMcpModal`/`AgentSkillsModal`'s detail view,
next to the Bind/Unbind buttons:

> Note: every global server is currently applied to every agent regardless
> of bind state — unbinding here updates what shows as "in use" but does
> not yet remove it from this agent's live config.

This is honest-UI-copy only — no behavior change, no risk to existing
config generation.

## Why a real fix is out of scope for that PR

Making `bound_to_agent` actually gate global-row inclusion in
`agent_open.rs` means every agent that has never explicitly bound/unbound
anything (i.e. every agent that existed before bind/unbind was
UI-reachable — almost certainly all of them) would suddenly stop
receiving every global MCP server/skill it currently gets, on next config
regeneration, unless a migration backfills an explicit bind ref for every
`(agent, global-item)` pair that's implicitly "in use" today. That's a
correctness-critical, blast-radius-on-every-agent change to the
config-generation path used on every launch — not something to bundle
into a PR whose actual goal was fixing Stash's `check_s1` "unauthorized"
errors.

## Recommended follow-up (separate PR/spec)

1. Write a migration that inserts a `db_agent_mcp_ref`/`db_agent_skills_ref`
   row for every `(agent, global item)` pair currently implicitly active
   (i.e. seed every existing agent as bound to every existing global item)
   — this makes "currently active" and "explicitly bound" the same set at
   migration time, so no agent's config silently changes on the next
   regeneration.
2. Change `agent_open.rs`'s `visible_skills`/`visible_mcp` computation to
   filter the global branch on `bound_to_agent`, not include it
   unconditionally — i.e. only inject a global row the agent actually
   holds a ref to.
3. Update `Store::skill_list`/`Store::mcp_server_list`'s query (or add a
   variant) to match — currently the `WHERE s.is_global = 1 OR ...` clause
   returns every global row for every agent by design; after the fix it
   should be `WHERE s.id IN (SELECT ... WHERE agent_id = ?1)` uniformly
   (own refs and bound-global refs collapse into the same ref-table
   lookup, dropping the two-branch OR entirely).
4. Regression-test: an agent that has never touched Armory/Stash should
   see identical config before/after the migration (proves the backfill
   is complete); an agent that explicitly unbinds a global item should
   see it actually disappear from its next generated config (proves the
   fix closes the gap this report documents).
5. Remove the caveat text added in PR #2329 once this lands.

## Files referenced

- `agentmux-srv/src/server/app_api/agent_open.rs` (lines ~550-620,
  `visible_skills`/`visible_mcp` computation)
- `agentmux-srv/src/backend/storage/skills.rs` (`skill_list`)
- `agentmux-srv/src/backend/storage/mcp_servers.rs` (`mcp_server_list`)
- `frontend/app/view/agent/components/AgentMcpModal.tsx` /
  `AgentSkillsModal.tsx` (caveat UI added in PR #2329)
