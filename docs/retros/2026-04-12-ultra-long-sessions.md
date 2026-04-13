# Ultra-Long Sessions — Verification Report & Retrospective

**Date:** 2026-04-12
**Plan:** `docs/plans/ultra-long-sessions.md`
**Status:** COMPLETE — all four phases shipped to `main`
**Version at completion:** 0.33.100
**PRs:** #336, #338, #340, #341, #342

---

## 1. Scope

Originally written as a four-phase plan to make AgentMux agent panes survive
multi-day sessions with hundreds of thousands of streamed events — without
blowing up RAM, stalling the typing path, or losing history on reconnect.

Four phases as defined in the plan:

| Phase | Theme                         | PR(s)         |
|-------|-------------------------------|---------------|
| 1     | Foundation                    | #336          |
| 2     | Core UX (virtualization etc.) | #338, #340    |
| 3     | Navigation, archival, digest  | #341          |
| 4     | Resilience + edge cases       | #342          |

---

## 2. Verification — what shipped per phase

### Phase 1 — Foundation (PR #336)

| Item                                    | Status | Anchor                                                                 |
|-----------------------------------------|:------:|------------------------------------------------------------------------|
| 1.1 `blockfile:read_range` pagination   | ✅     | `agentmux-srv/src/server/app_api.rs` (read_range + line_count handlers) |
| 1.1 `blockfile:line_count` O(1) fast-path via meta | ✅ | same; backed by `session:line_count`                                  |
| 1.2 WPS ring-buffer cap (`MAX_PERSIST = 4096`) | ✅ | `agentmux-srv/src/backend/wps.rs:53`                                  |
| 1.2 FileStore LRU eviction (128 MB cap) | ✅     | `agentmux-srv/src/backend/storage/filestore/` (cache layer)            |
| 1.3 Write-through to FileStore on stdout| ✅     | `agentmux-srv/src/backend/blockcontroller/shell.rs:918-998`            |
| 1.4 Session metadata (`session:*` meta) | ✅     | `agentmux-srv/src/backend/blockcontroller/session_stats.rs`            |

### Phase 2 — Core UX (PRs #338, #340)

| Item                                     | Status | Anchor                                                                  |
|------------------------------------------|:------:|-------------------------------------------------------------------------|
| 2.1 Virtualization via `content-visibility: auto` | ✅ | `frontend/app/view/agent/components/AgentDocumentView.tsx:208`         |
| 2.2 Paginated history loading on mount   | ✅     | `frontend/app/view/agent/agent-view.tsx` (history load + prepend)       |
| 2.2 Scroll-top "load older" with RAF position restore | ✅ | same                                                             |
| 2.3 Reconnect recovery via history reload| ✅     | same                                                                    |
| 2.4 Timeline minimap w/ 30-bucket density| ✅     | `frontend/app/view/agent/components/TimelineMinimap.tsx`                |
| 2.4 Bookmarks (Ctrl+B)                   | ✅     | `agent-view.tsx:741` (Ctrl+B handler), `agent-view.tsx:901` (read meta) |

### Phase 3 — Navigation + Archival + Digest (PR #341)

| Item                                           | Status | Anchor                                                                |
|------------------------------------------------|:------:|-----------------------------------------------------------------------|
| 3.1 In-session search (Ctrl+F, pane-scoped)    | ✅     | `frontend/app/view/agent/components/AgentSearchBar.tsx`              |
| 3.3 Session archival (gzip → `~/.agentmux/archives/`) | ✅ | `agentmux-srv/src/backend/session_archive.rs` + archiver sweep in `main.rs` |
| 3.3 Archive / Restore / Export RPCs            | ✅     | `agentmux-srv/src/server/app_api.rs` (register_session_*)             |
| 3.3 Frontend buttons in `AgentControlBar`      | ✅     | `frontend/app/view/agent/components/AgentControlBar.tsx`              |
| 3.4 AI session digest via block's own Claude CLI| ✅    | `app_api.rs` (register_session_digest + invoke_cli_for_digest)        |
| 3.4 `SessionDigestBanner` collapsible UI       | ✅     | `frontend/app/view/agent/components/SessionDigestBanner.tsx`          |

### Phase 4 — Resilience (PR #342)

| Item                                           | Status | Anchor                                                                |
|------------------------------------------------|:------:|-----------------------------------------------------------------------|
| 4.1 500K-line warning banner + one-click Archive| ✅    | `AgentControlBar.tsx` (isLargeSession check)                          |
| 4.1 SQLite WAL backpressure (already in place) | ✅     | `agentmux-srv/src/backend/storage/filestore/core.rs:69`               |
| 4.2 Orphaned-session detection (`scan_orphans`)| ✅     | `agentmux-srv/src/backend/blockcontroller/session_recovery.rs`        |
| 4.2 `session:active_pid` + `session:was_interrupted` meta flags | ✅ | same                                                          |
| 4.2 Startup scan hooked in main.rs             | ✅     | `main.rs` (after `heal_all_layouts`)                                  |
| 4.2 Frontend interrupted banner + Dismiss      | ✅     | `AgentControlBar.tsx` (wasInterrupted check)                          |
| 4.2 `--resume <session_id>` path (pre-existing)| ✅     | `agentmux-srv/src/backend/blockcontroller/subprocess.rs:193-209`      |
| 4.3 WPS broker fanout correctness (pre-existing)| ✅    | `wps.rs:221-238` (single-mutex publish)                               |

### Intentionally descoped

| Item                                    | Reason |
|-----------------------------------------|--------|
| 4.1 Per-block FileStore write rate-limit| SQLite WAL + WPS persist ring buffer already provide correct backpressure. Throttling without a demonstrated saturation issue is premature optimization. |
| 4.1 Frame-time-adaptive virtualization buffer | `content-visibility: auto` is already browser-adaptive. No evidence of actual frame-time issues after the DOM-size work in PR #334. |
| 4.3 Explicit snapshot isolation across read_range + live append | Per-operation mutexes are sufficient; explicit isolation would be a large refactor without a demonstrated race. |

---

## 3. What each PR actually delivers in user-visible terms

- **#336 Foundation** — nothing visible yet; unlocks everything else by giving the frontend a paginated read API and a persistent write path.
- **#338 Pagination + reconnect** — refreshing the pane or reconnecting no longer loses history; older events stay addressable.
- **#340 Timeline + bookmarks + segmentation** — tiny density strip on the right edge, Ctrl+B to bookmark the current node, bookmarks persist in block meta.
- **#341 Search + archive + digest** — Ctrl+F opens a pane-scoped search bar. Expanded control bar exposes Archive / Export / Restore buttons. A collapsible banner above the document renders an AI summary of the session on demand.
- **#342 Resilience** — sessions over 500K lines surface a soft warning with a one-click archive action, and sessions that were running when agentmux-srv crashed/rebooted are flagged on next boot with an "interrupted" banner. The very next message auto-resumes via the existing `--resume` path.

---

## 4. Build / review integrity

Every phase PR went through reagent review on opus-4-6 at high effort. Summary:

| PR   | Review rounds | Issues found by reagent                                                                                            |
|------|:-------------:|---------------------------------------------------------------------------------------------------------------------|
| #336 | 1             | clean                                                                                                              |
| #338 | 1             | `nodeIndexMap` not rebuilt on history prepend (fixed with `documentVersion` signal)                                |
| #340 | 1             | `read_range` O(N) memory warning — fixed by making `line_count` use meta fast-path; Ctrl+B was global (pane-scoped) |
| #341 | 3             | (1) stale `home_dir` fallback, (2) meta written *after* FileStore delete (orphan risk), (3) digest prompt via argv (MAX_ARG_STRLEN), (4) `package-lock.json` lockfile drift, (5) tokio child not killed on timeout |
| #342 | 1             | clean (LGTM on first pass)                                                                                         |

All issues were real bugs caught before merge. The reagent loop paid for itself.

---

## 5. Retrospective — what went well, what hurt, what we learned

### Went well
- **Phased rollout caught real issues early.** Phase 1 landing before Phase 2 meant the paginated reads were exercised by the foundation tests before the frontend pagination UI depended on them.
- **Persistent meta flags are a clean pattern.** Using `session:active_pid` + `session:was_interrupted` instead of a new out-of-band IPC surface meant zero new RPC types for Phase 4.2 — the frontend just watches block meta like it does for every other agent feature.
- **Reagent + Opus-4.6 high-effort is finding subtle bugs I would miss.** The MAX_ARG_STRLEN catch on the digest prompt and the tokio child-drop catch are both things I wouldn't have thought about under time pressure. 3 review rounds on PR #341 saved two follow-up PRs.
- **Delegating research to an Explore subagent before planning each phase.** The 400-word "what already exists / what needs implementation" brief let me scope each PR tightly without re-reading entire modules.

### Hurt
- **bump-cli is not syncing `package-lock.json` consistently.** It updated the five Cargo.toml files and `package.json` but left `package-lock.json` stale on every bump this session. Had to manually `npm install --package-lock-only` twice and still hit it in PR #341 review. **Action:** add `package-lock.json` to `.bump.json` targets (or run `npm i --package-lock-only` as part of `bump` post-hook).
- **Nested git clone under `~/.agentmux/agents/agentx/agentmux/`.** A 3.5 GB pre-SolidJS clone was thrashing I/O and confusing agents inside the pane into thinking the project was React. Unrelated to this plan but cost ~30 min. **Action:** add agent workspaces to gitignore OR make the agent startup code refuse to clone into `$AGENTMUX_DATA_DIR/agents/*/repo`.
- **Context-window churn across 5 PRs.** Each PR required re-fetching files that had been summarized away. **Action:** when running a multi-PR plan, write a one-page cheatsheet at the top of each PR that records the exact file paths and line numbers touched so future iterations can skip re-exploration.
- **Review iteration is slow** — each PR took 2–20 min of wall-clock time waiting for reagent. Cumulative wait across 5 PRs was ~1.5 hours. **Action:** none really — this is inherent to the review model, and the catches were worth the wait.

### Surprises
- **content-visibility: auto is enough.** Phase 2.1 was originally planned to need a full virtualized list implementation. Adding `content-visibility: auto` on each node wrapper + removing the hard cap gave us effectively unlimited DOM size at smooth framerate. Saved us from building a virtualizer.
- **--resume was already implemented.** Phase 4.2 originally called out `--resume` support as a deliverable; turned out `subprocess.rs:193-209` had been wiring session_id → `--resume` since an earlier PR. Phase 4.2 became a pure UX layer: scan for orphans, surface to user, let existing resume path do the work.
- **SQLite WAL + WPS ring-buffer already provide backpressure.** Phase 4.1 rate-limiting was descoped once I realized the existing stack handles floods correctly — runaway loops queue in SQLite WAL and get served at disk speed, with WPS ring buffer capping live memory.

### Takeaways for future multi-phase plans
1. Land foundations (RPC types, meta flags, backend plumbing) in one PR before touching frontend. Reviewers can reason about the API without UI noise.
2. Surface every backend state change through existing meta rather than new RPC calls where possible — reuses the subscribe/render path for free.
3. Budget one "reagent round 2" per PR as the default expectation. Round 3 is a signal to split the PR.
4. Always run `bump verify` + spot-check `package-lock.json` before pushing a bumped commit.
5. Describe descoped items explicitly in PR body with the reason. Saves reviewer time and documents the decision for future-you.

---

## 6. Test plan for the portable build

Manual verification in the 0.33.100 portable:

**Phase 1–2 (paginated history + virtualization):**
- [ ] Open an agent pane, run a 10K-line task (e.g. `ls -R /usr`). Confirm typing stays smooth.
- [ ] Close and re-open the pane. History loads from FileStore.
- [ ] Scroll to top. Older history pages in (look for loading indicator + preserved scroll position).

**Phase 2.4 (timeline + bookmarks):**
- [ ] Timeline strip appears on the right edge with density bars.
- [ ] Ctrl+B bookmarks the current node. Bookmark persists across pane reload.

**Phase 3.1 (search):**
- [ ] Ctrl+F opens the search bar only in the focused pane.
- [ ] Typing a query highlights matches; Enter / Shift+Enter cycle.
- [ ] Esc closes.

**Phase 3.3 (archival):**
- [ ] Expanded control bar shows Archive / Export buttons when `lineCount > 0`.
- [ ] Archive moves the session to `~/.agentmux/archives/<blockid>.jsonl.gz` — verify the file exists.
- [ ] Restore brings the session back.
- [ ] Export downloads `session-<blockId>-<ts>.jsonl`.

**Phase 3.4 (digest):**
- [ ] Trigger session digest. Collapsible banner renders with summary text.
- [ ] Regenerate re-invokes the CLI.

**Phase 4.1 (large-session banner):**
- [ ] (optional — requires 500K lines) Warning banner appears with one-click archive.

**Phase 4.2 (interrupted-session recovery):**
- [ ] Start a session, send a message, then kill `agentmux-srv` via Task Manager while the subprocess is running.
- [ ] Restart portable. The interrupted banner should appear above the control bar.
- [ ] Send a new message. The session should resume via `--resume` under the hood.
- [ ] Click Dismiss. The banner should clear.

---

## 7. Follow-ups

Open items that came out of this plan but weren't in scope:

1. **bump-cli package-lock.json drift** — see section 5.
2. **agent nested-repo workspace** — see section 5.
3. **WER dump collection** after any crash — configured but no triggers observed during this plan's testing.
4. **Rate-limit telemetry** — if users ever report runaway-loop saturation, the first step is adding per-block bytes/sec counters to session meta. Not implementing now, but the meta slot is available.
