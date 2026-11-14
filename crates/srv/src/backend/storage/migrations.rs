// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! SQL schema setup for Store, FileStore, and the saga log.
//!
//! `objects.db` uses a **flat schema**: `run_object_schema` defines the
//! final table set directly in one idempotent `CREATE TABLE IF NOT EXISTS`
//! batch. It replaced an 11-step incremental migration chain
//! (`run_forge_v1` … `run_forge_v11`) — see
//! `docs/specs/SPEC_SCHEMA_FLATTENING_2026_05_19.md`. The chain was pure
//! historical accretion: per-version data dirs mean every new version is
//! born with a fresh `objects.db` and ran the whole chain top-to-bottom
//! anyway, so the intermediate states were never reachable in production.
//!
//! `filestore.db` and `sagas.db` were already single-DDL stores; they keep
//! their existing schema functions and gain only the `user_version`
//! tripwire (`check_schema_compat` + `stamp_version`).

use rusqlite::Connection;
use tracing::warn;

use super::error::StoreError;

mod object_schema;
mod store_schema;

pub(in crate::backend::storage) use object_schema::apply_additive_columns;

/// `user_version` stamped into `~/.agentmux/shared/store.db` after `run_shared_store_schema`.
/// Versioned independently from `objects.db`.
///   v1 — initial: identity accounts/bundles/bindings/links, memory bundles, drone definitions,
///        muxbus credentials
///   v2 — db_cron_jobs: persistent scheduled injection jobs
///   v3 — rename db_identity_accounts -> db_accounts, db_memory_bundles -> db_bundles
///        (SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md Phase 4a,
///        SPEC_ARMORY_PHASE4_STORAGE_RENAME_COMPLETION_2026_07_12.md)
///   v4 — db_agent_credentials: per-agent M2M Cognito client_id/secret + cached access token, one
///        row per locally-registered agent_id, so cloud_subscriber's /reactive/* calls use a
///        credential bound to the agent, not the shared per-account MUXBUS_TOKEN
///   v5 — db_cron_jobs.max_age_secs: optional hard expiry (seconds since created_at) beside
///        max_fires; NULL = no expiry
///        (docs/specs/SPEC_AGENT_POLLING_AND_WAKEUP_HARDENING_2026_08_04.md Phase 0)
///   v6 — db_agent_native_memory: durable mirror of each agent's native memory files, keyed by
///        (agent_id, filename) (docs/specs/SPEC_NATIVE_MEMORY_DURABLE_SYNC_2026_08_07.md)
///   v7 — db_bundles.instructions_by_provider: JSON {provider_id: content} variants, additive to
///        the flat `instructions` column (ABF v0.2 §2.2,
///        docs/specs/SPEC_ABF_V0_2_PROVIDER_AWARE_COMPONENTS_AND_NATIVE_MEMORY_2026_08_10.md)
///   v8 — db_agent_native_memory_versions: append-only history, one row per
///        `agent:memory:write_file` call (plus reverts and, later, out-of-band-write detection);
///        the v6 mirror only holds the current value
///        (docs/specs/SPEC_MEMORY_VERSION_CONTROL_AND_ARMORY_AUDIT_2026_08_19.md)
///   v9 — db_bundles.is_system: schema parity with objects.db v27 (the AgentMux-controlled Global
///        Bundle tier)
///   v10 — db_bundle_versions: append-only Global Memory entry history, schema parity with
///        objects.db v35 (SPEC_AGENT_FACING_GLOBAL_MEMORY_API_2026_09_15.md)
///   v11 — db_cron_jobs.target_uid: the target agent's UID (`db_agents.id`) beside the slug column,
///        resolved once at create time and dual-written; nothing reads it yet, a job still fires by
///        `target`. Non-interactive ingress never resolves a name at fire time, so the UID is
///        captured while a human or model is present; empty when the name did not resolve, never
///        guessed (identity M1b, SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED_2026_09_23.md §5.4, §9.1)
///   v12 — db_cron_jobs.created_by_uid and db_bundle_versions.written_by_uid: the actor's UID
///        beside each actor-name column, from the request's `Caller` (its `X-Agent-Token`), never a
///        body field; empty for an Unattributed request, never guessed. Dual-written, not yet read
///        (identity M4c-1, SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED_2026_09_23.md §6.5.9)
pub const SHARED_STORE_SCHEMA_VERSION: i64 = 12;

/// `user_version` stamped into `objects.db` after `run_object_schema`. The flat schema reset
/// the counter to 1 (the pre-flatten chain never set `user_version`, so legacy files read 0).
/// Bumped per additive migration:
///   v1 — flat schema baseline
///   v2 — db_agent_definitions.updated_at
///   v3 — db_agent_definitions.user_hidden (Phase 2 hide templates,
///        SPEC_AGENT_PICKER_TWO_TIER_2026_05_24.md Q2 Decision Y)
///   v4 — db_agents consolidation table (Phase 3a: every definition/instance write dual-writes
///        here). Reads moved over piecemeal: the instance side in Phase 3b (#3080), every
///        `agent_def_*` method in Phase 3d (#3092, v31); both legacy tables were dropped in v32
///   v5 — db_agents.last_block_id (Phase 3c; the latest launch's block, so the consolidated read
///        finds the session snapshot without joining db_agent_instances)
///   v6 — container_image / container_volumes / container_name on db_agent_definitions and
///        db_agents (SPEC_CONTAINER_PANE_SUPPORT_2026_06_11.md Phase 0; host agents default to '' /
///        '[]' / '')
///   v7 — db_muxbus_credentials: global singleton for MuxBus cloud Cognito PKCE tokens (access,
///        refresh, id) + expiry + user email
///   v8 — db_memory_bundles.is_global: global-tier flag for Armory bundles injected into every
///        agent's CLAUDE.md at launch
///   v9 — db_memory_bundles.sort_order: explicit order of the Armory global bundles (the CLAUDE.md
///        injection order); existing rows default to 0, the Armory Global section assigns positions
///        via reorder
///   v10 — db_skills, db_mcp_servers, db_agent_skills_ref, db_agent_mcp_ref: standalone MCP Server
///        and Skill primitives with per-agent ref tables
///        (SPEC_V1_MCP_SKILLS_PRIMITIVES_2026_06_30.md)
///   v11 — rename db_identity_accounts -> db_accounts, db_memory_bundles -> db_bundles
///        (SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md Phase 4a,
///        SPEC_ARMORY_PHASE4_STORAGE_RENAME_COMPLETION_2026_07_12.md)
///   v12 — use_ambient_login on db_agent_definitions and db_agents: per-agent opt-in to the CLI's
///        global (ambient) login when no oauth-class account resolves at spawn
///        (SPEC_ACCOUNT_DELETE_DEAUTH_LAYERS_2_4_2026_07_14.md §2.3, layer 3). Defaults to 0 (fail
///        by default); a since-retired data migration set it to 1 for pre-existing linkless agents
///   v13 — db_agent_credentials: per-agent M2M muxbus credential, mirroring db_muxbus_credentials
///        in both schemas: id_store falls back to this per-channel schema on installs that haven't
///        applied 0011_shared_store_backfill, so cloud_subscriber must not assume the shared-schema
///        copy (v4 there) is in use
///   v14 — db_agent_native_memory: durable mirror of each agent's native memory files, keyed by
///        (agent_id, filename); duplicated in both schemas for the same id_store fallback reason as
///        v13 (docs/specs/SPEC_NATIVE_MEMORY_DURABLE_SYNC_2026_08_07.md)
///   v15 — model_vendor_base_url on db_agent_definitions and db_agents: points a harness (CLI) at a
///        non-default model vendor backend (e.g. ANTHROPIC_BASE_URL for a claude-provider agent),
///        the data side of treating harness and model vendor as distinct. Defaults to '' (the
///        harness's default vendor); settable only via `agent.define`, validated against the
///        provider's `ProviderConfig::base_url_env_var`
///   v16 — db_bundles.instructions_by_provider: JSON {provider_id: content} variants, additive to
///        the flat `instructions` column (which stays "default")
///        (docs/specs/SPEC_ABF_V0_2_PROVIDER_AWARE_COMPONENTS_AND_NATIVE_MEMORY_2026_08_10.md
///        §2.2). Defaults to '{}'. Delivering a variant into a running agent is out of scope (that
///        spec's non-goals): the column only makes the data storable, exportable and importable
///   v17 — auto_continue_enabled on db_agent_definitions and db_agents: the Warden Supervisor's
///        per-agent auto-continue opt-in (default 0). RETIRED with the Warden Supervisor: nothing
///        reads or writes it. It stays because older builds sharing the data dir still select it
///        (docs/analysis/ANALYSIS_WARDEN_AUTO_CONTROLLER_CONTINUATION_WATCHER_2026_08_12.md)
///   v18 — db_agent_jekt_keys: per-agent HMAC-SHA256 signing key for host-tier jekt sender
///        verification. One row per agent_id, minted on first use and injected only into that
///        agent's own MCP server env (AGENTMUX_JEKT_KEY), so a signature can only be produced by
///        the agent it claims to be from (docs/specs/SPEC_JEKT_TRUST_LAYER_COMPLETION_2026_08_13.md
///        §2.2)
///   v19 — db_agent_definitions.memory_id: the agent's own dedicated ABF bundle, set once at
///        creation and never updated (agent_def_update's SET clause omits it, like
///        is_seeded/slug/parent_id). Distinct from db_agent_instances.memory_id (a launch's bundle,
///        which may diverge). Mirrored into db_agents as a SEPARATE default_memory_id column,
///        because db_agents.memory_id is instance-only by convention (see db_agents' CREATE TABLE
///        comment). Defaults to ''; the m0021 migration backfills every unbound definition with a
///        fresh bundle (ARCHITECTURE_MANDATORY_ABF_RETHINK_2026_08_14.md §3.1)
///   v20 — db_agent_lan_keys: per-agent Ed25519 keypair for LAN-tier jekt sender verification,
///        mirroring v18 but asymmetric: LAN is multi-party, so a receiving peer must verify without
///        being able to forge. public_key is distributed to LAN peers on demand (not secret);
///        private_key goes only to that agent (its MCP config's AGENTMUX_LAN_KEY and its own
///        token-authenticated key fetch) (docs/specs/SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md §2.1)
///   v21 — db_lan_peer_pubkey_pins: trust-on-first-use pin of a REMOTE agent_id's LAN public key as
///        first seen from mDNS-discovered peers (#2588). Discovery is unauthenticated: any device
///        can announce an agent under an existing name, so trusting whichever peer answers first
///        lets an attacker's self-minted key be accepted for a victim's agent_id. Pinning the
///        first-seen key and rejecting a later mismatch is the SSH-host-key mitigation for "no PKI
///        available"; it narrows the spoofable case to a race on the first lookup
///        (docs/specs/SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md §2.2)
///   v22 — db_background_tasks: durable record of a declared long-running (`run_in_background:
///        true`) task attached to a block, so its liveness survives `DockSnapshotCache`'s 1-hour
///        eviction and a session reconnect/reload; the client-transcript-derived signal had no
///        durable source of truth. Per-channel (block ids mean something only within their
///        channel), not the shared store. Renumbered from v20 after merging with the LAN-tier
///        signing PR, which claimed v20/v21 independently
///        (docs/status/STATUS_ATTACHED_TASK_AXIS_AND_DEV_LOOP_2026_08_15.md, #2492)
///   v23 — db_bundle_skills_ref, db_bundle_mcp_ref: bundle-level references to the standalone
///        skill/MCP-server primitives, mirroring v10's agent-level ref tables but keyed by
///        bundle_id: composable model v2, step 1
///        (docs/specs/SPEC_BUNDLE_AS_CONTAINER_V2_2026_08_17.md, #2024 item 3). A bundle's
///        mcp_servers/skills JSON columns are untouched (still the .abf export/import format);
///        these tables are the live-resolution path. Deliberately NO FK to db_bundles (unlike the
///        agent-level tables' FK to db_agent_definitions); see their CREATE TABLE comment
///   v24 — db_agent_native_memory_versions: append-only version history for native memory content,
///        duplicated across schemas like the v14 table (id_store per-channel fallback)
///        (docs/specs/SPEC_MEMORY_VERSION_CONTROL_AND_ARMORY_AUDIT_2026_08_19.md)
///   v25 — db_agent_groups: saved, named target sets for fleet control (broadcast/bulk-stop across
///        many agent panes; docs/specs/SPEC_MULTI_AGENT_FLEET_CONTROL_2026_08_20.md): a durable
///        list of block ids under a name, since a saved group scales as a fleet grows where a raw
///        ephemeral selection doesn't. Per-channel (block ids), not the shared store. Renumbered
///        from v24 after colliding with the native-memory-version PR (the same collision class as
///        v20 to v22)
///   v26 — db_agent_definitions.conversation_visibility (TEXT, default 'private') +
///        db_conversation_trust_grants: `muxspect` Phase B/C's per-agent disclosure policy for an
///        incoming cross-tier `transcript_request` jekt
///        (docs/specs/SPEC_MUXSPECT_CROSS_TIER_CONVERSATION_VISIBILITY_2026_08_21.md; tier rules in
///        docs/specs/SPEC_JEKT_TRANSCRIPT_REQUEST_TIER_RULES_2026_08_22.md). One of "private"
///        (auto-deny), "trusted_peers" (auto-approve an allow-listed requester), "ask" (force human
///        escalation). Also dual-written to `db_agents`, like `auto_continue_enabled` (a simple
///        per-agent setting, unlike `memory_id`/`model_vendor_base_url`, which use a
///        differently-named column there). Deliberately NOT in `DefinitionRecordV1`'s cross-channel
///        wire format: channel-local, so a cross-channel-reopened agent starts at "private".
///        `db_conversation_trust_grants` mirrors `db_lan_peer_pubkey_pins`' shape (own module,
///        get/set methods, case-insensitive agent_id lookups)
///   v27 — db_bundles.is_system: an AgentMux-controlled, highest-priority Global Bundle tier
///        (INTEGER, default 0). A system row is always also is_global=1 (enforced in code, not a
///        CHECK; see `bundle_upsert_system`). Writable only through the
///        `upsertsystemmemory`/`deletesystemmemory` RPCs and
///        `Store::bundle_upsert_system`/`_delete_system`; the generic
///        `bundle_upsert`/`_delete`/`_reorder` refuse an is_system=1 row. Injected first in
///        `format_global_bundle_block`'s output, wrapped in explicit override wording, ahead of
///        every ordinary Global Bundle section
///        (docs/specs/SPEC_GLOBAL_MEMORY_SYSTEM_TIER_2026_08_24.md)
///   v28 — db_agent_activity_summaries: one-shot, on-demand Haiku-generated activity summary per
///        `definition_id`, the AgentPicker's "My Agents" conversation-preview fallback for legacy
///        rows with no structured `output.state.json` snapshot (see `has_snapshot` in
///        `listrecentsessions`, `agent_handlers/session.rs`). Generated from the instance's raw
///        `"output"` filestore file through the shared Ambient Model Call gateway (`invoke_haiku`,
///        docs/specs/SPEC_AMBIENT_MODEL_CALLS_FRAMEWORK_2026_07_03.md), lazily (first picker load
///        that needs it) and cached forever once non-empty, like `SubAgent.display_name`, not
///        `term:ambient_summary`'s per-turn refresh. Renumbered from v27 after colliding with the
///        system-tier PR (#2782)
///        (docs/reports/REPORT_AGENT_PICKER_FIELD_ORDER_SORT_AND_DATA_GAPS_AUDIT_2026_08_24.md §5a)
///   v29 — db_agents.session_id / status / started_at / ended_at: the agent's LATEST launch state,
///        the four `db_agent_instances` columns the consolidated row never got (Phase 3a left them
///        "on the block", but the recent-sessions picker, the pane close/reopen write-back and the
///        status-filtered instance list kept reading the legacy table, which is why it could not be
///        dropped). One row per agent means one launch state per agent; a continuation moves it to
///        the new block/session. Backfilled by `m0025_agents_launch_state_backfill` (latest
///        instance per projection key) and kept current by the instance dual-write; the Phase 3b
///        read flip (SPEC_AGENT_ARCHITECTURE_2026_05_27.md) retires the legacy reads on top of it.
///        No unique index on `db_agents(slug)`: template-launch projections copy their template's
///        slug, so uniqueness stays a definition-level rule enforced by the slug collision scan in
///        `agent_def_insert`
///   v30 — `db_agent_content` / `db_agent_skills` / `db_agent_history` / `db_agent_identity_links`
///        / `db_agent_skills_ref` / `db_agent_mcp_ref`: `agent_id`'s foreign key now targets
///        `db_agents(id)`, not `db_agent_definitions(id)`. Existing rows already satisfy it (each
///        `agent_id` is a definition id with a `db_agents` mirror at the same id); what changes is
///        that a `db_agents` row with no definition counterpart (a template launch) can now hold
///        these directly, closing the gap `listrecentsessions`' template fallback (Phase 3b, #3080)
///        exists for. SQLite can't redefine an FK in place, so
///        `m0028_agent_child_tables_repoint_fk` rebuilds the six tables (rename, recreate, copy,
///        drop, `PRAGMA foreign_key_check` before commit); a fresh install gets the new FK from the
///        CREATE TABLE statements below (Phase 3c, SPEC_AGENT_ARCHITECTURE_2026_05_27.md)
///   v31 — No DDL change. Every `agent_def_*` write now lands on `db_agents` only (Phase 3d,
///        #3092); `db_agent_definitions` is no longer kept in sync. Bumped anyway:
///        `check_schema_compat` is a downgrade guard as much as a schema guard, and this release
///        changes what a NEWER binary's writes mean to an OLDER one. Without the bump, rolling back
///        one binary would silently accept a store where new agents exist only in `db_agents`, and
///        that binary's `agent_def_get`/`update`/`delete` (still on `db_agent_definitions`) would
///        404 or replay stale legacy values into `db_agents` on edit. Stamping 31 makes that
///        downgrade refuse to open
///   v32 — `db_agent_definitions` and `db_agent_instances` are dropped (Phase 3e,
///        SPEC_AGENT_ARCHITECTURE_2026_05_27.md, closing out the tracking spec): nothing has read
///        or written either since v31 (the definition and instance flips, #3080/#3092). Their
///        `CREATE TABLE`s and indexes are gone from this function, because migrations run before it
///        on every boot and leaving them would recreate empty tables after the drop.
///        `db_agent_definitions`' `ALTER TABLE ADD COLUMN` entries are KEPT, gated on the table
///        still existing (see that block's comment below): several pending migrations (m0007,
///        m0025, m0026, m0028) call `Store::open()`, running this function, before m0029 drops the
///        table in the same upgrade pass, and an old store missing one of those columns would fail
///        their reads with "no such column" (#3096). An existing store's tables are removed by
///        `m0029_drop_legacy_agent_tables`; a fresh install never has them.
///        `adopt_legacy_table_names`' `db_forge_agents` → `db_agent_definitions` rename stays on
///        purpose: an install old enough to carry `db_forge_agents` reaches it in an EARLIER
///        migration's `Store::open()`, so the rename still fires before m0029 drops what it
///        produced, and no data is stranded under the old name
///   v33 — `db_agent_project_instructions`: what each agent was last observed to read as project
///        instructions from its working directory, keyed `(agent_id, path)` (Phase 3 of
///        SPEC_INSTRUCTION_AND_MEMORY_PORTABILITY_2026_09_09.md). Records a content hash and an
///        observation time, never the content: a tracking table, not a mirror. Unlike
///        `db_agent_native_memory`, which keeps content because it is a durable copy of files
///        AgentMux may have to serve when the originals are unreachable, these files belong to the
///        repository, are always readable from it and are never written by AgentMux; storing them
///        would make a second copy of somebody else's file with no one to keep it honest. Purely
///        additive: nothing reads it at launch, and an agent with no row has no prior observation
///        to compare against
///   v34 — `db_agent_skills_ref` / `db_agent_mcp_ref` / `db_bundle_skills_ref` /
///        `db_bundle_mcp_ref` drop their `skill_id` / `mcp_id` FK to this store's LOCAL `db_skills`
///        / `db_mcp_servers` (Phase 2 of SPEC_DURABLE_BINDINGS_2026_09_10.md §5.4, #3182). Once
///        catalog writes redirect to `identity_store`, the local catalog copies stop gaining rows,
///        so binding a post-redirect skill/server would hit `FOREIGN KEY constraint failed`
///        (`PRAGMA foreign_keys=ON` in production, see `Store::configure_and_migrate`). The
///        `agent_id → db_agents` FK on the agent-level pair stays; existence moves to the
///        application layer (`managed_bind_agent` / `managed_bind_bundle`'s `catalog: &Store`
///        check), as `db_bundle_skills_ref` / `db_bundle_mcp_ref` already did for `db_bundles` (see
///        their CREATE TABLE doc below). SQLite can't drop a constraint in place, so
///        `m0032_drop_catalog_fk_from_ref_tables` rebuilds an existing store's four tables; a fresh
///        install gets the trimmed FK from the CREATE TABLE statements below
///   v35 — db_bundle_versions: append-only version history for Global Memory entry content (`name`
///        + `instructions`), one row per write through the agent-facing Global Memory API
///        (docs/specs/SPEC_AGENT_FACING_GLOBAL_MEMORY_API_2026_09_15.md Phase 0). Same shape as
///        `db_agent_native_memory_versions`, keyed by a single `bundle_id` instead of `(agent_id,
///        filename)`. Global Memory (`db_bundles`) had no audit trail, only `updated_at`, and is
///        about to gain a write path an agent (not just a human in the Armory UI) can reach.
///        `written_by` is a TRUSTED identity column (an agent's real `AGENTMUX_AGENT_ID`, or
///        `"armory-ui"`), kept separate from the caller-describable `source` / `source_detail`
///        pair, which would let an agent claim `source: "human"` with nothing to contradict it
///        (#3237)
///   v36 — db_agent_wan_keys: per-agent Ed25519 keypair for agent-to-agent WAN-tier jekt signing
///        (#2586, second half). Mirrors v20's shape but is a SEPARATE keypair from
///        `db_agent_lan_keys`: LAN public keys are permanently TOFU-pinned by peers (v21), so they
///        cannot rotate (SPEC_JEKT_HOST_KEY_TTL_ROTATION_2026_09_14.md §4), while WAN keys are
///        served from muxbus's ownership registry and are meant to; one key cannot have both
///        lifecycles. `key_version` is reserved for the publish-sync and rotation phases (that
///        spec's §3.2/§3.6) and unread so far
///   v37 — db_agent_tokens: per-agent local identity token, keyed by the
///        agent's UID (`db_agents.id`) — identity M1 of
///        `SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED_2026_09_23.md` (§6, §9.1).
///        Minted on first spawn (`agent_token_ensure`), injected into that
///        one agent's process env as `AGENTMUX_AGENT_TOKEN`, revoked on
///        delete via `purge_agent_dependents`. Unlike v18's jekt key and the
///        registry entry files, which are keyed by display name and so are
///        shared by two same-named agents (spec §6.3), this is keyed by a
///        `PRIMARY KEY` that is a UUID for every non-template row. Read
///        since M4: it identifies a request's Caller (host-tier sender
///        verification, the agent's own key fetch). See `storage/agent_tokens.rs`.
///   v38 — db_agent_key_tombstones: the lowercased names (slug, display
///        name, instance name) of every deleted agent — identity M4a of
///        `SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED_2026_09_23.md` §6.5.4.
///        Signing keys are keyed by name, so a later agent that takes a
///        deleted agent's name would inherit its keys; M4d copies name-keyed
///        keys to UIDs and must never copy a tombstoned name's. Written from
///        M4a on, before any copy exists, so the record is complete when M4d
///        reads it. Written only — nothing reads it until M4d.
///   v39 — db_bundle_versions.written_by_uid: the writer's UID beside the
///        `written_by` name, taken from the request's `Caller` (its
///        `X-Agent-Token`), never from a body field; empty for an
///        Unattributed write, the Armory UI and the seeders. Dual-written,
///        not yet read. Not only parity with the shared store: when that
///        store is unavailable `id_store` falls back to this one and writes
///        here. Identity M4c-1 of
///        `SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED_2026_09_23.md` §6.5.9.
///   v40 — db_jekt_held: a jekt to a known agent of this channel that no
///        delivery tier could take (the agent was not running anywhere), held
///        for up to 24 h and replayed when it registers, with the trust
///        verdicts it was accepted with as explicit columns.
///        `SPEC_DURABLE_JEKT_DELIVERY_2026_09_24.md` Phase 1.
///   v41 — db_background_tasks.owner_tool_use_id: the `tool_use_id` of the
///        Agent call whose subagent launched this background task (the
///        `parent_tool_use_id` on the stream line that issued the Bash call);
///        NULL for the agent's own tasks. Joins a task to its subagent, whose
///        `meta.json` records the same id. Phase 2 of
///        `SPEC_BACKGROUND_TASK_STRUCTURED_FEED_AND_SWARM_OWNERSHIP_2026_09_27.md`.
///   v42 — db_agents.last_runtime: the Runtime menu choices (permission mode,
///        model, effort) the user last made for this agent, as a JSON object,
///        '' when none. Lets Continue / Reattach / a fresh open start where
///        the user left off instead of on the definition's defaults. Written
///        only when the user picked something. Item 2 of
///        `SPEC_RUNTIME_MENU_REMAINING_GAPS_2026_10_01.md`.
///   v43 — db_viewer_devices: the devices paired with this channel's viewer
///        listener, each with the SHA-256 of its viewer token (never the
///        token), its name, its optional public key and when it was paired
///        and last seen; plus db_agents.hide_from_devices, an agent's opt-out
///        from the viewer routes. agentmux-mobile's
///        SPEC_AGENT_STATUS_AND_LIVE_PANE_FEED_2026_10_07 §13.2, §4.3. See
///        `storage/viewer_devices.rs`.
///   v44 — db_mcp_server_reach: per catalog MCP server, whether it reaches
///        every agent (`all_agents`) and whether container agents get it
///        too (`in_containers`). A table of its own beside `db_mcp_servers`,
///        so nothing that copies catalog rows changes. Both default 0: a
///        catalog server reaches only the agents it is bound to.
///        `SPEC_CONNECTORS_MCP_WHAT_AGENTS_RUN_2026_10_10.md` §2.1.
///   v45 — db_lan_pin_installs: the install whose certificate vouched for a LAN
///        name pin's key (instance id and host hint), kept beside the pin so a
///        key moves only on that install's say; and db_lan_retired_keys: the
///        fingerprints a name has moved off, with when and by whom, so a message
///        signed with one is refused as retired. Tables of their own beside
///        `db_lan_peer_pubkey_pins`, so that table's rows and readers are
///        unchanged. See `storage/lan_peer_pin_installs.rs`.
pub const OBJECT_SCHEMA_VERSION: i64 = 45;
/// `user_version` value stamped into `filestore.db`.
pub const FILESTORE_SCHEMA_VERSION: i64 = 1;
/// `user_version` value stamped into `sagas.db`.
pub const SAGA_LOG_SCHEMA_VERSION: i64 = 1;

/// Object type table names matching the `db_<otype>` convention.
const WSTORE_OTYPES: &[&str] = &[
    "client",
    "window",
    "workspace",
    "tab",
    "layout",
    "block",
    "temp",
];

/// Legacy `objects.db` table names retired by the de-forge rename, paired
/// with their replacement. `adopt_legacy_table_names` renames any of these
/// it finds — the single surviving piece of the old migration chain (it
/// also subsumes the v11 `db_memories` rename).
const LEGACY_TABLE_RENAMES: &[(&str, &str)] = &[
    ("db_forge_agents", "db_agent_definitions"),
    ("db_forge_content", "db_agent_content"),
    ("db_forge_skills", "db_agent_skills"),
    ("db_forge_history", "db_agent_history"),
    ("db_forge_agent_identities", "db_agent_identity_links"),
    ("db_memories", "db_memory_bundles"),
    // Phase 4 of SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md — the
    // "bundle" naming collision fix (SPEC_ARMORY_PHASE4_STORAGE_RENAME_COMPLETION_2026_07_12.md).
    // Order matters: this entry runs AFTER the `db_memories` rename above
    // in the same pass, so a database old enough to still have `db_memories`
    // chains straight through to `db_bundles` in one migration run instead
    // of stopping at the intermediate name.
    ("db_identity_accounts", "db_accounts"),
    ("db_memory_bundles", "db_bundles"),
];

/// Legacy index names that must be dropped after their table is renamed —
/// `ALTER TABLE … RENAME` keeps indexes attached but under their old names,
/// which would collide with the flat DDL's `CREATE INDEX`. The flat DDL
/// recreates each under the new name.
const LEGACY_INDEX_DROPS: &[&str] = &[
    "idx_forge_agents_slug",
    "idx_forge_history_agent_date",
    "idx_forge_agent_identities_account",
    "idx_memories_is_blank",
    // Phase 4 rename (see LEGACY_TABLE_RENAMES above) — objects.db names,
    // then the shared store.db names (`idx_ss_*` prefix, see
    // run_shared_store_schema).
    "idx_identity_accounts_provider",
    "idx_memory_bundles_is_blank",
    "idx_ss_identity_accounts_provider",
    "idx_ss_memory_bundles_is_blank",
];

/// Tables retained by the old chain only for a downgrade path the flatten
/// abandons. Dropped from any legacy DB by the adopt step; never created
/// by the flat schema. `db_workflow_*` data was already copied into
/// `db_drone_*` by the old v10 migration, so dropping loses nothing.
///
/// `db_identities` (the pre-v11 name), `db_identity_bundles`, and
/// `db_identity_bindings` were dropped outright rather than renamed
/// forward in Phase 4c of SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md —
/// `db_agent_identity_links`/`db_accounts` is the sole credential-
/// resolution path (`identity/resolver/inject.rs::resolve_bindings_for_instance`),
/// confirmed by the direct-link backfill migrations, since retired
/// (see SPEC_ARMORY_PHASE4_STORAGE_RENAME_COMPLETION_2026_07_12.md §6/§8a).
const DEAD_TABLE_DROPS: &[&str] = &[
    "db_workflow_definitions",
    "db_workflow_runs",
    "db_v10_migrated_legacy_defs",
    "db_v10_migrated_legacy_runs",
    "db_identities",
    "db_identity_bundles",
    "db_identity_bindings",
];

/// Initialize (or re-validate) the full `objects.db` schema.
///
/// Idempotent — safe on every srv startup. Steps:
///
/// 1. `adopt_legacy_table_names` — renames any pre-flatten forge/bundle
///    tables found (protects dev databases created before the flatten;
///    see the spec §3/§7) and drops the dead workflow/sentinel tables.
/// 2. The flat `CREATE TABLE IF NOT EXISTS` batch — the canonical schema.
/// 3. Seeds the blank Bundle singleton row.
///
/// A database stuck at a pre-v11 intermediate schema cannot be fully
/// adopted (its tables predate later columns). The adopt step still
/// renames what it finds; the first query referencing a missing column
/// then fails loudly with `no such column` — a hard error, not silent
/// empty state, with the data preserved on disk. Per-version data dirs
/// make that case unreachable for released builds.
pub fn run_object_schema(conn: &Connection) -> Result<(), StoreError> {
    adopt_legacy_table_names(conn)?;

    object_schema::create_generic_tables(conn)?;
    object_schema::create_input_history_tables(conn)?;
    conn.execute_batch(object_schema::OBJECT_TABLES_SQL)?;
    object_schema::add_object_columns(conn)?;
    object_schema::add_legacy_definition_columns(conn)?;
    object_schema::add_agent_columns(conn)?;
    object_schema::seed_blank_bundle(conn)?;
    object_schema::create_channel_migrations(conn)?;

    Ok(())
}

/// Rename any pre-flatten `objects.db` tables to their de-forged names and
/// drop the dead workflow/sentinel tables. Idempotent — on a fresh or
/// already-flat database every check is a no-op.
///
/// This is the single surviving fragment of the old v1–v11 chain: it
/// exists only to carry a developer's pre-flatten `objects.db` (always at
/// the post-v11 schema, since v11 is merged) forward without data loss.
/// SQLite ≥ 3.25 auto-updates foreign-key references in child tables when
/// a parent table is renamed, so the agent/identity cascades survive.
fn adopt_legacy_table_names(conn: &Connection) -> Result<(), StoreError> {
    for (legacy, current) in LEGACY_TABLE_RENAMES {
        if !crate::migrations::table_exists(conn, legacy)? {
            continue;
        }
        if crate::migrations::table_exists(conn, current)? {
            // Both present — only reachable on a deliberate
            // downgrade-roundtrip dev DB (flat build → pre-flatten build,
            // which re-creates the legacy name → flat build again). The
            // flatten abandons the downgrade path, but the legacy table
            // may hold rows the downgraded build wrote. Do NOT drop it —
            // that would be silent data loss (the bug class behind PR
            // #933's Codex P1). Leave it on disk and warn loudly so the
            // developer can recover or delete it manually.
            warn!(
                legacy_table = *legacy,
                current_table = *current,
                "objects.db has both a legacy table and its de-forged \
                 replacement — this only happens after a downgrade to a \
                 pre-flatten build; the legacy table is left untouched \
                 for manual recovery and is otherwise unused",
            );
        } else {
            conn.execute_batch(&format!("ALTER TABLE {legacy} RENAME TO {current};"))?;
        }
    }

    // Drop indexes orphaned by the renames — the flat DDL recreates them
    // under the new names.
    for idx in LEGACY_INDEX_DROPS {
        conn.execute_batch(&format!("DROP INDEX IF EXISTS {idx};"))?;
    }

    // Drop tables retained only for the abandoned downgrade path.
    for table in DEAD_TABLE_DROPS {
        conn.execute_batch(&format!("DROP TABLE IF EXISTS {table};"))?;
    }

    Ok(())
}

/// Initialize (or re-validate) the `~/.agentmux/shared/store.db` schema.
///
/// Contains only the durable user content that must survive across
/// channels/versions: identity accounts, agent→account links, memory
/// bundles, drone definitions, and muxbus credentials. Session state
/// (`db_block`, `db_tab`, etc.) and drone run history stay in the
/// per-channel `objects.db`.
///
/// `db_agent_identity_links` drops the FK to `db_agent_definitions` here
/// (cross-DB FK enforcement is impossible in SQLite); application code
/// enforces referential integrity instead.
///
/// Idempotent — safe to call on every startup.
pub fn run_shared_store_schema(conn: &Connection) -> Result<(), StoreError> {
    // Rename any legacy table names (including the Phase 4 accounts/bundles
    // rename — LEGACY_TABLE_RENAMES entries that don't apply to this
    // connection's tables are safely skipped) before the idempotent
    // CREATE TABLE IF NOT EXISTS batch below, same as run_object_schema.
    adopt_legacy_table_names(conn)?;

    conn.execute_batch(store_schema::SHARED_STORE_TABLES_SQL)?;
    store_schema::seed_shared_blank_bundle(conn)?;
    store_schema::add_shared_store_columns(conn)?;

    Ok(())
}

/// `user_version` stamped into the permanently-global `~/.agentmux/shared/identity-store.db` (a
/// different file from `store.db`; see [`run_identity_store_schema`]). Independent counter from
/// `SHARED_STORE_SCHEMA_VERSION`.
///   v1 — initial: agent→account links, bundles, drone definitions, muxbus credentials, per-agent
///        M2M credentials, native memory mirror, cron jobs: everything `run_shared_store_schema`
///        held EXCEPT `db_accounts` (docs/specs/SPEC_IDENTITY_STORE_SPLIT_2026_08_17.md §3.1)
///   v2 — db_accounts, as a FALLBACK MIRROR only (#2632): a link resolves via this store, but the
///        account row it points to could exist only in the per-channel-isolatable `id_store`, empty
///        on a fresh channel, so the bug wasn't fixed end to end without it. `id_store` stays the
///        PRIMARY, authoritative write path (Armory's disposable-test-account isolation is
///        unchanged); this copy is consulted only when `id_store` lacks the row (see
///        `identity::resolver::inject`'s account resolution: read-with-fallback plus write-back to
///        the source store)
///   v3 — db_agent_native_memory_versions, for parity with the v1 native memory mirror this store
///        carries. Not an actively written duplicate either (`id_store` / `store.db` stay
///        authoritative, see v2); it is here so a future always-global read path, like v2's for
///        db_accounts, isn't blocked on a missing table
///        (docs/specs/SPEC_MEMORY_VERSION_CONTROL_AND_ARMORY_AUDIT_2026_08_19.md)
///   v4 — db_bundles.is_system, for parity with OBJECT_SCHEMA_VERSION v27 /
///        SHARED_STORE_SCHEMA_VERSION v9 (same reasoning as v3: not an actively written duplicate).
///        A real ALTER TABLE ADD COLUMN in `run_identity_store_schema`, so the counter MUST bump
///        with it: an unbumped counter leaves an existing identity-store.db at the old
///        `user_version` forever, defeating `check_schema_compat` / `stamp_version`'s
///        forward-compat lock for this store (#2782;
///        docs/specs/SPEC_GLOBAL_MEMORY_SYSTEM_TIER_2026_08_24.md)
///   v5 — `db_work_queue`, the Muxqueue backlog
///        (docs/reports/REPORT_UNIVERSAL_AGENT_WORK_QUEUE_2026_09_01.md). A new table, no ALTER,
///        but the counter still bumps: an identity-store.db stamped v4 that a v5 binary opens must
///        read as older, not "already current" (the #2782 reasoning of v4). Deliberately in THIS
///        store, not the per-channel one: a queue with per-channel rows means "any agent IN THIS
///        CHANNEL can pick it up", close to useless when every local/dev/portable build is its own
///        channel (the report's §6.2;
///        docs/analysis/ANALYSIS_PER_CHANNEL_AUTH_BYPASSES_2026_08_31.md for what the
///        per-channel/global seam has already cost)
///   v6 — `db_skills`, `db_mcp_servers` (Phase 2 of SPEC_DURABLE_BINDINGS_2026_09_10.md): the
///        AUTHORITATIVE catalogs from this version. The per-channel copies in `run_object_schema`
///        keep their declarations (never removed; that spec's §5.1 and §8 item 3) and remain only
///        the best-effort degraded-mode fallback `bootstrap/stores.rs` substitutes (`mstore` for
///        `identity_store`) when the identity store can't be resolved, created or opened. NOT
///        declared here with the ref tables (`db_agent_skills_ref`, `db_bundle_skills_ref` and the
///        MCP equivalents): those stay channel-scoped until Phase 3/6 promotes them, and adding
///        them unpaired with a live writer would be schema for a phase that hasn't shipped
///   v7 — partial UNIQUE indexes on `db_skills(name, skill_type)` and `db_mcp_servers(name)`, each
///        `WHERE is_global = 1`: global rows only, so two owners' private rows may share a name.
///        Closes a race in `m0031_carry_skills_and_mcp_servers_to_identity_store`: two channels'
///        migrations can run concurrently (several AgentMux instances is a supported scenario),
///        both read "no matching global row yet" and insert under different ids, and the first-wins
///        cross-channel dedup fails without a DB-level constraint to arbitrate (#3181). The
///        migration's insert now handles the resulting `UNIQUE constraint failed` by re-querying
///        for the winning row
///   v8 — `idx_ids_skills_global_name` replaces `idx_ids_skills_global_name_type`: the skill
///        global-uniqueness index narrows from `(name, skill_type)` to `name`, matching the
///        invariant `skill_upsert_unique_global`'s dup-check enforces (`WHERE name = ?1 AND id <>
///        ?2 AND is_global = 1`, no `skill_type` filter). The v7 index was narrower than that
///        invariant, so a same-name, different-`skill_type` race between two channels could insert
///        two rows the application treats as duplicates (#3182, SPEC_DURABLE_BINDINGS_2026_09_10.md
///        §5.4). MCP servers don't have the mismatch (`idx_ids_mcp_servers_global_name` is already
///        name-only), so this bump is skills-only. `m0033_narrow_skill_global_uniqueness_index`
///        collapses any same-name/different-type pair that `m0031`'s `(name, skill_type)` dedup may
///        have produced, keeping the earlier-`created_at` row, before the new index is created
///   v9 — db_bundle_versions, for parity with OBJECT_SCHEMA_VERSION v35 /
///        SHARED_STORE_SCHEMA_VERSION v10 (same reasoning as v3;
///        docs/specs/SPEC_AGENT_FACING_GLOBAL_MEMORY_API_2026_09_15.md)
///   v10 — db_work_queue.target_agent_uid / claimed_by_uid and db_cron_jobs.target_uid (parity with
///        SHARED_STORE_SCHEMA_VERSION v11): the UID (`db_agents.id`) beside each slug column,
///        dual-written, not yet read (identity M1b,
///        SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED_2026_09_23.md §9.1). `target_agent_uid` is
///        resolved ONCE at enqueue (§5.4); `claimed_by_uid` is CARRIED from the claimer's own
///        `AGENTMUX_AGENT_UID` (M1a, #3548), else resolved from its `agent_id` with the fallback
///        counted (§9.2). Empty when unknown, never guessed
///   v11 — db_work_queue.created_by_uid, and db_cron_jobs.created_by_uid /
///        db_bundle_versions.written_by_uid (parity with SHARED_STORE_SCHEMA_VERSION v12): the
///        actor's UID beside each actor-name column, from the request's `Caller` (its
///        `X-Agent-Token`), never a body field. Empty for an Unattributed request, never guessed.
///        Dual-written, not yet read (identity M4c-1,
///        SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED_2026_09_23.md §6.5.9)
///
/// Added WITHOUT a bump: `db_mcp_server_reach` (OBJECT_SCHEMA_VERSION v44
/// has the same table). The catalog's reach switches live with the catalog,
/// here, so they survive an upgrade where an agent bind
/// (`db_agent_mcp_ref`, in `objects.db`) does not. This store is one file
/// shared by every AgentMux on the computer, installed and dev builds alike,
/// and `check_schema_compat` makes any build that speaks an older version
/// refuse it. So a bump here locks every older AgentMux on the computer out
/// as soon as one newer build starts: a dev build of a branch with a bump
/// did exactly that on 2026-10-11. A table an older build never reads needs
/// no lock: an older build ignores it, a newer one creates it if missing.
/// Bump only for a change an older build would misread or corrupt.
/// `SPEC_CONNECTORS_MCP_WHAT_AGENTS_RUN_2026_10_10.md` §2.1.
pub const IDENTITY_STORE_SCHEMA_VERSION: i64 = 11;

/// Initialize (or re-validate) the `~/.agentmux/shared/identity-store.db` schema: the
/// permanently-global store from docs/specs/SPEC_IDENTITY_STORE_SPLIT_2026_08_17.md, which
/// fixes conversation continuity across a version switch
/// (docs/specs/REPORT_HISTORY_CONTINUITY_ACROSS_VERSION_UPGRADE_2026_08_17.md).
///
/// A **near-superset** of [`run_shared_store_schema`]'s tables (v2), including `db_accounts`,
/// but there it is a read-through FALLBACK MIRROR, not the authoritative write path. Accounts
/// have one intentional isolation need (Armory's disposable delete-account testing flow) and
/// the per-account creation-time scope split (SPEC §3.2) hasn't shipped, so `id_store` /
/// `resolve_shared_store_path()` stays the primary read/write path for accounts. This copy
/// exists only so a link resolved here (always global) doesn't dead-end on an account row stuck
/// in a fresh, per-channel `id_store`: see `identity::resolver::inject`'s account resolution
/// (falls back to this store's `identity_get` only when `id_store`'s lookup misses) and
/// `migrations::m0022_identity_store_links_backfill`, which backfills the mirror as it does
/// links. `db_agent_identity_links.account_id` has NO foreign key here, as
/// `run_shared_store_schema` does for `db_agent_definitions`: cross-database FKs can't be
/// enforced in SQLite and the mirror isn't guaranteed complete, so application code enforces
/// integrity and tolerates a miss.
///
/// A dedicated store, not a subset of the shared store reached by a new always-global path, is
/// deliberate: "can this table ever be reached from a per-channel-isolated codepath" becomes a
/// structural fact (a separate file and connection) rather than a convention a later call site
/// of `resolve_shared_store_path()` could violate, the failure mode
/// docs/specs/CHECKLIST_AGENT_DATA_SCOPE_ROUTING_2026_08_17.md items 2-4 record as having
/// bitten more than once.
///
/// Idempotent — safe to call on every startup.
pub fn run_identity_store_schema(conn: &Connection) -> Result<(), StoreError> {
    conn.execute_batch(store_schema::IDENTITY_STORE_TABLES_SQL)?;
    conn.execute_batch(super::vault::VAULT_SCHEMA)?; // The Vault's tables, no version bump: storage/vault.rs.
    store_schema::seed_identity_blank_bundle(conn)?;
    store_schema::add_identity_store_columns(conn)?;

    Ok(())
}

/// Initialize the FileStore schema. Creates the wave_file and file_data
/// tables. Already a flat single-DDL store — unaffected by the
/// `objects.db` flattening.
pub fn run_filestore_migrations(conn: &Connection) -> Result<(), StoreError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS db_wave_file (
            zoneid TEXT NOT NULL,
            name TEXT NOT NULL,
            size INTEGER NOT NULL DEFAULT 0,
            createdts INTEGER NOT NULL DEFAULT 0,
            modts INTEGER NOT NULL DEFAULT 0,
            opts TEXT NOT NULL DEFAULT '{}',
            meta TEXT NOT NULL DEFAULT '{}',
            PRIMARY KEY (zoneid, name)
        );

        CREATE TABLE IF NOT EXISTS db_file_data (
            zoneid TEXT NOT NULL,
            name TEXT NOT NULL,
            partidx INTEGER NOT NULL,
            data BLOB NOT NULL,
            PRIMARY KEY (zoneid, name, partidx)
        );",
    )?;
    // Line counter (Phase 5a-2, SPEC_AGENT_PANE_BOUNDED_LIVE_WINDOW_MIGRATION_
    // 2026_09_23.md §6.3.7; see `filestore/counter.rs`). Additive and
    // nullable, and deliberately WITHOUT a FILESTORE_SCHEMA_VERSION bump: the
    // global transcript store is shared by every srv instance on the machine,
    // and a bump would make older builds refuse to open it. Older builds
    // insert rows without these columns (NULL = not counted) and write
    // without maintaining them, which the counter detects. "duplicate
    // column" is expected when another instance added them first.
    apply_additive_columns(conn, [
        "ALTER TABLE db_wave_file ADD COLUMN gen TEXT",
        "ALTER TABLE db_wave_file ADD COLUMN lines INTEGER",
        "ALTER TABLE db_wave_file ADD COLUMN lines_size INTEGER",
        "ALTER TABLE db_wave_file ADD COLUMN lines_tail INTEGER",
        "ALTER TABLE db_wave_file ADD COLUMN lines_modts INTEGER",
        // Bumped (by the triggers below) by every write that changes bytes
        // already in the file — not by appends — and the value an epoch was
        // counted at. An epoch is valid only while they are equal.
        "ALTER TABLE db_wave_file ADD COLUMN rev INTEGER",
        "ALTER TABLE db_wave_file ADD COLUMN lines_rev INTEGER",
        // A random identity per row, set by the insert trigger below: a file
        // deleted and re-created is a different incarnation even if every
        // other column (and the creation millisecond) comes out the same.
        "ALTER TABLE db_wave_file ADD COLUMN incarnation BLOB",
    ])?;
    // `rev` is maintained by triggers, not by this code's writes, because
    // triggers are part of the database: they fire for every connection,
    // including older builds that know nothing about the counter. Any write
    // that changes existing bytes bumps it:
    //   - a part deleted (write_file's replace, a delete);
    //   - a part written over with bytes that aren't a pure extension of it
    //     (REPLACE INTO / UPDATE). Appending extends the last part, which
    //     keeps it a prefix, so appends — ours or an older build's — don't.
    // REPLACE INTO doesn't fire DELETE triggers (recursive_triggers is off,
    // SQLite's default; nothing here enables it), so an append can't reach
    // the delete trigger. A trigger on a deleted row's parts finds no row.
    conn.execute_batch(
        "-- Versioned name: CREATE TRIGGER IF NOT EXISTS never updates an
         -- existing definition, so a changed rule gets a new name and the
         -- old one is dropped.
         DROP TRIGGER IF EXISTS db_file_data_rev_insert;
         CREATE TRIGGER IF NOT EXISTS db_file_data_rev_insert_v2
         BEFORE INSERT ON db_file_data
         WHEN EXISTS (
             SELECT 1 FROM db_file_data d
             WHERE d.zoneid = NEW.zoneid AND d.name = NEW.name AND d.partidx = NEW.partidx
               AND substr(NEW.data, 1, length(d.data)) IS NOT d.data
         )
         -- A NEW part inside the range the file already claims: bytes the
         -- file declared (and a reader may have read as missing) are being
         -- filled in, not appended. Every writer inserts appended parts
         -- before raising `size`, so an append's new parts always start at
         -- or past it. Older builds' write_file raises `size` first and
         -- inserts after (Codex on #3631). 65536 is PART_DATA_SIZE.
         OR (
             NOT EXISTS (
                 SELECT 1 FROM db_file_data d
                 WHERE d.zoneid = NEW.zoneid AND d.name = NEW.name AND d.partidx = NEW.partidx
             )
             AND NEW.partidx * 65536 < (
                 SELECT COALESCE(MAX(w.size), 0) FROM db_wave_file w
                 WHERE w.zoneid = NEW.zoneid AND w.name = NEW.name
             )
         )
         BEGIN
             UPDATE db_wave_file SET rev = COALESCE(rev, 0) + 1
             WHERE zoneid = NEW.zoneid AND name = NEW.name;
         END;

         CREATE TRIGGER IF NOT EXISTS db_file_data_rev_update
         BEFORE UPDATE OF data ON db_file_data
         WHEN substr(NEW.data, 1, length(OLD.data)) IS NOT OLD.data
         BEGIN
             UPDATE db_wave_file SET rev = COALESCE(rev, 0) + 1
             WHERE zoneid = OLD.zoneid AND name = OLD.name;
         END;

         CREATE TRIGGER IF NOT EXISTS db_file_data_rev_delete
         AFTER DELETE ON db_file_data
         BEGIN
             UPDATE db_wave_file SET rev = COALESCE(rev, 0) + 1
             WHERE zoneid = OLD.zoneid AND name = OLD.name;
         END;

         -- Every row, whoever inserts it (older builds included), gets 64
         -- random bits of identity. A delete + re-create in the same
         -- millisecond with the same size and bytes is still a different
         -- incarnation (Codex on #3631).
         CREATE TRIGGER IF NOT EXISTS db_wave_file_incarnation
         AFTER INSERT ON db_wave_file
         BEGIN
             UPDATE db_wave_file SET incarnation = randomblob(8)
             WHERE zoneid = NEW.zoneid AND name = NEW.name;
         END;

         UPDATE db_wave_file SET incarnation = randomblob(8) WHERE incarnation IS NULL;",
    )?;
    Ok(())
}

/// Initialize the saga durability schema (`saga` + `saga_step` tables and
/// their indexes). See `docs/specs/SPEC_SAGA_DURABILITY_2026-05-01.md` §2.2.
/// Already a flat single-DDL store.
pub fn run_saga_log_migrations(conn: &Connection) -> Result<(), StoreError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS saga (
            saga_id        INTEGER PRIMARY KEY,
            name           TEXT NOT NULL,
            state          TEXT NOT NULL,
            started_at     INTEGER NOT NULL,
            terminal_at    INTEGER,
            failure_reason TEXT,
            input_json     TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS saga_step (
            saga_id     INTEGER NOT NULL REFERENCES saga(saga_id),
            step_index  INTEGER NOT NULL,
            name        TEXT NOT NULL,
            state       TEXT NOT NULL,
            cmd_json    TEXT NOT NULL,
            output_json TEXT,
            started_at  INTEGER NOT NULL,
            ended_at    INTEGER,
            PRIMARY KEY (saga_id, step_index)
        );

        CREATE INDEX IF NOT EXISTS saga_state_idx
            ON saga(state) WHERE state IN ('running', 'compensating');
        CREATE INDEX IF NOT EXISTS saga_terminal_idx
            ON saga(terminal_at);",
    )?;
    Ok(())
}

/// `PRAGMA user_version` tripwire (AUDIT_SQLITE_SYSTEMS §8.5): compare the file's
/// `user_version` to `current` and refuse to open a database written by a NEWER binary. This is
/// the forward-compat safety lock of the channels design (`SPEC_DATA_CHANNELS_2026_05_24.md`
/// §3.3): within a channel several released AgentMux versions share one data dir, and the lock
/// keeps an older binary from writing into a schema a newer one laid down (as Chrome's
/// profile-too-new check and Postgres's catalog version mismatch do).
///
/// **Must be called BEFORE `run_*_schema` / `run_*_migrations`.** Those include mutating steps
/// (legacy-table renames, seed inserts, `CREATE TABLE` for tables the older binary doesn't
/// have), so checking afterwards would let a downgraded binary alter a newer database before
/// the error fires, breaking the "reject without touching disk" invariant (#1029).
///
/// Read-only: only a `PRAGMA user_version` query. [`stamp_version`] does the corresponding
/// write AFTER migrations succeed.
///
/// On `SchemaTooNew` the user upgrades AgentMux to a version ≥ `found`, or sets
/// `AGENTMUX_CHANNEL=<other>` to land in a different channel dir; the data on disk is preserved
/// either way, since this function never modifies the database on the rejected path.
pub fn check_schema_compat(
    conn: &Connection,
    current: i64,
    db_label: &str,
) -> Result<(), StoreError> {
    let found: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if found > current {
        warn!(
            db = db_label,
            found, expected = current,
            "database user_version is newer than this build — refusing \
             to open. Upgrade AgentMux or switch channels.",
        );
        return Err(StoreError::SchemaTooNew {
            db: db_label.to_string(),
            found,
            expected: current,
        });
    }
    Ok(())
}

/// Stamp the database's `user_version` PRAGMA to `current`. Called
/// AFTER `run_*_schema` succeeds, paired with a prior
/// [`check_schema_compat`] that gated the migrations on the
/// caller-binary speaking a compatible (or newer) schema version.
///
/// Splitting the read from the write makes the safety-lock order
/// explicit at every call site:
///
/// ```ignore
/// check_schema_compat(&conn, OBJECT_SCHEMA_VERSION, "objects.db")?;
/// run_object_schema(&conn)?;
/// stamp_version(&conn, OBJECT_SCHEMA_VERSION)?;
/// ```
pub fn stamp_version(conn: &Connection, current: i64) -> Result<(), StoreError> {
    conn.execute_batch(&format!("PRAGMA user_version = {current};"))?;
    Ok(())
}

#[cfg(test)]
#[path = "migrations_tests.rs"]
mod tests;
