# Retro: subagent watcher misattributes one agent's subagents to unrelated, closed panes

**Date:** 2026-07-23
**Severity:** Medium (data-quality/resource leak — no crash, but corrupts the Swarm data model and leaks OS watch handles + tokio tasks indefinitely)
**Area:** `agentmux-srv/src/backend/subagent_watcher.rs` (since split into `subagent_watcher/` submodules by #2283, landed after this investigation but before this fix — the fix below targets the new module layout)
**Status:** Root-caused and fixed. See the companion PR for the code change.

---

## 1. What the user saw

> "why do I see 'nobile-percolating-ritchie ab9384' in the conversation pane? it appears to be an agent from somewhere else...pull in latest from github, figure out whats causing that"

The concern was reasonable: an unfamiliar-looking agent name appearing unprompted looks like it could be a cross-tenant/security leak (this repo has a documented, real gap along those lines — see `docs/specs/SPEC_MUXBUS_MULTI_TENANT_SECURITY_2026_07_06.md`). It is not that. The actual name (log-verified, not "nobile") is `noble-percolating-ritchie` — a real subagent slug, misattributed to five unrelated, already-closed local agent panes.

---

## 2. Ground truth, with evidence

### 2.1 One real subagent, six attributed owners

Every occurrence of the slug across `~/.agentmux/logs/agentmuxsrv-v0.54.0.log.2026-07-23` carries the identical `session_id`:

```
d019e2e4-7223-4eeb-a2a6-b16b688b9893
```

logged under six different `parent`/`parent_block_id` pairs — `Agent1`, `Agent2`, `Agent3`, `AgentX`, `AgentY`, `Camper`:

```
{"message":"subagent spawned","agent_id":"ad0b1f3b89525bbcf","slug":"noble-percolating-ritchie","parent":"Agent2","parent_block_id":"930f221a-9302-4578-9346-5cc311aef8ff","session_id":"d019e2e4-7223-4eeb-a2a6-b16b688b9893", ...}
{"message":"subagent spawned","agent_id":"a82039d9d4ebab07e","slug":"noble-percolating-ritchie","parent":"AgentY","parent_block_id":"864330fd-339e-4991-9034-ab6da1dc026b","session_id":"d019e2e4-7223-4eeb-a2a6-b16b688b9893", ...}
{"message":"subagent spawned","agent_id":"ae39a5c4a7c274a05","slug":"noble-percolating-ritchie","parent":"Camper","parent_block_id":"fbf55f24-8d30-4c8b-ba8e-7718288ea84d","session_id":"d019e2e4-7223-4eeb-a2a6-b16b688b9893", ...}
```

`find ~/.agentmux -type d -name d019e2e4-7223-4eeb-a2a6-b16b688b9893` returns exactly **one** match:

```
/c/Users/asafe/.agentmux/shared/providers/claude/projects/C--Users-asafe--agentmux-agents-agentx-0623n/d019e2e4-7223-4eeb-a2a6-b16b688b9893
```

— i.e. this is genuinely `AgentX`'s own, real, single session. There is no UUID collision and no duplicate session file. The other five names are misattributions of AgentX's activity to themselves, not five agents that coincidentally share a session.

### 2.2 The five other block_ids no longer exist

Brute-forced every `objects.db` under `~/.agentmux` (93 files, all channels/dev branches/versions) for the six `parent_block_id`s seen in the logs. **Zero matches anywhere** — including in the `stable` channel's own `db_block` table, which is where a currently-open pane's block record would live. These five panes are closed and gone; only `AgentX`'s session is still live and actually producing subagent activity.

### 2.3 The events are not coming from pane-(re)open backfill

The only production caller of `SubagentWatcher::scan_session_subagents` (the backfill path, which logs `"reactive register request"` right before it can run) is `handle_reactive_register` in `agentmux-srv/src/server/reactive.rs:301-357` — confirmed identical in both current `main` and the exact `v0.54.0` release commit (`3d1eb999`) the running binary was built from.

`grep -c -i "register" agentmuxsrv-v0.54.0.log.2026-07-23` → **0**. No register requests happened today. Yet 21 fresh `"subagent spawned"` events (plus continuous `"re-observed under a different parent_block_id"` churn) fired today alone. So these are not backfill-on-reopen events — they are coming from somewhere else entirely.

### 2.4 The timing signature: one fs event, six near-simultaneous attributions

Isolating one burst (`18:04:57.634` → `18:05:01.523`), a *single* underlying change is processed once per synthetic id, and for each one, `"re-observed under a different parent_block_id"` cycles through the exact same six `parent_block_id`s, sub-millisecond apart, in a stable order (`dec53dd5`→`c7f6f560`→`fbf55f24`→`930f221a`→`df70402b`→`864330fd`). That is the signature of **one real filesystem event being delivered to six independent, already-registered watchers**, each of which processes it under its own identity — not six agents independently calling the same backfill function with (coincidentally) the same argument.

---

## 3. Root cause — two independent bugs

### Bug A — `watch_agent()`'s filesystem watch isn't scoped to the calling agent's own files

`agentmux-srv/src/backend/subagent_watcher.rs`, `watch_agent()` (~line 407 onward) sets up a `notify` watcher, recursively, on the agent's resolved Claude `config_dir`. Per `docs/specs/REPORT_SWARM_SUBAGENT_HISTORY_FLOOD_2026_07_07.md` Finding 3, any agent **without an explicit per-identity bundle override** resolves to the single shared default path: `~/.agentmux/shared/providers/claude/`. `Agent1`, `Agent2`, `Agent3`, `AgentX`, `AgentY`, and `Camper` all use the default provider auth, so all six independently registered a recursive watch on the *same physical directory tree*.

The live-watch dispatch loop (lines ~565-609) captures `parent_agent`/`parent_block_id` once per `watch_agent()` call and, for every path the shared `notify` watcher reports changed — with no check that the changed file actually belongs to a subagent spawned within *that* watcher's own agent/session — calls:

```rust
self_clone.process_jsonl_change(&parent_agent, &parent_block_id, &changed_path, true);
```

So one real write to `AgentX`'s subagent transcript fans out to every other agent's watcher on the same shared directory, and each stamps the discovery with its own (wrong) identity. This is the direct cause of the misattribution.

### Bug B — the fs watcher is never torn down when a pane closes ungracefully

The *only* code that removes a `watched_agents` entry (and thereby stops its `notify` watcher + associated tokio task) is `unwatch_agent()` (line 631), called from exactly one place: `handle_reactive_unregister` in `reactive.rs:406`, itself only reached via the frontend's graceful `/agentmux/reactive/unregister` round trip.

The code's own doc comment on `prune_block()` (line ~686) already acknowledges this is unreliable:

> "independent of whether the frontend's normal `/agentmux/reactive/unregister` teardown path (which drives `unwatch_agent`) actually fires for this close — that path depends on a live renderer's `TermWrap.dispose()` completing an async fetch, which an API-driven delete, a tab/workspace cascade delete, or a crash can all skip."

`prune_block()` was built as the "robust backstop" for exactly that gap — triggered off `Event::BlockDeleted`/`TabDeleted`/`WorkspaceDeleted` — but reading its actual body (lines 706-742), it only prunes the **derived** state: `sessions`, `dispatches`, `pending_activity`, all keyed by `block_id`. It never touches `watched_agents` and never calls `unwatch_agent`. So even when the backstop correctly notices a block was deleted and prunes its existing subagent rows, the underlying filesystem watcher for that agent keeps running — and per Bug A, keeps re-creating fresh, wrongly-attributed rows the next time *any* other shared-path agent (here, the still-live `AgentX`) writes to its own subagent transcript.

### Combined effect

`Agent1`, `Agent2`, `Agent3`, `AgentY`, and `Camper` closed at some point before today (ungracefully enough that `unwatch_agent` never ran for them), leaking five filesystem watchers on the shared Claude config path. `AgentX` is still actively running and spawning subagents. Every write to AgentX's own subagent transcript is broadcast (via the shared-directory `notify` watch) to all six leaked-or-live watchers, each of which relabels it under its own long-dead identity — which is what surfaced in the Swarm/conversation UI as an agent that "appears to be from somewhere else." It has been happening continuously since at least 2026-07-20 (first "backfilling" log line found) and was still firing at the time of writing (2026-07-23T18:35).

---

## 4. Why this isn't the muxbus/cross-tenant issue it initially resembled

Ruled out early and worth stating plainly: every `parent` name involved (`Agent1`, `Agent2`, `Agent3`, `AgentX`, `AgentY`, `Camper`) is a local agent identity on this same machine, in this same srv process's own `db_block`/log history — not an external account, not a WAN/muxbus-delivered identity. `SPEC_MUXBUS_MULTI_TENANT_SECURITY_2026_07_06.md`'s documented `agent_id` authorization gap is real but unrelated to this symptom.

---

## 5. Fix

- **Bug A:** a new `session_belongs_to_block(block_id, session_id)` check — compares the changed file's derived session id against the block's own currently-persisted `agent:sessionid` meta (read live from `wstore`) — gates every live filesystem event in the watch dispatch loop before it's processed.
- **Bug B:** added `parent_block_id` to `WatchedAgent`, plus a new block-scoped `unwatch_block`, called from `prune_block`. Block-scoped (not agent-name-scoped, unlike `unwatch_agent`) so an agent identity reused across multiple blocks over time only loses the watcher tied to the block that actually closed.

Landed alongside `agentmux-srv/src/backend/subagent_watcher/`'s Tier 2 modularization split (#2283) — the fix targets the new `mod.rs`/`types.rs`/`tests.rs` layout rather than the old monolithic file.
