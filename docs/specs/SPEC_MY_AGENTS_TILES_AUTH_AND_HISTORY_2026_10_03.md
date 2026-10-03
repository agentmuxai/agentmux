# SPEC: My Agents tiles — larger tiles, an honest auth line, real history summaries, and no ambient credentials

**Date:** 2026-10-03
**Status:** active — Phase 1 (#4285) and Phase 2a (#4290) built, see §11; Phase 2b and 3 not started; §8 lists the decisions that block Phase 4.
**Author:** AgentY, at the owner's request
**Related:** `docs/reports/REPORT_AGENT_PICKER_FIELD_ORDER_SORT_AND_DATA_GAPS_AUDIT_2026_08_24.md` (§4 the "(ambient creds)" regression, §5 the snapshot states, §5a the Haiku fallback), `SPEC_AGENT_PICKER_TILE_GRID_2026_06_17.md`, `SPEC_AMBIENT_SWARM_SUMMARY_HARDENING_2026_10_02.md`, `PLAN_LOGIN_SINGLE_PATH_CONSOLIDATION_2026_07_20.md` §7 (the retired ambient-login escape hatch), `SPEC_ACCOUNT_DELETE_DEAUTH_LAYERS_2_4_2026_07_14.md`, `docs/retro/RETRO_DEV_BUILD_SHARED_AGENT_SESSION_COLLISION_2026_07_29.md`.

## 1. The request (owner, 2026-10-03)

In the agent pane's **My Agents** list ("MY AGENTS 20"):

1. The tiles should be **2x the size** they are now.
2. If an agent isn't bound to an auth account, the tile says **"No auth"**; if it is bound, it shows the account. There should be **no "(missing account)" and no "(ambient creds)"**. If there is an actual ambient-credential implementation, **sack it**.
3. The **ambient text summaries must be more robust**. Every tile reads "(history may exist in another version)" although every one of these agents has a history.

"Ambient" means two unrelated things here: *ambient credentials* (a launch that uses whatever login is lying around, §4) and the *ambient summary* (a model-written one-line description of what an agent is doing, §3). This spec keeps them apart.

## 2. How a tile is built today

One backend call fills the list: `ListRecentSessionsCommand`, handled in `crates/srv/src/server/agent_handlers/session.rs` (`register()`), returning `RecentSessionRow` (`crates/srv/src/backend/rpc_types/instance.rs:164-231`). The frontend is `frontend/app/view/agent/components/MyAgentsList.tsx`; tile styling is `frontend/app/view/agent/styles/_recent-sessions.scss`.

| Element | Where it comes from |
|---|---|
| Name | `instance_name \|\| definition_name` |
| Account line | `identity_name`, built at `session.rs:386-409` |
| Preview line | `preview` from a per-block snapshot, else `definition` summary, else the fallback text from `noSnapshotText` (`MyAgentsList.tsx:146-150`) |
| "N messages" | `node_count`, from the same snapshot |
| Timestamps | `started_at`, `last_active_at` |

### 2.1 Size

The list is a grid: `repeat(auto-fill, minmax(min(var(--agent-tile-min, 240px), 100%), 1fr))` with an 8 px gap, `--agent-tile-min: 240px` (`_picker.scss:24`). A tile has `padding: 8px 30px 8px 10px`, a 24 px icon, a 13 px name, an 11 px account line, a **single-line** 12 px preview (`nowrap` + ellipsis, no line clamp), and 10 px meta lines. No `min-height`: height is whatever the content needs, and grid rows stretch to the tallest tile.

### 2.2 The account line

`session.rs:392-408` produces one of four things, never an empty string:

- real account names, joined with `, `;
- **"(missing account)"**: the agent has a link row, but that link's `account_id` is not in the account list the handler loaded;
- **"(unknown account)"**: the links lookup itself failed (`identity_links_source_failure_uses_distinct_fallback_text_not_ambient_creds`, `session.rs:958`);
- **"(ambient creds)"**: no link row for the agent or its template. The frontend repeats the same literal for an empty string (`MyAgentsList.tsx:1068`). `AgentLaunchModal.tsx:534` has its own copy for its "Continue an existing agent" dropdown, and `agent_handlers/identity.rs:770-785` builds the same literals for `listnamedagents`.

**Why "(missing account)" appears on a whole list.** Links are read from `identity_store`, which is always the global store. Accounts are read from `id_store`, which is the global/shared store only when it is attached and migration `0011_shared_store_backfill` has run, and otherwise falls back to the per-channel store (`bootstrap/stores.rs:431-474`). In an **isolated** channel (a dev build, `isolated_auth_enabled()`), nobody is signed in, so `db_accounts` is empty while the global links exist. Every linked tile then reads "(missing account)". Other display code (`identity/resolver/inject.rs:321-331`, `resolve_account`) falls back to the global store; the list handler does not. A second cause: `identity_list` (`storage/identities.rs:237-296`) silently skips any account whose `secret_ref` JSON fails to parse, so that agent also reads "(missing account)" and nothing is logged.

The owner's screenshot was of a dev build (an isolated channel), so the first cause is the likely one for the "(missing account)" tiles. **Not verified** which cause applies to which tile; Phase 1 adds the log fields that settle it.

## 3. Why every tile says "(history may exist in another version)"

That text is chosen by `noSnapshotText` when `snapshot_check_failed` is false and `block_id_hint` is empty. Three separate problems stack up, any one of which would blank the preview:

1. **No per-block snapshot is written any more.** The frontend now persists a lightweight overlay under the agent's *global* zone `agent:<definition_id>:current` (`useSnapshotPersistence.ts:114`, `AgentSessionWriteStateCommand`). Nothing in `frontend/app` writes the per-block `output.state.json` any more, yet the list handler still stats `<block_id>/output.state.json` (`session.rs:436`). So `has_snapshot` is false for any agent launched by a current build.
2. **The snapshot it would read has no messages.** `read_session_preview` (`agent_handlers/mod.rs:94-150`) needs a top-level `nodes` array to find the first user message. The current overlay is under 1 KB and deliberately has no `nodes` (`useSnapshotPersistence.ts:61-67`). Even a found snapshot gives `preview = ""` and `node_count = 0`.
3. **The Haiku fallback never runs for these rows.** `db_agent_activity_summaries` is written by `generate_definition_activity_summary` (`ambient/tasks.rs:139-222`), but the call site (`session.rs:477`) requires a non-empty `block_id` and reads the per-block raw output (`ambient/digest.rs:66-93`). A row with no `block_id` can never get one. It also runs **once per definition and is cached forever**, so a summary goes stale the moment the agent does more work.

`block_id_hint` is empty for a row that (a) came from the shared registry with no matching local instance (a synthetic "available" row, `session.rs:233-249`), or (b) is a local row never launched in *this* channel (`last_block_id` is only set by `instance_record_launch`, `storage/agents.rs:2198-2216`). In the owner's dev build both are common, because a dev channel has its own `objects.db` and sees the main channel's agents only through the registry.

**The history itself exists and is not channel-bound.** `~/.agentmux/shared/agents/transcripts/filestore.db`, zone `agent:<db_agents.id>:current` (archives `…:archive:<ms>`), holds the raw `output` NDJSON, a `output.tsidx` receive-time sidecar and the overlay `output.state.json` (`agent_session/zone_naming.rs:21-27`, `agent_session/global_store.rs`). It is keyed by agent id, written by every channel, and survives pane close. A second source, `HistoryService` (`backend/history/mod.rs`, `sessions_for_owner` :453-545), indexes the providers' own session files and gives `first_user_message`, `message_count`, `modified_at` per session. **The list handler reads neither.**

## 4. Is there an ambient-credential implementation?

Yes, in two senses: a dead one, and several live ones. Findings, from reading the source (nothing was run):

**Dead (safe to remove).**

- The `use_ambient_login` column and field: stored, round-tripped and mirrored (`storage/agents.rs:142`, `rpc_types/agent.rs:511`, `def_registry_mirror.rs:61/:121`, `core.rs:260`) but read only for a log line (`inject.rs:593`). The gate comment says it "no longer has any effect on the outcome" (`inject.rs:641-655`), and a test confirms it (`inject.rs:2106`). The UI toggle is gone (`AgentIdentityModal.tsx:18-30`).
- Stale documentation that still describes ambient as live: `identity/mod.rs:10-18` ("the agent inherits ambient credentials", "warn-don't-block"), `inject.rs:58-62`, `frontend/types/rpc/AgentDefinition.ts:88/:106`, and the `"Empty = ambient creds"` wording on `AgentRef`, `AgentInstance.identity_id`, the registry schema and the create-from-template and create-instance RPC types (`agents/types.rs:25`, `rpc_types/instance.rs:61`, `storage/agents.rs:316`, `registry/schema.rs:41`, `rpc_types/agent.rs:330`).

**Live: no account bound, launch proceeds.**

| Path | What happens | Where |
|---|---|---|
| Create-from-template modal | An "(ambient credentials)" option, and `canSubmit` does not require an account, so an agent can be created unbound | `AgentCreateFromTemplateModal.tsx:429`, `:283-286` |
| API-key providers (`muxcode`, `qwen`, `pi`, `kimi`) | No backend gate (`provider_class` returns `None`, `identity/resolver/provider.rs:89`), so they launch on whatever the CLI finds. Every catalog provider is `oauth` or `api-key`; none is credential-free | `providers/catalog.ts`, `inject.rs` |
| Oauth-class providers | **Already refused** at spawn when unbound (`gate_oauth_failure`, `inject.rs:656-671`); the UI blocks it too (`useLaunchAuthGate.ts`). `antigravity` is `oauth` in the catalog but missing from the backend class list, so it has no backend gate | `provider.rs:89`, `catalog.ts:507` |
| `/btw` side questions | Run on ambient credentials *even when the source pane's agent is bound*; the code records this as a known limitation | `side_question.rs:40-48` |
| Quick-launch / template panes with no agent row | `inject_identity_env` returns `Ok(())` with nothing injected | `inject.rs:511-527` |
| Background ambient summaries with no live pane | Run the CLI with an empty auth env | `ambient/tasks.rs` (`provider_fallback_target`) |

So "ambient creds" in the list today does **not** mean the agent launches ambiently (an oauth agent with no link is refused). It just means "no link row".

## 5. Requirements

### 5.1 Larger tiles

R1. Tiles are twice the size: **width ×2** (`--agent-tile-min` 240 → **480 px**; in a narrower pane the existing `min(…, 100%)` makes it one full-width column) and **height ≈ ×2**, from a larger icon, more padding and a multi-line summary (R6), not from stretching empty space.

R2. Proposed scale: icon 24 → 40 px; padding 8/30/8/10 → 14/40/14/16; name 13 → 15 px; account 11 → 13 px; summary 12 → 13 px, up to **3 lines** (a `-webkit-line-clamp: 3`, which the tile-grid spec already calls for and the code never did); meta lines 10 → 11 px. Tiles in one row keep equal height.

### 5.2 The auth line

R3. The line shows **"No auth"** when the agent has no bound, resolvable account, and otherwise the bound account's name (several names joined with `, ` as now). The literals "(missing account)", "(ambient creds)" and "(unknown account)" no longer exist, in the list, in `AgentLaunchModal.tsx`, or in `listnamedagents`.

R4. "Bound" means a link whose account **resolves**, looking in the per-channel store first and then the global store, the order `resolve_account` already uses (`inject.rs:321-331`). That removes the isolated-channel false "(missing account)". A link whose account is genuinely gone (deleted, or an unparseable `secret_ref`) reads **"No auth"**, which is true: the agent cannot launch on it. The unparseable-row skip is logged (`warn`, agent id and account id), not silent.

R5. If the links lookup itself fails (`degraded` contains `identity_links`), the tile shows **no auth line** rather than "No auth", because the state is unknown. The list-level degradation handling is unchanged.

### 5.3 History previews

R6. Each tile shows, in order: the agent's **ambient summary** (what it was last doing, up to 3 lines); else the **last user message** of its most recent conversation; else, only when the agent truly has no conversation, **"No conversations yet"**. "(history may exist in another version)", "(no conversation snapshot)" and "(no user message yet)" are removed. A failed history read shows "Couldn't read history" (the one honest error state, replacing "(couldn't check for history)").

R7. Preview, message count and last-active time come from the agent's **global transcript zone** (`agent:<definition_id>:current`), keyed by agent id, so they work for a closed pane, an agent never launched in this channel, and a registry-only row. `HistoryService` is the fallback for an agent with no transcript zone. The list never depends on `block_id` or on a per-block snapshot.

R8. The read is bounded: a stat plus a **tail range read** (the last N KB of `output`) to find the latest user message, memoized by `(zone, size, modified-at)`; it never loads a whole transcript, and runs on the blocking pool. The list can ask for 100 rows (`SEARCH_LIMIT`), so cost per row must stay small and constant.

R9. The ambient summary is generated **from the global transcript**, not from `<block_id>/output`, for any agent that has one, so it no longer needs a pane. It is refreshed when the transcript has grown since the summary was written, at most once per agent per hour and only while the list or Swarm is being viewed, so a long-idle agent costs nothing. It runs under the agent's **own** bound account (R11), and an agent with none is skipped, not run on whatever login is around.

### 5.4 No ambient credentials

R10. Phase 3 (safe): remove every dead path in §4 — stop reading and writing `use_ambient_login`, drop it from the RPC and TypeScript types, delete the "(ambient credentials)" option and the "Empty = ambient creds" wording, make `canSubmit` in the create-from-template modal require an account when the provider needs one, and correct `identity/mod.rs` and the other stale comments. The database column stays (a migration to drop it is separate and not urgent); nothing reads it.

R11. Phase 4 (needs the §8 decisions): every launch and every model call made on an agent's behalf uses that agent's bound account or does not run. That covers `/btw`, the ambient summary calls, the API-key providers and `antigravity`.

## 6. Design

**Frontend** (`_recent-sessions.scss`, `MyAgentsList.tsx`): R1/R2 are CSS tokens plus the line clamp. `noSnapshotText` and the `"(ambient creds)"` fallback go; the row shows `identity_name` when present (the backend now sends `"No auth"` or an empty string for "unknown").

**Backend, account line** (`session.rs:386-409`): resolve each link's account through a small helper that tries `id_store` then the global store, the way `resolve_account` does; build `"No auth"` when no link resolves; leave `identity_name` empty and keep `degraded` when the links source failed. Mirror the same helper in `agent_handlers/identity.rs:770-785`.

**Backend, history** (`session.rs:420-602`): replace the per-block `output.state.json` stat and `read_session_preview` with `read_agent_history_summary(definition_id)` over the global transcript store (`global_transcript_store()`), returning `{ last_user_message, message_count, last_activity_ms }` from the tail read. Keep the old per-block read only as a fallback for a channel with no global store (unit tests). Drop the `block_id` gate on the summary fallback and feed `generate_definition_activity_summary` from the same transcript tail.

**Summary cadence** (`ambient/tasks.rs`, `storage/agent_activity_summaries.rs`): add a `source_size` or `source_ms` column so "stale" is a comparison, not a guess; at most one refresh per agent per hour; the existing once-per-process claim stays so a list refetch never fans out calls.

## 7. Phases

| Phase | What | Risk |
|---|---|---|
| 1 | Larger tiles; "No auth"; account lookup with global fallback; remove the three account literals; log which cause (store mismatch, malformed row, no link) each non-resolving link had | low, display only |
| 2 | History from the global transcript; the new preview states; bounded tail read; summary refresh from the transcript | medium: new read path on a hot RPC; needs the perf test in §9 |
| 3 | Remove dead ambient-credential code and wording (R10) | low |
| 4 | R11: bind `/btw`, summaries, API-key providers and `antigravity` to the agent's account | **breaks launches that work today** (§8) |

Phases 1-3 are independent of Phase 4 and can ship in that order.

## 8. Decisions for the owner

1. **Size.** Is "2x" right as width ×2 and height ≈ ×2 with a 3-line summary (R1/R2), or should the text scale 2x as well? A 26 px name would look odd, so I assumed not.
2. **A failed account lookup (R5).** Show no line, as proposed, or something visible like "Auth unavailable"? "No auth" would be wrong because the state is unknown.
3. **Phase 4 breaks things.** Requiring a bound account for the API-key providers (`muxcode`, `qwen`, `pi`, `kimi`) would stop any of those agents that work today on a login the CLI finds by itself. Do any of your agents rely on that? Same question for `/btw` from an unbound pane. My recommendation: make Phase 4 refuse, with a message saying which account to bind, and leave plain terminals (no agent row) alone, since a terminal running the user's own shell is not an agent launch.
4. **Summary cost.** Summaries are model calls on your account. One per agent per hour while viewed is my proposal; with 20 agents that is at most 20 small calls an hour. Is that acceptable, or should it be on-demand only?

## 9. Tests

1. Account line: a linked account in the per-channel store → its name; linked only in the global store (isolated channel) → its name, not "(missing account)"; link to a deleted or unparseable account → "No auth", plus a `warn`; no link → "No auth"; links source failed → empty line with `degraded`. No test or output contains the three removed literals.
2. History: an agent with a global transcript and no block id gets its last user message, message count and last-activity time; a registry-only row gets the same; an agent with no transcript and no provider session → "No conversations yet"; a transcript read error → "Couldn't read history".
3. Bounded read: a 200 MB transcript is previewed by reading only the tail (assert bytes read), and 100 rows complete within a time budget on the test store.
4. Summary: generated from the transcript with no pane; skipped for an agent with no bound account; refreshed only when the transcript grew and an hour has passed; never twice concurrently for one agent.
5. Tile CSS: min width 480 px, summary clamped to 3 lines, equal heights per row (a DOM test on the class and computed style).
6. Phase 3: grep-style test that `use_ambient_login` is not read or written outside the migration, and the create-from-template modal cannot submit an unbound oauth-provider agent.
7. Phase 4 (once decided): `/btw` from a bound pane uses that account; from an unbound pane it refuses.

## 10. Not in this spec

Dropping the `use_ambient_login` column (a migration, no behaviour); the Swarm view's use of `agent:summary` (`SPEC_AMBIENT_SWARM_SUMMARY_HARDENING_2026_10_02.md`); the agent history browser itself.


## 11. As built

**Phase 1 (#4285):** tiles at 480 px minimum width with a 3-line preview, `No auth` for an unbound agent, an empty line (not `No auth`) when the links or accounts lookup itself failed, the three retired account labels removed.

**Phase 2a (history previews):** `agent_session/history_summary.rs`. For each distinct agent id in the list, `read_agent_history` stats `agent:<id>:current/output` (falling back to the newest `agent:<id>:archive:<ms>` that has output, found with a primary-key range scan, `FileStore::zone_ids_with_prefix`), then reads backwards from the end in 1 MiB chunks, at most 4 MiB, until it finds the newest real user prompt. A real prompt is a `user` record that is not a tool result, has no `parent_tool_use_id`, and does not start with harness scaffolding (`# Session Context`, `[JEKT:`, `[BROADCAST:`, `<system-reminder>`, and similar). The result is remembered by `(store, zone)` until the file's size or modified time changes. It runs on the blocking pool, once per distinct agent id per list call.

Row states: a prompt is found → `preview` is that prompt, `has_snapshot` true, `last_active_at` is the transcript's modified time. A transcript with no prompt in the last 4 MiB → `has_snapshot` true, empty preview, shown as "No prompt in recent history". No transcript → the old per-block snapshot is consulted (old builds wrote it), else "No conversations yet". A failed read → `snapshot_check_failed`, shown as "Couldn't read history", and `transcript` is added to `degraded`.

Differences from §5.3:
- **No message count.** Counting messages would mean reading the whole transcript (a 1.5 GB file today), so `node_count` is 0 and the tile shows no count. The picker still sorts by last activity.
- **`HistoryService` fallback not built.** An agent with no transcript zone reads "No conversations yet" even if a provider session file exists. Add it if that turns out to matter.
- **Summary-first order not built.** The preview is the last user message; the ambient summary (R9) is Phase 2b, because it needs the refresh policy and the per-account rule to land together.
