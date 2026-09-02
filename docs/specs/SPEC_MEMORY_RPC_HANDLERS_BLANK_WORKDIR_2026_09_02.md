# Spec: fix agent:memory:{list,read_file,write_file,revert} for a blank working_directory

**Date:** 2026-09-02
**Status:** Proposed
**Motivated by:** a live smoke test of the Armory Personal Memory grid (PR #2917,
v0.55.31) — asked to verify that writing a memory via the `MemoryWrite` MCP tool
actually surfaces in the Armory UI.

## What the test found

Writing a memory via `MemoryWrite` (this session's own agent, definition id
`43f2b0c6-...`, display name "Manoz") succeeded and the file existed on disk.
The Armory Personal Memory grid, on the exact same build, showed "No memories
yet" for that agent — both from the card's cached count and from a fresh
`agent:memory:list` RPC call issued directly (bypassing the UI). Attempting a
raw `agent:memory:write_file` RPC for the same agent failed outright with
*"agent 43f2b0c6-... has no configured working directory."*

## Root cause

`SPEC_FIX_PERSONAL_MEMORY_EMPTY_WORKDIR_2026_09_01.md` (#2901) fixed exactly
this class of bug — but only inside `memory_dir_for_agent` and
`memory_dir_for_agent_by_id` (`native_memory_handlers.rs`), the resolvers used
by the App API (`app_api/mod.rs`, what `MemoryWrite`/`MemoryList` MCP tools
call) and by `bundle.rs`'s import/export.

The four WebSocket RPC handlers that back the Armory UI —
`agent:memory:list`, `read_file`, `write_file`, `revert` — never called either
fixed resolver. Each had its **own, separate, un-synced** inline check:

```rust
if agent.working_directory.is_empty() {
    return Ok(Some(... files: vec![] ...));      // list, read_file: silent empty
    // or
    return Err("... has no configured working directory");  // write_file, revert: hard error
}
```

This is the exact pre-#2901 behavior (`list`'s short-circuit dates to PR
#1588), sitting right next to the fixed code without ever calling it. The
`memory_dir_for_agent_by_id` doc comment already (incorrectly) claimed *"Shared
by those three handlers... so all five resolve identically"* — aspirational,
not actual, prior to this fix.

Consequence: **any agent whose definition has a blank `working_directory`**
(the default state — `agent.open` substitutes a derived directory at spawn
time rather than persisting it back to the definition row) shows "No memories
yet" in Armory Personal Memory no matter how many real memory files it has.
`list` never errors for this case, so it's indistinguishable in the UI from a
genuinely-empty agent — the exact trap
`SPEC_ARMORY_PERSONAL_MEMORY_AGENT_BLOCKS_2026_09_01.md`'s four-state card
design exists to catch, just via a different root cause (silent
success-with-empty, not #2901's HTTP 500) than the one it was built against.

## Fix

Repoint all four handlers at `memory_dir_for_agent_by_id(&wstore, &agent)` —
the same resolver `bundle.rs` and `native_memory_drift.rs` already use —
instead of their own inline `working_directory.is_empty()` check +
`memory_dir_for_cwd` call. `memory_dir_for_agent_by_id` returning `None` (only
possible if the agent has no name at all — a pathological case) is treated as
an error, matching the existing "error must be visually distinct from empty"
convention rather than silently reporting zero files.

`history` and `diff` are unaffected — they resolve purely through
`id_store`/`agent.id` (the SQLite version table), never touch
`working_directory`, and were never part of this bug.

## Tests

Three new handler-level regression tests in `native_memory_handlers.rs`,
each driving the real `WshRpcEngine` dispatch (not calling the resolver
function directly — that path was already covered; the gap was the handlers
bypassing it):

- `write_file_resolves_a_blank_working_directory_instead_of_erroring` — a
  blank-workdir `write_file` call now succeeds (previously errored), and the
  written content round-trips through `list`/`read_file`.
- `list_finds_files_already_on_disk_at_the_derived_default_dir_for_a_blank_workdir_agent`
  — a file planted directly on disk at the derived-default directory (not
  written through this RPC surface) is found by `list` — proves `list`'s own
  resolution is fixed, not just piggybacking on `write_file` now using the
  same (fixed) path.
- `revert_resolves_a_blank_working_directory_instead_of_erroring` — same
  shape for `revert`.

All three take the module's existing `ENV_LOCK` (matching
`blank_working_directory_resolves_to_the_same_default_agent_open_substitutes`,
which exercises the same registry-lookup code path) to avoid racing other
tests that mutate process-global env state.

## How this was caught

Live-tested against a running v0.55.31 build via CDP (`Runtime.evaluate`
against the app's exposed `window.RpcApi`/`window.TabRpcClient`), not
inferred from reading the code — the same "measure, don't just read" practice
this session's operator notes (`~/.agentmux/shared/providers/claude/CLAUDE.md`
§6) call out. Reading the code alone would very plausibly have missed this:
`memory_dir_for_agent_by_id`'s own doc comment already claims the three
handlers use it.

## Non-goals

- No change to `history`/`diff` — not part of this bug.
- No change to the Armory grid UI itself (`native-memory-manager.tsx`,
  `MemoryAgentCard.tsx`) — this is a backend fix; the four-state card design
  from #2917 already handles the corrected behavior correctly once the
  backend stops silently reporting empty.
- No change to `memory_dir_for_agent`/`memory_dir_for_agent_by_id` themselves
  — #2901 already fixed their internal logic; this PR only fixes their
  *callers*.
