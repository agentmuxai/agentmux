# Report: Swarm view floods with stale subagents on reopen

**Status:** Root-caused and fixed.
**Author:** AgentX
**Date:** 2026-07-07
**Triggered by:** user report while live-testing the four PRs from
`REPORT_AGENT_PANE_STATE_RECONCILIATION_2026_07_07.md` in `task dev`: "in
your task dev, neither the swarm summaries or subagents are showing in
swarm" → clarified to "nm, the summary comes through, not the subagent" →
after the initial fix landed, live-tested again and reported: "when I opened
an agent that had spawned subagents before, the swarm pane floods with old
subagents... we need an architecture rethink, this looks sloppy."

## tl;dr

Two distinct, independently-confirmed bugs in `agentmux-srv/src/backend/subagent_watcher.rs`, both stemming from the same root pattern this session's other findings share: state gets (re)constructed at "reopen" time from raw filesystem/process truth, with no scoping to what's actually *current*.

1. **Subagents never appeared at all** — `watch_agent()` (called from the reactive-register handshake) failed outright and silently gave up when the Claude CLI's config directory didn't exist yet on disk, which is the common case (register fires ~47s before the directory is created).
2. **Subagents flooded in from every past session** — once (1) was fixed, reopening any agent pane whose identity had prior conversations flooded the Swarm view with every subagent that identity had *ever* spawned, in *every* project, across the process's whole history — not just the current session.

## Finding 1 — `watch_agent()` fails silently when the config dir doesn't exist yet

### The gap

`handle_reactive_register` (`agentmux-srv/src/server/reactive.rs:227`) calls `subagent_watcher.watch_agent(agent_id, block_id, config_dir)` synchronously, as soon as the CLI's registration hook reaches AgentMux. A live trace confirmed the exact timing:

```
14:58:45.363  reactive register request received
14:58:45.364  WARN failed to watch directory for subagents
              dir=C:\Users\asafe\.config\claude-mazs
              error="Input watch path is neither a file nor a directory."
14:59:32.550  persistent process spawned            ← 47 seconds later
```

`watch_agent()`'s old fallback logic only handled `projects_dir` missing (falls back to watching `config_dir`) — it never checked whether `config_dir` *itself* existed. `notify::Watcher::watch()` fails outright on a nonexistent path, and the old code just `return`ed on that error, with no retry. Since `watch_agent()` is only ever called once per agent registration, that agent's subagent tracking was then permanently disabled for the rest of its session — explaining "subagents not showing" cleanly.

### Fix

`watch_agent()` now falls back further: if neither `projects_dir` nor `config_dir` exists, it walks up `config_dir`'s ancestors (`nearest_existing_ancestor()`, new) to find the closest directory that *does* exist (in practice, `~/.config`, which is effectively always present) and watches that instead, recursively — `notify`'s recursive mode still picks up `config_dir`/`projects_dir` once the CLI creates them. Since a shared ancestor could catch other agents' files too, every event handler now filters `path.starts_with(&config_dir)` before processing (previously the filter was purely filename-pattern-based, with no path-prefix check — harmless before since the watched dir was always exactly `config_dir` or a descendant, but now essential).

## Finding 2 — historical scan at register time is unscoped

### The gap

Once Finding 1 stopped `watch_agent()` from failing outright, its trailing step — `scan_existing_subagents(agent_id, block_id, &projects_dir)` — started actually running for the first time in this scenario, and revealed the deeper bug: it walks `config_dir/projects/` (**every** project this agent identity has ever worked in) and, within each, **every** session directory found there, processing every `agent-*.jsonl` file unconditionally. There is no recency filter, no session scoping — every historical subagent this identity has ever spawned gets treated as "currently active" and pushed into `list_active()` / broadcast as `subagent:spawned`.

Verified against real on-disk data for this agent identity:

```
$ find ~/.config/claude-agentx/projects -maxdepth 2 -mindepth 2 -type d | wc -l
20                                                    # 20 past sessions
$ for d in .../subagents; do echo "$d: $(ls $d | wc -l) files"; done
...session.../subagents: 4 files
...session.../subagents: 18 files
...session.../subagents: 5 files
...session.../subagents: 6 files
...session.../subagents: 11 files
```

Reopening a pane for this identity would flood the Swarm view with 50+ stale entries from 20 unrelated past sessions — exactly the reported symptom.

### Fix

Removed the blind scan from `watch_agent()` entirely. Reasoning about what's actually needed:

- A **brand-new session** has nothing to backfill — its subagents don't exist yet, and the live filesystem watcher (unaffected by this fix, still installed unconditionally in `watch_agent()`) correctly picks them up in real time as the Task tool spawns them. No scan needed.
- A **resumed session** (the pane reopens against a session that already has its own subagent history from earlier in *that same conversation*) is the only case with anything legitimate to backfill — and that's a narrow, exactly-scoped case: backfill *that one session's* subagents, nothing else.

`handle_reactive_register` now checks whether the block being registered already has a persisted `agent:sessionid` (block meta key `META_SESSION_ID`, written by `persist_session_id` the first time a session id is captured live, and reused verbatim as the `--resume <uuid>` arg on subsequent spawns — confirmed via the same live trace above). If present, it calls a new `SubagentWatcher::scan_session_subagents(agent_id, block_id, config_dir, session_id)`, which searches only the top level of `config_dir/projects/*` for a child directory literally named `session_id` (the nested-per-session layout, `projects/<ws>/<session-uuid>/subagents/`, confirmed as what's actually on disk) and scans only that one directory. If no persisted session id exists (fresh session), no scan runs at all.

If the on-disk layout is ever flat (no per-session directory — `subagents/` sitting directly under the project dir), `scan_session_subagents` intentionally finds nothing rather than falling back to a broader scan — showing nothing is a smaller failure than reintroducing the flood.

## Why this wasn't caught by the earlier report

`REPORT_AGENT_PANE_STATE_RECONCILIATION_2026_07_07.md`'s Finding 2 (subagent completion detection) and this report's bugs live in the same file and the same general area (`subagent_watcher.rs`'s registration/scan path) but are functionally unrelated — Finding 2 was about a broken *completion* signal for subagents already correctly discovered; this report is about *discovery* itself being unscoped. Finding 2's fix (PR #2002) didn't touch `watch_agent()` or the scan functions at all. Both bugs were pre-existing and unrelated to any of the four PRs from the earlier report — they only surfaced because this was the first time an agent identity with real multi-session history was tested against a live `task dev` build with subagent tracking actually working end-to-end (Finding 1 above had been silently swallowing the whole subagent feature for any freshly-registering agent up to this point).

## Verification

- `cargo test -p agentmux-srv subagent_watcher` — 16 tests passing, including:
  - `watch_agent_falls_back_to_nearest_existing_ancestor_when_config_dir_is_missing` (Finding 1 regression test)
  - `scan_session_subagents_only_backfills_the_named_session` (Finding 2 — confirms cross-session isolation)
  - `scan_session_subagents_is_a_noop_for_an_unknown_session_id` (Finding 2 — confirms no flood-prone fallback)
- Live-verified in `task dev` against this agent identity's real 20-session history.
