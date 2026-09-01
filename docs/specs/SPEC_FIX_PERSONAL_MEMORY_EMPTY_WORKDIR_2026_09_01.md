# Spec: Personal Memory is empty for any agent with a blank `working_directory`

**Date:** 2026-09-01
**Status:** Proposed
**Severity:** P1 — a whole Armory tab silently shows nothing for the common case
**Found by:** operator asked to verify that an agent's written memories show up
in Armory → Memory → Personal. They do not.

## Problem

`MemoryList` (MCP) and Armory → Memory → **Personal** both resolve an agent's
native-memory directory through
`native_memory_handlers.rs::memory_dir_for_agent()`. For this agent
(`manoz`), that call fails outright:

```
memory/list failed: HTTP 500 — {"error":"memory: agent manoz has no working directory"}
```

while the memory files sit on disk, intact and correct:

```
.../claude/projects/C--Users-area54--agentmux-agents-manoz-0803a/memory/
    MEMORY.md
    feedback_agentmux_cdp_live_debugging.md
    feedback_merge_on_approval.md
    project_composer_strip_layout_history.md
    reference_secrets_cli.md
```

## Root cause

`memory_dir_for_agent()` resolves in two stages — instance row first,
registry second — but the first stage **errors out** instead of falling
through when the row's `working_directory` is blank:

```rust
if let Some(instance) = wstore.instance_get_by_slug(agent_id)? {
    if instance.working_directory.is_empty() {
        return Err(format!("memory: agent {agent_id} has no working directory"));
    }
    ...
}
memory_dir_from_registry(agent_id).ok_or_else(...)   // ← never reached
```

A blank `working_directory` is **not** an error condition. `agent_open.rs`
substitutes a default whenever the field is blank:

```rust
let work_dir = if agent.working_directory.is_empty() {
    format!("~/.agentmux/agents/{}", agent_slug)
} else { agent.working_directory.clone() };
```

So a blank row describes an agent that is nonetheless running, and writing
memories, in a real directory. The registry — deliberately consulted second,
and which reconstructs the true path from `source_agents_base` +
`working_dir` (`memory_dir_for_registry_record`) — is precisely the component
that knows that directory. The early `Err` prevents it from ever being asked.

Because `working_directory` is **blank by default**, this is the common case,
not an edge case: Personal Memory is broken for most agents.

This also explains the shape of the failure — not "no memories found" (an
empty list) but a hard HTTP 500. The tab cannot distinguish "this agent has
no memories yet" from "resolution blew up", so it surfaces as broken rather
than empty.

## Design

Treat a blank `working_directory` as "this row cannot answer" rather than
"the answer is no": use the instance row only when it actually carries a
working directory, and otherwise resolve the real directory another way.

### Resolving the blank case (revised after review)

The first cut simply fell through to the existing registry fallback. Review
found two holes in that, both real:

1. **The registry doesn't cover the target case** (Codex P1). `agent.open`
   computes and uses `~/.agentmux/agents/<name-slug>` but does **not** create
   a named-agent registry record, so a freshly defined blank-workdir agent —
   exactly what this fix targets — still ended at `not found`.
2. **The registry lookup can resolve the WRONG agent** (Codex P1).
   `find_active_registry_record_by_slug` matches on
   `derive_slug(instance_name)` alone, so two agents whose display names
   slugify identically collide. Resolving another agent's memory dir would let
   list/read/write operations touch that agent's files.

So the blank case resolves in two bound stages
(`memory_dir_for_blank_working_dir`):

1. A registry record **whose `definition_id` matches this agent** — the
   registry records the dir an agent was really launched with, which beats
   re-deriving it, but identity must be verified.
2. Otherwise the **derived default**, via a new shared
   `default_agent_working_dir(name)` in `backend/storage/agents.rs` that
   `agent.open` now also calls. Extracting it is the point: the resolver and
   the spawn path previously disagreed about where a blank-workdir agent runs,
   and that disagreement *was* the bug. They can no longer drift.

Returns `None` when the agent has no name either — genuinely unresolvable,
and the one case where callers' fail-fast guards still fire.

### Knock-on: the import fail-fast guard narrows

`bundle.rs`'s `import_for_agent` refuses to import when `memory_dir` is
`None`, so a bundle's memory isn't silently dropped (reagent P2, PR #2527).
A blank `working_directory` no longer reaches that guard — it now resolves.
That **serves the guard's own stated purpose better than erroring did**: the
memory gets written where the agent will actually read it, instead of the
import being refused. The guard stays for the genuinely-unresolvable case
(blank name), and its test was renamed and re-pointed at that case rather
than deleted.

The same applies to `export_for_agent`, which ReAgent flagged as silently
exporting an empty memory list for these agents — it now finds the files.

`agentmux-srv/src/server/native_memory_handlers.rs`:

```rust
if let Some(instance) = wstore.instance_get_by_slug(agent_id)? {
    if !instance.working_directory.is_empty() {
        let config_dir = /* … unchanged … */;
        return Ok(memory_dir_for_cwd(&config_dir, &instance.working_directory));
    }
    // blank → fall through to the registry below
}
memory_dir_from_registry(agent_id).ok_or_else(...)
```

The error path is preserved for the genuine failure — an agent resolvable by
neither the instance row nor the registry still returns the registry's own
`agent {id} not found`.

### Tests

Two, both in `native_memory_handlers.rs`'s existing `tests` module:

- **`blank_working_directory_falls_through_to_the_registry_rather_than_erroring`**
  — an instance row with `working_directory: ""` and no registry record must
  fail with the registry's `not found`, **not** `has no working directory`.
  Asserting on *which* error proves control reached the fallback instead of
  stopping at the guard — the exact regression.
- **`non_blank_working_directory_still_resolves_from_the_instance_row`** — the
  unaffected path keeps resolving straight from the row, asserted on path
  components so mixed separators don't matter on Windows.

## Non-goals

- **No change to the registry fallback itself** (`memory_dir_from_registry`,
  `memory_dir_for_registry_record`) — it was already correct and already the
  intended handler for live agents; it just wasn't being reached.
- **No change to `agent_open.rs`'s default-working-dir substitution.** Making
  it *persist* the substituted value into the instance row would also fix the
  symptom, but is a broader behavioural change to spawn, and would leave every
  already-created agent row still blank. Fixing the resolver repairs existing
  data as well as new.
- **No attempt to distinguish "no memories yet" from a resolution failure in
  the UI.** Worth doing (an empty Personal tab and a 500 look different to a
  user), but separate from this fix.
