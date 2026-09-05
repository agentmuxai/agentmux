# Report: subagent-watcher config-dir resolution races identity binding, permanently, with no correction mechanism

**Date:** 2026-09-04
**Status:** implemented
**Author:** Agent5

## User's request (verbatim, for traceability)

> meanwhile, we also have something you can work on simultaneously and it is introspective to your work. why are your agent tool calls not showing in the swarm?

Followed by, after root-causing: "Implement and ship the fix now."

## Symptom

My own "Agent" tool calls (Claude Code subagent dispatches, materializing as real `agent-<id>.jsonl` transcript files on disk) never appeared in AgentMux's own Swarm view for the entire lifetime of my agent pane — not a rendering bug (`muxspect dock` confirmed the underlying ToolNode/dock data was fine), and not a rare edge case: it affected 100% of my dispatches, for the whole session.

## Root cause, confirmed via live log evidence

`agentmux-srv/src/server/reactive.rs`'s `handle_reactive_register` — the RPC the CLI's own startup hook calls to tell AgentMux "I'm here" — resolves each agent's Claude config directory exactly **once**, calling `subagent_watcher::watch_agent()` a single time per pane, and never again:

```
reactive register request  agent_id=Agent5 block_id=a04e8274...
watching for subagent JSONL files  agent=Agent5 dir=C:\Users\asafe\.agentmux\shared\providers\claude\projects
```

Confirmed directly against my own srv log: this pair of lines appears **exactly once** across my instance's entire history, even though the underlying persistent CLI process restarted several times in between (`ControllerResync` with `forcerestart=true`). The watched directory is a generic, ambient `shared/providers/claude` path — not my actual, currently-live identity-bound config dir (`...\identities\02317200-.../claude`, confirmed directly on the filesystem as where my real subagent JSONL files were being written the entire time).

The chain:
1. My agent launched without a committed identity binding yet (an `agent_identity_link` row that hadn't been written at the moment registration fired) — evidenced by a separate, persistent `"instance ... has empty/blank identity_id"` warning on every subsequent turn.
2. `resolve_bound_oauth_config_dir` (keyed off `instance.definition_id`, not `identity_id` — confirmed by reading it directly, so the blank `identity_id` itself isn't the proximate cause) found no binding yet and returned `None`.
3. `resolve_claude_config_dir`'s fallback chain landed on a **stale `cmd:env.CLAUDE_CONFIG_DIR` snapshot** captured in the block's meta at launch time — before whatever bound my instance to its current identity (most likely the recently-shipped "auto-unblock on external auth, one-click Bind account" flow, given the log's neighboring `"agent failure classified: No account linked"` warning at first-turn time).
4. My CLI's actual runtime environment WAS correctly re-injected with the right identity-bound config dir on every subsequent turn (per `inject_identity_env_with_broker`'s "re-resolved fresh on every turn" design) — but nothing ever told the subagent watcher its one-shot resolution was now stale. `watch_agent`'s own dedup logic means even a hypothetical second `reactive register request` for the same pane wouldn't have helped: an already-watched `agent_id` just adds to `parent_block_ids` and returns immediately, never re-resolving the directory.

This is the same class of gap `SPEC_SUBAGENT_WATCHER_IDENTITY_BOUND_CONFIG_DIR_2026_08_22.md` closed — except that fix only helps when a bound identity *can already be resolved* at registration time. It does nothing for the case where the binding is written moments *after* registration, which is exactly the shape of the auto-unblock/bind-after-failure flow.

## Fix

`agentmux-srv/src/backend/subagent_watcher/mod.rs`:
- Split the old `watch_agent`/`start_watch` into three pieces: `build_watch` (pure construction — creates the `notify` watcher and its dispatch channel, returns `None` on any failure, **no side effect on `watched_agents`**), `start_watch` (first-time registration: build, then install + spawn), and `spawn_consumer_loop` (the debounced file-event dispatch task, shared by both first-time and repoint paths).
- Added `recheck_config_dir(agent_id, new_config_dir)`: no-op if the agent isn't currently watched or the resolved directory is unchanged; otherwise **builds the replacement watch first**, and only if that succeeds does it atomically swap the old entry for the new one — preserving the *exact* original `primary_block_id` and full dependent-block set — then backfills each dependent block's own persisted session.
- Added `recheck_all_watched_agents()`: re-resolves every currently-watched agent's config dir fresh (using `id_store`/`identity_store`, now held on `SubagentWatcher` itself) and calls `recheck_config_dir` for any whose answer changed. Cheap — runs only on the rare identity-bind path, never per-turn.
- `WatchedAgent` gained a `primary_block_id: String` field — the block whose id the live consumer loop actually filters/attributes events against, tracked explicitly rather than re-derived from the unordered `parent_block_ids` set.

Wired into both production call sites that write an `agent_identity_link` row (`server/app_api/identity.rs`'s account-upsert handler, `server/agent_handlers/identity.rs`'s direct link RPC) via `subagent_watcher::global()`, and into `handle_reactive_register` itself (`server/reactive.rs`) right after its own `watch_agent` call — closing the narrowest version of the race, where a bind lands in the exact window between that handler resolving `config_dir` and `watch_agent` installing it.

### Review round: three real findings from Codex, one self-caught design error while addressing them

Codex flagged three issues on the first version of this fix, all confirmed valid:
- **P1** — the resolve-then-install window in `handle_reactive_register` itself wasn't covered; an `agent_identity_link` write landing in that exact window had no later trigger, since `watch_agent` only ever runs once per pane.
- **P2** — `recheck_config_dir` removed the old (working) watch *before* attempting to build the replacement; a transient build/watch failure would have turned a temporary error into a permanent loss of Swarm tracking for that agent.
- **P3** — the repoint picked an arbitrary member of the unordered `parent_block_ids` `HashSet` as the new primary, risking silently attributing subsequent filesystem events to a different pane than before.

Fixing P1 took two attempts. The first version added a post-insert self-check *inside* `watch_agent` itself (re-resolve identity, repoint if changed, right after every install). Running the full pre-existing test suite immediately caught that this was wrong: `watch_agent` is a generic primitive with other callers whose `config_dir` isn't backed by a resolvable instance/binding row at all — the legacy manual `subagent.WatchAgent` RPC entry point (`server/service/misc.rs`) deliberately passes `parent_block_id: ""`, and the self-check's blind re-resolution fell through to `derive_claude_config_dir`'s last-resort fallback and silently repointed the watch away from that caller's own explicit choice, breaking `live_fs_event_with_empty_block_id_bypasses_the_ownership_check` and `live_fs_event_is_not_misattributed_to_a_block_that_does_not_own_the_session`. Relocated the self-check to `handle_reactive_register` instead — the one call site that actually derives `config_dir` from identity/binding resolution in the first place, so a fresh re-resolve there can only ever mean "the DB state changed since I last read it," never "this caller never intended identity resolution to apply here at all."

## Verification

- 7 new tests: no-op when unwatched, no-op when unchanged, repoint replaces the entry, repoint preserves multiple dependent blocks, repoint backfills a session's subagents missed while watching the wrong directory, repoint preserves the *exact* original `primary_block_id` across 6 dependent blocks (not an arbitrary set member), repoint keeps the old watch when the replacement fails to build.
- Falsified three times: (1) neutered `recheck_config_dir` to an early return, confirmed exactly the 3 original behavioral tests fail; (2) reproduced the P2 bug (remove-before-build ordering), confirmed the new "keeps the old watch on failure" test fails; (3) the P1 self-check design error was caught live by the pre-existing suite itself, not a deliberate falsification — see above. Every case restored, diffs clean.
- Full `subagent_watcher` suite: 94/94 (87 pre-existing + 7 new), confirming the three-way split (`build_watch`/`start_watch`/`spawn_consumer_loop`) is behavior-preserving.
- Full `agentmux-srv` suite: 3016/3016 passing, zero regressions.
- `cargo clippy -p agentmux-srv`: no new warnings at any touched line.

## What this does not fix

- The pre-existing, separately-documented "events carry the first registrant's block id" limitation when multiple blocks share one `agent_id` (`watch_agent`'s own doc comment already flags this as rare and out of scope) — a repoint preserves the dependent-block set correctly, but the live consumer loop still only attributes events to whichever one block seeded it, same as before.
- My own two subagent dispatches from *before* this fix shipped remain unrecovered — this fix corrects the watch going forward (and backfills each affected block's *current* session once the bind event fires again), not historical dispatches from a session that already ended without ever triggering a rebind event.
- Does not address why my instance ended up with a blank `identity_id` in the first place (flagged in its own log line as `"Legacy row or UI regression?"`) — that's a separate, narrower question about launch-flow/DB consistency, not the subagent-watcher's own resolution logic.

## Files

| File | Role |
|---|---|
| `agentmux-srv/src/backend/subagent_watcher/mod.rs` | `start_watch` extraction, `recheck_config_dir`, `recheck_all_watched_agents` |
| `agentmux-srv/src/backend/subagent_watcher/tests.rs` | 5 new tests |
| `agentmux-srv/src/server/app_api/identity.rs` | Call site 1 (account upsert/bind) |
| `agentmux-srv/src/server/agent_handlers/identity.rs` | Call site 2 (direct link RPC) |
| `agentmux-srv/src/identity/resolver/inject.rs` | `resolve_bound_oauth_config_dir`, `resolve_claude_config_dir`'s fallback chain (unmodified, read for diagnosis) |
| `agentmux-srv/src/server/reactive.rs` | `handle_reactive_register` — call site 3 (the P1 self-check, closing the resolve-then-install window) |
| `agentmux-srv/src/bootstrap.rs`, `agentmux-srv/src/main.rs` | Threading `id_store`/`identity_store` into `SubagentWatcher::new`/`spawn` and `spawn_background_subsystems` |
| `docs/specs/SPEC_SUBAGENT_WATCHER_IDENTITY_BOUND_CONFIG_DIR_2026_08_22.md` | The adjacent, earlier fix for the "identity already resolvable at registration" case |
