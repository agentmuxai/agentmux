# Report: Swarm view floods with stale subagents on reopen

**Status:** Findings 1-3 fixed and merged (#2008). Finding 4 fixed
(frontend-only, workflow grouping) — PR #2018.
**Author:** AgentX
**Date:** 2026-07-07
**Triggered by:** user report while live-testing the four PRs from
`REPORT_AGENT_PANE_STATE_RECONCILIATION_2026_07_07.md` in `task dev`: "in
your task dev, neither the swarm summaries or subagents are showing in
swarm" → clarified to "nm, the summary comes through, not the subagent" →
after the initial fix landed, live-tested again and reported: "when I opened
an agent that had spawned subagents before, the swarm pane floods with old
subagents... we need an architecture rethink, this looks sloppy." → after
Findings 1-2 landed, reported a live agent ("Mazs") that had genuinely just
spawned subagents still showed nothing, which led to Finding 3. → after
Findings 1-3 merged and a fresh build tested, reported the flood *again*:
"if I open a new agent pane with an existing agent, a bunch of old
subagents appear in the swarm... why would that happen?" — Finding 4.

## tl;dr

Four distinct, independently-confirmed issues in `agentmux-srv/src/backend/subagent_watcher.rs` / `agentmux-srv/src/server/reactive.rs`, all stemming from the same root pattern this session's other findings share: state gets (re)constructed at "reopen"/"register" time from raw filesystem/process truth, with no scoping to — or even correct identification of — what's actually *current*.

1. **Subagents never appeared at all** — `watch_agent()` (called from the reactive-register handshake) failed outright and silently gave up when the Claude CLI's config directory didn't exist yet on disk, which is the common case (register fires ~47s before the directory is created).
2. **Subagents flooded in from every past session** — once (1) was fixed, reopening any agent pane whose identity had prior conversations flooded the Swarm view with every subagent that identity had *ever* spawned, in *every* project, across the process's whole history — not just the current session.
3. **The watched directory itself was often just wrong** — `derive_claude_config_dir()` hardcodes `~/.config/claude-<agent_id>`, which only matches reality for an agent with an explicit per-identity bundle override. Any agent without one (the common case — confirmed live for two different test agents) launches under the shared default auth path, `~/.agentmux/shared/providers/claude/`, a completely different subtree. Fix (1) above then watched forever for a directory that was never going to be created at that path, no matter how long it waited.
4. **A single "current" session can itself be unbounded** — Finding 2's fix correctly scopes the backfill to exactly one session (the agent's ongoing "current" session — an AgentMux agent has exactly one, by design, resumed via the picker's reattach flow on every pane open). But that session persists indefinitely across every reopen, so its subagent history only ever grows. Live-confirmed: opening a pane for agent "Loap" replayed **45 subagent-spawned events in under 500ms** — the session's entire lifetime history, dumped in one instant burst, every single time the pane opens.

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

## Finding 3 — `derive_claude_config_dir()`'s path convention doesn't match reality for most agents

### The gap

After Findings 1-2 shipped, a live agent ("Mazs") that had genuinely just spawned subagents via a Task-tool workflow still showed nothing in Swarm. The log told the real story — `~/.config/claude-mazs` had never existed, across *multiple* registration attempts spanning 38+ minutes:

```
14:58:45  WARN failed to watch directory for subagents  dir=...\claude-mazs
15:04:12  WARN failed to watch directory for subagents  dir=...\claude-mazs
15:04:36  WARN failed to watch directory for subagents  dir=...\claude-mazs
15:05:38  WARN failed to watch directory for subagents  dir=...\claude-mazs
15:05:57  WARN failed to watch directory for subagents  dir=...\claude-mazs
15:07:26  WARN failed to watch directory for subagents  dir=...\claude-mazs
```

This isn't a timing race (Finding 1's fix even engaged correctly — "watching nearest existing ancestor instead" — but the ancestor it fell back to, `~/.config`, still never had a `claude-mazs` child appear under it). `derive_claude_config_dir(agent_id)` hardcodes `home/.config/claude-<agent_id>` — a convention that turned out not to be universal. Reading `agentmux-cef`'s `ensure_auth_dir` (the function that actually resolves the CLI's `CLAUDE_CONFIG_DIR` at launch) revealed why: the DEFAULT provider auth dir lives under the account-wide shared root, `~/.agentmux/shared/providers/claude/` — *not* a per-agent `~/.config` path — with a per-identity bundle override taking priority only when one is explicitly configured. `derive_claude_config_dir()`'s guess only ever matches the override case; any agent without one (apparently the common case) was watching a directory tree that was never going to receive the files, no matter how long Finding 1's ancestor-fallback waited.

### Fix

Added `resolve_claude_config_dir(meta, agent_id)`, which reads the block's own `cmd:env.CLAUDE_CONFIG_DIR` — the literal value the CLI process was launched with, written by the launch flow before spawn, from the exact same resolution `ensure_auth_dir` performs — falling back to the legacy `derive_claude_config_dir` guess only when `cmd:env` isn't set yet. `handle_reactive_register` now calls this instead of the raw guess.

### Verification

Live-confirmed post-fix: both test agents ("AgentY", with an identity override, and "Mazs", without one) now resolve to `~/.agentmux/shared/providers/claude/projects` — the real shared path — and the watch succeeds immediately (no ancestor-fallback needed, since the real path already exists). Mazs's pending workflow batch of 13 subagents (all sharing one slug, confirming they're one legitimate concurrent spawn from the *current* session, not a resurfaced flood) appeared correctly as `subagent:spawned` events immediately after.

## Finding 4 — a single "current" session has no lifetime bound on its own history

### The gap

Findings 1-3 shipped and merged (#2008). Live-tested again against a fresh build on latest `main` — the flood came back, this time for agent "Loap", opened via the *normal* agent picker (`MyAgentsList`/`AgentPicker`), not some edge case.

Tracing why: opening an existing agent from the picker is not a fresh launch — it's a **reattach**. `MyAgentsList.tsx`'s own doc comment: "each entry triggers a normal definition launch... with `continueOfInstanceId` + `workDirOverride` set from the row... so Claude's `--continue` (and equivalents) resumes the session." `agent-model.ts:533` writes `"agent:sessionid": continueSid` directly onto the **new** block's meta as part of building the launch config — before the CLI ever spawns, well before `handle_reactive_register` fires. This is by design (`docs` for `META_SESSION_ID`: "under Option E, a user agent has exactly one 'current' session by construction" — session continuity across pane reopens is the intended UX, not a bug).

That means Finding 2's `scan_session_subagents` check (`agent:sessionid` already non-empty at register time) is true on essentially *every* picker reopen of an existing agent — not just the "resuming mid-conversation" edge case it was originally reasoned about. It correctly scopes to exactly one session (verified — no cross-session leak), but that one session is the agent's entire lifetime conversation, with no upper bound on how long it's been running or how much subagent activity it has accumulated.

Live trace, opening agent "Loap" from the picker:

```
22:08:15.690  reactive register request  agent_id=Loap
22:08:15.703  subagent spawned  slug=zesty-crafting-kahan
22:08:15.713  subagent spawned  slug=zesty-crafting-kahan
   ... (45 total "subagent spawned" events) ...
22:08:16.180  subagent spawned  slug=zesty-crafting-kahan
```

45 subagents "spawned" in under 500ms is not live activity — real subagent processes take real wall-clock time to run and complete. This is `scan_session_subagents` replaying Loap's entire current-session subagent history in one instant burst, 13ms after registration. Most share one slug (a single large historical workflow-tool batch, the same "one shared slug = one legitimate concurrent spawn" signature Finding 3 used to confirm Mazs's 13-subagent batch was real) — so this genuinely is the correct session's real history, just a lot of it, replayed with no sense of "how long ago."

### Fix (chosen: group, not truncate)

Considered three candidates: time-bound the backfill (arbitrary threshold, no precedent for what "recent" means for a sporadically-used agent), count-bound the backfill (predictable ceiling, but risks truncating a batch mid-workflow-run), and grouping — collapse subagents spawned together by one Task/Workflow-tool run into a single collapsed row instead of hiding any of them.

Went with **grouping**. It targets the actual complaint (*volume of rows shown at once*, not the data being wrong or stale) without discarding anything — Loap's 45-subagent flood becomes a small number of workflow-group headers (collapsed by default), not 45 individual rows, with zero risk of hiding a subagent that's still relevant. Time/count-bounding remains an option later if a single agent accumulates enough *distinct* (non-grouped) workflow runs or loose subagents to still feel like a flood — not needed for what's been observed so far.

Implementation (frontend-only — `SubagentInfo.workflow_id` was already on the wire, just typed away in `ActiveSubagent`):

- `frontend/app/view/swarm/swarm-model.ts` — added `workflow_id` to `ActiveSubagent`; new `WorkflowGroup` type (`workflowId`, `name`, `subagents`, `activeCount`/`totalCount`, `status: "active" | "retired"`, `lastEventAt`) and `groupSubagentsByWorkflow()`, which partitions a block's subagents into loose (no `workflow_id`) vs. grouped (shared `workflow_id`), computing each group's name from the first member with a non-empty slug (no separate workflow-name concept on the backend) and status from whether any member is still `"active"`. `buildTree()` now calls this instead of returning a flat sorted list.
- `frontend/app/view/swarm/swarm-view.tsx` — new `WorkflowGroupRow` component: header shows name, `X/Y active` or `N retired`, and a status badge; click toggles expand **in place** (no pane navigation) — expanding reveals the member subagents using the existing `SubagentRow`, whose own click-to-open-a-pane behavior is unchanged for individual members. Expand state lives on `SwarmViewModel._expandedIds` (keyed by `workflowId`), not a row-local signal — `buildTree()` rebuilds a fresh `WorkflowGroup` wrapper object on every recompute regardless of whether that group's own data changed, so a local signal silently collapsed on the next unrelated tree refresh (fixed in 4da4e76).
- `frontend/app/view/swarm/swarm-view.scss` — new `.swarm-workflow-*` rules, styled to match the existing `.swarm-subagent-row` hierarchy (members indented one level deeper than a loose subagent row).

### Verification

- `frontend/app/view/swarm/swarm-model.test.ts` (new) — 8 tests: loose subagents pass through ungrouped; shared `workflow_id` collapses to one group; groups and loose rows coexist per distinct `workflow_id`; a group is `"active"` if any member is still active, `"retired"` only once every member has completed; group name derives from the first member with a slug, falling back to the raw workflow id; loose rows and groups sort together by most recent activity.
- `tsc --noEmit` — clean.

## Why this wasn't caught by the earlier report

`REPORT_AGENT_PANE_STATE_RECONCILIATION_2026_07_07.md`'s Finding 2 (subagent completion detection) and this report's bugs live in the same file and the same general area (`subagent_watcher.rs`'s registration/scan path) but are functionally unrelated — Finding 2 was about a broken *completion* signal for subagents already correctly discovered; this report is about *discovery* itself being broken (Findings 1 and 3) and unscoped (Finding 2 of this report). Finding 2's fix in the earlier report (PR #2002) didn't touch `watch_agent()`, `reactive.rs`, or the scan functions at all. All three bugs here were pre-existing and unrelated to any of the four PRs from the earlier report — they only surfaced because this was the first time agent identities with real multi-session history, and a mix of identity-bound vs. shared-default auth configs, were tested against a live `task dev` build with subagent tracking actually exercised end-to-end.

## Verification

- `cargo test -p agentmux-srv subagent_watcher` — 19 tests passing, including:
  - `watch_agent_falls_back_to_nearest_existing_ancestor_when_config_dir_is_missing` (Finding 1 regression test)
  - `scan_session_subagents_only_backfills_the_named_session` (Finding 2 — confirms cross-session isolation)
  - `scan_session_subagents_is_a_noop_for_an_unknown_session_id` (Finding 2 — confirms no flood-prone fallback)
  - `resolve_claude_config_dir_prefers_cmd_env_over_the_legacy_guess` (Finding 3)
  - `resolve_claude_config_dir_falls_back_to_the_legacy_guess_when_cmd_env_is_absent` (Finding 3)
  - `resolve_claude_config_dir_falls_back_when_cmd_env_lacks_the_key` (Finding 3)
- `cargo test -p agentmux-srv reactive` — 52 passing
- Live-verified in `task dev`:
  - Finding 2 against this agent identity's real 20-session history
  - Finding 3 against two live agents with different auth configurations (one identity-bound, one shared-default) — both resolved to their real config dirs and correctly picked up a live subagent spawn (13 subagents, one workflow batch) that had previously shown nothing
  - Findings 1-3 merged (#2008), then Finding 4 found live on the *next* test pass — opening agent "Loap" from the picker replayed 45 subagent-spawned events in under 500ms. Fixed by grouping (#2018) — see "Fix (chosen: group, not truncate)" above.
