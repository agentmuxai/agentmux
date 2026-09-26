# Why AgentA Started Blank on the v0.57.5 Build — Incident Report

**Date:** 2026-09-25
**Author:** AgentA (agent, `~/.agentmux/agents/agenta-07017`), at operator request
**Status:** analysis — a point-in-time incident report. Recommendation 1 (§6) ships in the same PR as this report; 2–5 are open.
**Ground truth:** running build `089af6ebe` (release v0.57.5, portable, channel
`local-main-b28b7a-846e847e`); re-checked against `origin/main` `00a12dc09` (2026-09-25 17:15 PDT).
Citations are `file:line` at `089af6ebe` unless marked HEAD. Every claim below was checked
against the live logs, the SQLite stores, and the on-disk transcripts, not against docs alone.
**Related:** `SPEC_DURABLE_CONVERSATION_MEMORY_2026_09_23.md` (§4.2, §4.4),
`REPORT_HISTORY_CONTINUITY_ACROSS_VERSION_UPGRADE_2026_08_17.md`,
`SPEC_IDENTITY_STORE_SPLIT_2026_08_17.md`, `ANALYSIS_PER_CHANNEL_AUTH_BYPASSES_2026_08_31.md`.

---

## 0. Verdict

The operator opened AgentA on a freshly packaged v0.57.5 build. The agent came up with **no memory of
the prior conversation**, while the pane itself reported the session as *continued*. Three
independent failures lined up. Any one of the first two alone makes native resume impossible. The
third is a real bug that removed the designed safety net.

1. **Every local build forces a re-login** (by policy). `scripts/package.sh:103` gives each build
   its own channel. The spawn gate looks the bound account up **only in the current channel's
   store** (`identity/resolver/inject.rs:362`, `:691`), so the account bound in the previous
   channel is "row not found" and the spawn is refused (`inject.rs:702`).
2. **A re-login creates a new account, and history is keyed by account.** The Armory upsert mints
   a fresh UUID for every new login (`server/agent_handlers/identity.rs:224`, no reuse by login
   email). `CLAUDE_CONFIG_DIR` is `identities/<account-id>/claude`, so the new account's
   `projects/` folder is empty. `--resume <old sid>` fails with "No conversation found". In this
   incident the new login was also a *different* Claude account, so the spec deliberately leaves
   native resume off (§4.2, "cross-account native resume stays off").
3. **Bug: the continuation packet (the §4.2 → R3 fallback) was never built.**
   `pane_history_tail` (`persistent/resume_retry.rs:750`) reads the pane's **local** transcript
   whenever it is non-empty (`:756`), and only then falls back to the agent's global zone. On a new
   channel the local file is created by AgentMux's *own* session-outcome line plus the failed
   resume's error result. That tail contains no user turn, so `build_continuation_packet`
   (`backend/continuity.rs:261`) returns `None`. The 1.1 GB global record was never consulted.
   **The pane had already told the user the session was continued (`"continued": true`).**

Nothing on HEAD changes (1) or (3). HEAD's recent continuity/memory work (#3721, #3756, #3797,
#3804, #3812, #3813) carries **memory** and **bundle ids** across accounts and channels, and #3813
records each segment's config dir. None of it relocates transcripts or fixes the packet read path.
`resume_retry.rs` and `continuity.rs` are byte-identical between `089af6ebe` and HEAD.

---

## 1. Timeline (2026-09-25, UTC; log = `channels/local-main-b28b7a-846e847e/versions/0.57.5/logs/agentmuxsrv-v0.57.5.log.2026-09-25`)

| Time | Event | Evidence |
|---|---|---|
| 02:46 → 03:26 | Last real session `49cc350c` under account `0fe0be54` (channel `…-a917d202`, v0.57.3) | transcript mtime, `shared/identities/0fe0be54…/claude/projects/…/49cc350c….jsonl` |
| 22:05:33 | New channel `…-846e847e` created; new ABF bundle `6bd49c3e` minted for AgentA in the channel's `identity-store.db` | `db_bundles.created_at` |
| 22:05:50 | Armory login flow starts for new account `2c5f5e22` | `auth.start (direct-account): OAuth config dir wired` |
| 22:06:31 | Eager resume of `49cc350c` **refused**: `account 0fe0be54… row not found; spawn refused` | `identity.spawn.blocked` |
| 22:06:36 | Account `2c5f5e22` bound to AgentA (`d76da857…`); config dir changes from `shared/providers/claude` to `…/identities/2c5f5e22…/claude` | `armory bind`, `identity rebind changed this agent's Claude config dir` |
| 22:06:37 | Spawn with `--resume 49cc350c`, segment rung `Native` | `continuity: segment started` |
| 22:06:38 | `No conversation found with session ID: 49cc350c…`; id poisoned | `persistent stderr`, `stale --resume session id unreachable` |
| 22:06:39.099 | Local pane `output` file **created** by the session-outcome line `{"attempted_sid":"49cc350c…","continued":true,"outcome":"fresh",…}`, followed by the `error_during_execution` result | channel `filestore.db`, zone `4f17352b…`, `createdts=1790373999099` |
| 22:06:42 | Fresh spawn, rung **`Fresh`** (not `Virtualized`); new session `7e2f6a82` | `continuity: segment started … rung":"Fresh"` |
| — | No `continuity: carrying AgentMux's record…` line anywhere in either day's log | `grep -c` = 0 |
| 22:06:43 | First provider turn is the bare user message `u there`, with no packet | `7e2f6a82….jsonl` |

---

## 2. Failure 1 — per-build channels + per-channel spawn gate = forced re-login

- `scripts/package.sh:103`: `CHANNEL="local-${BRANCH_SLUG}-${BRANCH_HASH}-${BUILD_ID}"`, where
  `BUILD_ID` hashes the build label (SHA + timestamp + PID). The script's own comment says this is
  safe "now that agents + auth are GLOBAL … stays logged in". **That is no longer true** for
  spawning.
- `identity/resolver/inject.rs:362` `resolve_account_for_spawn` reads the current channel's
  `id_store` only. Its doc comment deliberately reverses the PR #2632 global fallback ("per-channel
  isolation is a wanted feature, and re-login-per-channel is the accepted cost").
- Consequence: on each new build, the global link (`shared/identity-store.db`,
  `db_agent_identity_links`: `d76da857… → 0fe0be54…`) points at an account the spawn path refuses
  to see. The account row itself *is* in the global mirror (`shared/identity-store.db`
  `db_accounts`), so the display paths still show it as healthy.

The two design decisions contradict each other. Per-build channels were justified by global auth,
and global auth was later withdrawn for spawns. Neither side's comment was updated.

## 3. Failure 2 — history is keyed by account, and a re-login creates a new account

- `agentmux-common/src/data_paths.rs:370` (`identities_dir`) and the history junction
  (`identity_history_dir`, `link_history_if_isolated`) make the transcript location
  `shared/identities/<account-id>/claude/projects/`. This survives **channel** changes but not
  **account** changes. SPEC_DURABLE_CONVERSATION_MEMORY §1 states this directly.
- `server/agent_handlers/identity.rs:224`: `if account.id.is_empty() { account.id = uuid::new_v4() }`.
  There is no lookup of an existing account for the same provider and login email, so even a
  same-user re-login lands in a new, empty config dir.
- `resume_retry.rs:231` `find_recovery_session_id` and `session_backfill::session_is_reachable`
  only search the **current** config dir. `SPEC_DURABLE_CONVERSATION_MEMORY` §4.2's R2 ("Relocate")
  is not started. The segment log's `config_dir` (made accurate per provider by #3813 on HEAD) is
  consumed only by `memory_reconcile.rs`, not by resume.
- In this incident the old account and the new one belong to **different Claude logins**. By §4.2,
  cross-account native resume stays off (the thinking-signature and prefix-integrity concern is
  [unverified]), so the designed outcome is R3: fresh session + continuation packet. That makes
  Failure 3 decisive.

## 4. Failure 3 (bug) — the continuation packet read the wrong transcript

Path at `089af6ebe`:

1. `persistent/spawn.rs:320`: `fresh_onto_history = attempted_resume_sid.is_none() && self.has_prior_transcript()`.
   `has_prior_transcript` (`resume_retry.rs:147`) is true because the local `output` file is
   non-empty, so `continuation_packet()` is called.
2. `continuation_packet()` → `pane_history_tail()` (`resume_retry.rs:750`). At `:756` it returns
   the local file's tail if one exists and never reaches `global_prior_zone`.
3. The local file was created at 22:06:39.099 by `emit_session_outcome_now_with`
   (`resume_retry.rs:114`), writing the outcome line, plus the held error result. At the moment
   of the 22:06:42 spawn it held no user or assistant turn.
4. `build_continuation_packet` (`continuity.rs:261`) finds no user turn and returns `None`, giving
   rung `Fresh`.

Meanwhile the agent's global zone `agent:d76da857…:current` in
`shared/agents/transcripts/filestore.db` held `output` = **1,165,339,667 bytes** plus
`continuity.state.jsonl` (the rolling summary). The block meta's `agent:sessionZone` pointed at it
and the block was not archived, so `global_prior_zone` would have resolved.

Secondary defect: the session-outcome line was written with `"continued": true` because at
settle time (`resume_retry.rs` `settle_empty_resume_retry` → `fresh_spawn_would_continue`) the
local file was still empty, so *that* check saw the global zone. The act of writing the
disclosure then changed the answer. The UI reported "continued" for a session that received
nothing.

---

## 5. Scope — this is the normal case, not a one-off

AgentA's last eight top-level sessions for this working directory sit under **seven different
account ids** (`shared/identities/<id>/claude/projects/C--Users-area54--agentmux-agents-agenta-07017/`):

| Session | Account | UTC span | Size |
|---|---|---|---|
| `c47ceab2` | `908412be` | 09-17 → 09-18 | 4.1 MB |
| `cabe30a4` | `e580811e` | 09-18 → 09-20 | 27.7 MB |
| `c790f26b` | `e580811e` | 09-20 | 16.4 MB |
| `c9cfb151` | `65c68862` | 09-22 → 09-23 | 15.6 MB |
| `7695ff03` | `120c5afd` | 09-23 | 2.2 MB |
| `6c016670` | `e69a2189` | 09-23 → 09-25 | 1.2 MB |
| `49cc350c` | `0fe0be54` | 09-25 02:46 → 03:26 | 0.7 MB |
| `7e2f6a82` | `2c5f5e22` | 09-25 22:06 → (this session) | — |

Session `7695ff03` (09-23) opens with the operator asking the agent to "find your last
conversation", which is the same failure one rebuild earlier. Older history sits under
`shared/providers/claude/projects/…` (pre-account layout, through 07-22) and further account
dirs back to July.

No transcript was deleted. Everything is on disk, and the global transcript zone has the full
rendered history. The loss is purely one of **reachability** from the new session.

---

## 6. Recommendations (ordered by impact per effort)

1. **Fix the packet read path (Failure 3). Done in this PR.** `continuation_packet()` now reads
   through `packet_history_tail` (`persistent/resume_retry.rs`), which prefers the agent's global
   zone, the system of record per §3 of the durable-memory spec. It falls back to the pane file
   only when the pane has no agent zone, or the zone is empty. The zone mirrors every pane append
   (`emit_session_outcome_now_with` writes both), so this never loses anything the local file had.
   `pane_history_tail`'s local-first order is unchanged for its other caller,
   `pane_history_session_id`. The `continued` flag becomes truthful as a consequence: its
   `fresh_spawn_would_continue()` check and the spawn's own packet now read the same source, so
   the outcome line written in between no longer flips the answer. Regression tests are in
   `persistent/tests/continuation.rs`: the incident's exact local file (outcome line + error
   result), a lone echoed first message, and the no-zone / empty-zone fallbacks.
2. **Stop minting a new account for a same-identity re-login (Failure 2, same-login case).** On a
   successful OAuth login, reuse an existing account row for the same provider and login email
   (the email is already backfilled, #3617), so the config dir and its `projects/` stay stable. A
   same-user rebuild then resumes natively (R1) with no packet needed.
3. **Implement §4.2 R2 "Relocate" for the same account/provider**, using the segment log's
   `config_dir` (now accurate per provider since #3813) to locate the predecessor transcript and
   `--resume <absolute path>` / `CLAUDE_CODE_PROJECT_DIR_NAME` rather than copying.
4. **Resolve the policy contradiction (Failure 1).** Either per-build channels should share an
   auth scope (for example, per-branch rather than per-build for spawn credentials), or
   `package.sh`'s comment and the operator-facing expectation should say plainly that every local
   build requires a fresh Armory login, **and** recommendations 1–2 must hold so that re-login
   costs nothing in continuity.
5. **Make the silent case loud.** When a resume is poisoned *and* no packet is built, the pane
   should say "started without prior context" rather than "continued".

## 7. Immediate recovery for AgentA (operator's choice)

- Read `49cc350c` (and, if needed, `6c016670`/`c9cfb151`) from the paths above and hand the agent
  a summary. This is safe and needs no code.
- Or copy `49cc350c….jsonl` into `…/identities/2c5f5e22…/claude/projects/C--Users-area54--agentmux-agents-agenta-07017/`
  and resume it. Because that session belongs to a different Claude login, this is the §4.2
  **[unverified]** cross-account case, so treat it as an experiment, not a fix.
