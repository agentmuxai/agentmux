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
- Extracted `watch_agent`'s filesystem-watch setup into a private `start_watch` helper (pure relocation, no behavior change — verified via the full pre-existing 87-test suite passing unchanged).
- Added `recheck_config_dir(agent_id, new_config_dir)`: no-op if the agent isn't currently watched or the resolved directory is unchanged; otherwise tears down the stale watch (dropping its `notify::RecommendedWatcher`) and re-points it at the corrected directory via `start_watch`, preserving every dependent block, then backfills each dependent block's own persisted session (the same scoped mechanism `handle_reactive_register`'s first-registration backfill already uses, to avoid flooding Swarm with every session that identity has ever run).
- Added `recheck_all_watched_agents(id_store, identity_store)`: re-resolves every currently-watched agent's config dir fresh and calls `recheck_config_dir` for any whose answer changed. Cheap — runs only on the rare identity-bind path, never per-turn.

Wired into both production call sites that write an `agent_identity_link` row (`server/app_api/identity.rs`'s account-upsert handler, `server/agent_handlers/identity.rs`'s direct link RPC), right after the write succeeds — the same point each already publishes an `agentidentities:changed:*` WPS event. Reached via the existing `subagent_watcher::global()` singleton accessor (the same pattern `blockcontroller/persistent.rs` already uses to reach this module from elsewhere), so neither identity handler needed `AppState` plumbing changes beyond capturing one more already-existing `Arc` clone.

## Verification

- 5 new tests covering: no-op when unwatched, no-op when unchanged, repoint replaces the entry, repoint preserves multiple dependent blocks, repoint backfills a session's subagents missed while watching the wrong directory.
- Falsified by neutering `recheck_config_dir` to an early return and confirming exactly the 3 behavioral tests fail (the 2 no-op tests correctly still pass — indistinguishable from a stub); restored, diff clean.
- Full pre-existing 87-test `subagent_watcher` suite passes unchanged (confirms the `start_watch` extraction is a pure relocation).
- Full `agentmux-srv` suite: 3014/3014 passing (3002 unit + 5 integration + 7 subprocess-io), zero regressions.
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
| `agentmux-srv/src/server/reactive.rs` | `handle_reactive_register` — the one-shot call site this fix compensates for (unmodified) |
| `docs/specs/SPEC_SUBAGENT_WATCHER_IDENTITY_BOUND_CONFIG_DIR_2026_08_22.md` | The adjacent, earlier fix for the "identity already resolvable at registration" case |
