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

/// `user_version` stamped into `~/.agentmux/shared/store.db` after
/// `run_shared_store_schema`. Versioned independently from `objects.db`.
///   v1 — initial: identity accounts/bundles/bindings/links, memory
///         bundles, drone definitions, muxbus credentials
///   v2 — db_cron_jobs: persistent scheduled injection jobs
///   v3 — Phase 4a of SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md: rename
///        db_identity_accounts -> db_accounts, db_memory_bundles -> db_bundles
///        (SPEC_ARMORY_PHASE4_STORAGE_RENAME_COMPLETION_2026_07_12.md).
///   v4 — db_agent_credentials: per-agent M2M Cognito client_id/secret +
///        cached access token, one row per locally-registered agent_id.
///        Lets cloud_subscriber use a credential bound to a specific agent
///        instead of the single shared per-account MUXBUS_TOKEN for every
///        /reactive/* call.
///   v5 — db_cron_jobs.max_age_secs: optional hard expiry bound (seconds since
///        created_at) alongside the existing max_fires bound. NULL = no expiry
///        (existing rows unaffected). See
///        docs/specs/SPEC_AGENT_POLLING_AND_WAKEUP_HARDENING_2026_08_04.md Phase 0.
///   v6 — db_agent_native_memory: durable, location-consistent mirror of
///        each agent's native memory files, keyed by (agent_id, filename).
///        See docs/specs/SPEC_NATIVE_MEMORY_DURABLE_SYNC_2026_08_07.md.
///   v7 — db_bundles.instructions_by_provider: JSON object of
///        {provider_id: content} instruction variants, additive to the
///        existing flat `instructions` column. ABF v0.2 §2.2, see
///        docs/specs/SPEC_ABF_V0_2_PROVIDER_AWARE_COMPONENTS_AND_NATIVE_MEMORY_2026_08_10.md.
///   v8 — db_agent_native_memory_versions: append-only version history for
///        native memory content, one row per `agent:memory:write_file`
///        call (plus reverts and, later, out-of-band-write detection) —
///        the mirror in v6 only ever holds the current value, with no way
///        to see what a file contained before its last write. See
///        docs/specs/SPEC_MEMORY_VERSION_CONTROL_AND_ARMORY_AUDIT_2026_08_19.md.
///   v9 — db_bundles.is_system: AgentMux-controlled, highest-priority
///        Global Bundle tier — see OBJECT_SCHEMA_VERSION's v27 doc
///        comment (objects.db, above run_object_schema) for the full
///        design; this store just needs schema parity.
///   v10 — db_bundle_versions: append-only version history for Global
///        Memory entry content — see OBJECT_SCHEMA_VERSION's v35 doc
///        comment for the full rationale
///        (SPEC_AGENT_FACING_GLOBAL_MEMORY_API_2026_09_15.md); this store
///        just needs schema parity, same as v9 above.
///   v11 — db_cron_jobs.target_uid: the target agent's UID (`db_agents.id`)
///        beside the slug column, resolved once at create time and
///        dual-written; nothing reads it yet, a job still fires by
///        `target`. Identity M1b of
///        SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED_2026_09_23.md §9.1 —
///        spec §5.4: non-interactive ingress never resolves a name at fire
///        time, so the UID has to be captured while a human or model is
///        present. Empty when the name did not resolve; never guessed.
///   v12 — db_cron_jobs.created_by_uid and db_bundle_versions.written_by_uid:
///        the actor's UID beside each actor-name column, taken from the
///        request's `Caller` (its `X-Agent-Token`), never from a body field.
///        Empty for an Unattributed request; never guessed. Dual-written,
///        not yet read. Identity M4c-1 of
///        SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED_2026_09_23.md §6.5.9.
pub const SHARED_STORE_SCHEMA_VERSION: i64 = 12;

/// `user_version` value stamped into `objects.db` after `run_object_schema`.
/// The flat schema reset the counter to 1 (the pre-flatten chain never set
/// `user_version`, so legacy files read 0). Bumped per additive migration:
///   v1 — flat schema baseline
///   v2 — db_agent_definitions.updated_at
///   v3 — db_agent_definitions.user_hidden (Phase 2 hide templates,
///        SPEC_AGENT_PICKER_TWO_TIER_2026_05_24.md Q2 Decision Y)
///   v4 — db_agents consolidation table (Phase 3a: every definition/
///        instance write dual-writes here). Reads moved over
///        piecemeal afterwards — the instance side in Phase 3b (#3080),
///        every `agent_def_*` method in Phase 3d (#3092, v31) — and both
///        legacy tables were dropped in v32.
///   v5 — db_agents.last_block_id (Phase 3c; latest launch's block, so the
///        consolidated read can find the session snapshot without joining
///        db_agent_instances)
///   v6 — container_image / container_volumes / container_name on both
///        db_agent_definitions and db_agents (Phase 0 of
///        SPEC_CONTAINER_PANE_SUPPORT_2026_06_11.md; host agents default
///        to '' / '[]' / '')
///   v7 — db_muxbus_credentials: global singleton for MuxBus cloud
///        Cognito PKCE tokens (access, refresh, id) + expiry + user email
///   v8 — db_memory_bundles.is_global: global-tier flag for Armory
///        bundles injected into every agent's CLAUDE.md at launch
///   v9 — db_memory_bundles.sort_order: explicit ordering for the Armory
///        global bundles (controls CLAUDE.md injection order). Existing
///        rows default to 0; the Armory Global section assigns positions via reorder.
///   v10 — db_skills, db_mcp_servers, db_agent_skills_ref, db_agent_mcp_ref:
///        standalone MCP Server and Skill primitives with per-agent ref tables
///        (v1 composable model, SPEC_V1_MCP_SKILLS_PRIMITIVES_2026_06_30.md).
///   v11 — Phase 4a of SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md: rename
///        db_identity_accounts -> db_accounts, db_memory_bundles -> db_bundles
///        (SPEC_ARMORY_PHASE4_STORAGE_RENAME_COMPLETION_2026_07_12.md).
///   v12 — use_ambient_login on both db_agent_definitions and db_agents:
///        explicit per-agent opt-in to the CLI's global (ambient) login when
///        no oauth-class account resolves at spawn (layer 3 of
///        SPEC_ACCOUNT_DELETE_DEAUTH_LAYERS_2_4_2026_07_14.md §2.3). Defaults
///        to 0 (fail-by-default); the m0017 data migration grandfathers
///        pre-existing linkless agents to 1.
///   v13 — db_agent_credentials: per-agent M2M muxbus credential, mirroring
///        db_muxbus_credentials' presence in both schemas — id_store falls
///        back to this (per-channel) schema on installs that haven't yet
///        applied 0011_shared_store_backfill, so cloud_subscriber's calls
///        must not assume the shared-schema copy (v4 there) is the one in use.
///   v14 — db_agent_native_memory: durable mirror of each agent's native
///        memory files, keyed by (agent_id, filename). Same both-schemas
///        duplication as db_agent_credentials, for the same id_store
///        fallback reason. See
///        docs/specs/SPEC_NATIVE_MEMORY_DURABLE_SYNC_2026_08_07.md.
///   v15 — model_vendor_base_url on both db_agent_definitions and db_agents:
///        redirects a harness (CLI) at a non-default model vendor backend
///        (e.g. ANTHROPIC_BASE_URL for a claude-provider agent) — the data-
///        model side of formalizing harness vs. model-vendor as distinct
///        concepts. Defaults to '' (use the harness's default vendor).
///        Only settable via `agent.define`, validated against the resolved
///        provider's `ProviderConfig::base_url_env_var`.
///   v16 — db_bundles.instructions_by_provider: JSON object of
///        {provider_id: content} instruction variants, additive to the
///        existing flat `instructions` column (which keeps meaning
///        "default"). ABF v0.2 storage half of provider-scoped
///        instructions — see
///        docs/specs/SPEC_ABF_V0_2_PROVIDER_AWARE_COMPONENTS_AND_NATIVE_MEMORY_2026_08_10.md
///        §2.2. Defaults to '{}' for existing rows; delivering a selected
///        variant into a running agent is explicitly out of scope for this
///        version (per that spec's non-goals) — this column only makes the
///        data storable/exportable/importable.
///   v17 — auto_continue_enabled on both db_agent_definitions and db_agents:
///        the Warden Supervisor's per-agent auto-continue opt-in (default 0).
///        RETIRED with the Warden Supervisor: nothing reads or writes it now.
///        It stays in the schema because older builds sharing the data dir
///        still select it; new rows get DEFAULT 0. See
///        docs/analysis/ANALYSIS_WARDEN_AUTO_CONTROLLER_CONTINUATION_WATCHER_2026_08_12.md.
///   v18 — db_agent_jekt_keys: per-agent HMAC-SHA256 signing key for
///        host-tier jekt sender verification. One row per agent_id, minted
///        on first use and injected into that agent's own MCP server
///        process env (AGENTMUX_JEKT_KEY) at spawn — never into any other
///        agent's env, so a signature can only be produced by the agent it
///        claims to be from. See
///        docs/specs/SPEC_JEKT_TRUST_LAYER_COMPLETION_2026_08_13.md §2.2.
///   v19 — db_agent_definitions.memory_id: the agent's own dedicated ABF
///        bundle, set once at creation and never updated thereafter
///        (agent_def_update's SET clause deliberately omits it, same
///        immutable-after-creation treatment as is_seeded/slug/parent_id).
///        Distinct from db_agent_instances.memory_id (a specific launch's
///        bundle, can still diverge on purpose). Mirrored into db_agents as
///        a SEPARATE default_memory_id column, not db_agents.memory_id —
///        that column is instance-only by existing convention (see
///        db_agents' CREATE TABLE comment). Defaults to '' for existing
///        rows; the m0021 migration
///        backfills every unbound definition to a freshly-provisioned
///        bundle. See ARCHITECTURE_MANDATORY_ABF_RETHINK_2026_08_14.md §3.1.
///   v20 — db_agent_lan_keys: per-agent Ed25519 keypair for LAN-tier jekt
///        sender verification — mirrors v18, asymmetric instead of HMAC
///        (LAN is multi-party: a receiving peer must verify without being
///        able to forge, which a shared secret can't provide). public_key
///        is distributed to LAN peers on demand (not secret); private_key
///        goes only to that one agent (its MCP config's AGENTMUX_LAN_KEY,
///        and its own token-authenticated key fetch), same guarantee as
///        v18. See docs/specs/SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md §2.1.
///   v21 — db_lan_peer_pubkey_pins: trust-on-first-use pin of a REMOTE
///        agent_id's LAN public key, as first observed from mDNS-discovered
///        peers. reagentx P0 on the LAN signing PR: peer discovery itself
///        is unauthenticated (any device can broadcast an mDNS instance and
///        register an agent under an existing agent's name), so trusting
///        "whichever peer answers first" for a pubkey lookup lets an
///        attacker's self-minted key be accepted as authoritative for a
///        victim's agent_id. Pinning the first-seen key and rejecting any
///        later mismatch (rather than silently accepting the newest
///        answer) is the standard SSH-host-key mitigation for exactly this
///        "no PKI available" situation — closes the always-spoofable case,
///        narrows it to a race on the very first lookup ever performed for
///        that agent_id. See
///        docs/specs/SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md §2.2.
///   v22 — db_background_tasks: durable record of a declared long-running
///        (`run_in_background: true`) task attached to a block, so its
///        liveness survives past `DockSnapshotCache`'s 1-hour eviction and
///        a session reconnect/reload — the ephemeral client-transcript-
///        derived signal chain had no durable source of truth at all. Per-
///        channel (block ids are only meaningful within their own
///        channel), not the shared store. Renumbered from an earlier v20
///        to v22 when merged alongside the LAN-tier jekt signing PR, which
///        independently claimed v20/v21 for an unrelated table pair — see
///        docs/status/STATUS_ATTACHED_TASK_AXIS_AND_DEV_LOOP_2026_08_15.md,
///        issue #2492.
///   v23 — db_bundle_skills_ref, db_bundle_mcp_ref: bundle-level references
///        to the standalone skill/MCP-server primitives, mirroring the v10
///        agent-level ref tables but keyed by bundle_id — composable model
///        v2, step 1 of docs/specs/SPEC_BUNDLE_AS_CONTAINER_V2_2026_08_17.md
///        (GH issue #2024 item 3). A bundle's existing mcp_servers/skills
///        JSON columns are untouched by this — still the .abf export/import
///        format — these ref tables are the new live-resolution path.
///        Deliberately NO FK to db_bundles (unlike the agent-level ref
///        tables' FK to db_agent_definitions) — see this table's own doc
///        comment at its CREATE TABLE for why.
///   v24 — db_agent_native_memory_versions: append-only version history for
///        native memory content, mirroring the v14 table's own dual-
///        schema duplication (id_store per-channel fallback). See
///        docs/specs/SPEC_MEMORY_VERSION_CONTROL_AND_ARMORY_AUDIT_2026_08_19.md.
///   v25 — db_agent_groups: saved, named target sets for the fleet-control
///        feature (broadcast/bulk-stop across many agent panes at once —
///        docs/specs/SPEC_MULTI_AGENT_FLEET_CONTROL_2026_08_20.md). A
///        group is just a durable list of block ids under a name — the
///        Ansible-inventory-group lesson from that spec's research: a
///        saved group, not raw ephemeral selection, is the abstraction
///        that scales as a fleet grows. Per-channel (block ids are only
///        meaningful within their own channel), not the shared store.
///        Renumbered from an earlier v24 — merged alongside the native-
///        memory-version-history PR, which independently claimed v24 for
///        an unrelated table (see that entry above for the same class of
///        collision this codebase has hit before, e.g. v20→v22).
///   v26 — db_agent_definitions.conversation_visibility (TEXT, default
///        'private') + db_conversation_trust_grants: `muxspect` Phase B/C's
///        per-agent disclosure policy for an incoming cross-tier
///        `transcript_request` jekt (LAN/WAN conversation visibility —
///        docs/specs/SPEC_MUXSPECT_CROSS_TIER_CONVERSATION_VISIBILITY_2026_08_21.md,
///        jekt tier rules confirmed in
///        docs/specs/SPEC_JEKT_TRANSCRIPT_REQUEST_TIER_RULES_2026_08_22.md).
///        One of "private" (default, auto-deny)/"trusted_peers" (auto-
///        approve an allow-listed requester)/"ask" (force human
///        escalation). Also dual-written to `db_agents` (mirrors
///        `auto_continue_enabled`'s treatment exactly — a simple per-agent
///        opt-in-style setting, unlike `memory_id`/`model_vendor_base_url`,
///        which use a differently-named column there). Deliberately NOT
///        added to `DefinitionRecordV1`'s cross-channel wire format —
///        channel-local only, same precedent as those two fields; a
///        cross-channel-reopened agent starts back at the safe "private"
///        default. `db_conversation_trust_grants` mirrors
///        `db_lan_peer_pubkey_pins`'s exact shape (own module,
///        get/set-style methods, case-insensitive agent_id lookups).
///   v27 — db_bundles.is_system: an AgentMux-controlled, highest-priority
///        Global Bundle tier (INTEGER, default 0). A system row is always
///        also is_global=1 (enforced in code, not a CHECK constraint —
///        see `bundle_upsert_system`). Writable only through the
///        dedicated `upsertsystemmemory`/`deletesystemmemory` RPCs and
///        `Store::bundle_upsert_system`/`_delete_system` — the
///        generic `bundle_upsert`/`_delete`/`_reorder` all refuse
///        to touch an is_system=1 row. Injected first in
///        `format_global_bundle_block`'s output, wrapped in explicit
///        override wording, ahead of every ordinary Global Bundle
///        section. See docs/specs/SPEC_GLOBAL_MEMORY_SYSTEM_TIER_2026_08_24.md.
///   v28 — db_agent_activity_summaries: one-shot, on-demand Haiku-generated
///        activity summary per `definition_id`, used as the AgentPicker's
///        "My Agents" conversation-preview fallback when a row has no
///        structured `output.state.json` snapshot to preview (legacy rows
///        predating snapshot persistence — see `has_snapshot` in
///        `listrecentsessions`, `agent_handlers/session.rs`). Generated
///        from the instance's raw `"output"` filestore file (present even
///        when the newer structured snapshot isn't) via the same shared
///        Ambient Model Call gateway `dispatch_name`/`term:ambient_summary`
///        already use (`invoke_haiku`,
///        `docs/specs/SPEC_AMBIENT_MODEL_CALLS_FRAMEWORK_2026_07_03.md`).
///        Generated lazily (first picker load that needs it, not on every
///        turn) and cached forever once non-empty — mirrors
///        `SubAgent.display_name`'s cache-once posture, not
///        `term:ambient_summary`'s per-turn refresh. Renumbered from an
///        earlier v27 — merged alongside the Global Bundle system-tier PR
///        (#2782), which independently claimed v27 for an unrelated
///        column (same collision class as this file's own v20→v22 and
///        v24→v25 history, see those entries above). See
///        docs/reports/REPORT_AGENT_PICKER_FIELD_ORDER_SORT_AND_DATA_GAPS_AUDIT_2026_08_24.md §5a.
///   v29 — db_agents.session_id / status / started_at / ended_at: the
///        agent's LATEST launch state, the four `db_agent_instances`
///        columns the consolidated row never got (Phase 3a deliberately
///        left them "on the block", but the recent-sessions picker, the
///        pane close/reopen continuity write-back and the status-filtered
///        instance list all kept reading them from the legacy table — the
///        reason it could not be dropped). One row per agent ⇒ one launch
///        state per agent; a continuation moves it to the new block/session.
///        Backfilled from the legacy rows by
///        `m0025_agents_launch_state_backfill` (latest instance per
///        projection key), kept current by the instance dual-write, and
///        the read flip in `SPEC_AGENT_ARCHITECTURE_2026_05_27.md` Phase
///        3b retires the legacy reads on top of it. No unique index on
///        `db_agents(slug)` is added: template-launch projections copy
///        their template's slug, so uniqueness stays a definition-level
///        rule enforced by the slug collision scan in `agent_def_insert`.
///   v30 — `db_agent_content` / `db_agent_skills` / `db_agent_history` /
///        `db_agent_identity_links` / `db_agent_skills_ref` /
///        `db_agent_mcp_ref`: `agent_id`'s foreign key now targets
///        `db_agents(id)` instead of `db_agent_definitions(id)`. Every
///        existing row already satisfies the new target (its `agent_id` is
///        a definition id, and every definition has a `db_agents` mirror
///        at the same id) — what changes is that a `db_agents` row with NO
///        definition counterpart (a template launch) can now hold these
///        directly too, closing the gap `listrecentsessions`' template
///        fallback (Phase 3b, #3080) exists for. SQLite has no in-place FK
///        redefinition, so an existing store needs its six tables rebuilt;
///        `m0028_agent_child_tables_repoint_fk` does that (rename,
///        recreate, copy, drop, `PRAGMA foreign_key_check` before commit).
///        A fresh install gets the new FK directly from the CREATE TABLE
///        statements below — nothing to rebuild there. Phase 3c
///        (`SPEC_AGENT_ARCHITECTURE_2026_05_27.md`).
///   v31 — No DDL change. Every `agent_def_*` write now lands on
///        `db_agents` only — `db_agent_definitions` is no longer kept in
///        sync (Phase 3d, #3092). Bumped anyway (codex P1 on #3092):
///        `check_schema_compat` is a downgrade guard as much as a schema
///        guard, and this release changes what a NEWER binary's writes
///        mean to an OLDER one just as much as a column would. Without the
///        bump, rolling back to the immediately preceding binary after
///        this version has run would silently accept a store where new
///        agents exist only in `db_agents` — that binary's `agent_def_get`/
///        `update`/`delete` still target `db_agent_definitions` and would
///        404 on them, or replay stale legacy values back into `db_agents`
///        on edit. Stamping 31 makes that downgrade refuse to open instead.
///   v32 — `db_agent_definitions` and `db_agent_instances` are dropped.
///        Nothing has read or written either table since v31 (agent-concept
///        consolidation's definition and instance flips, #3080/#3092); this
///        is the final phase (Phase 3e,
///        `SPEC_AGENT_ARCHITECTURE_2026_05_27.md`) closing out the tracking
///        spec. `db_agent_instances`' `CREATE TABLE`/indexes and
///        `db_agent_definitions`' `CREATE TABLE` are gone from this function
///        — leaving either would silently recreate the tables (empty) on
///        the very next `Store::open()` after the drop migration runs,
///        since migrations execute before this function does on every
///        boot. `db_agent_definitions`' own `ALTER TABLE ADD COLUMN`
///        entries, by contrast, are KEPT — gated on the table still
///        existing (see that block's own comment below) — because several
///        pending migrations (m0007, m0025, m0026, m0028) call
///        `Store::open()` internally, running this function, before m0029
///        (numbered last) ever drops the table in the SAME upgrade pass; a
///        store old enough to be missing one of those columns would fail
///        those migrations' own reads with "no such column" otherwise
///        (Codex P1 on #3096). An existing store's tables are actually
///        removed by `m0029_drop_legacy_agent_tables`; a fresh install
///        never has them to begin with, so the gate is a no-op for it.
///        `adopt_legacy_table_names`'s
///        `db_forge_agents` → `db_agent_definitions` rename entry is left
///        in place deliberately: an install old enough to still carry
///        `db_forge_agents` reaches it during an EARLIER migration's own
///        `Store::open()` call (every migration in this chain lays down
///        schema that way), so the rename still fires before
///        `m0029` runs later in the same upgrade pass and correctly drops
///        what the rename just produced — no special-casing needed, and no
///        data is stranded under the old name.
///   v33 — `db_agent_project_instructions`: what each agent was last observed
///        to read as project instructions from its working directory, keyed
///        `(agent_id, path)`. Phase 3 of
///        `SPEC_INSTRUCTION_AND_MEMORY_PORTABILITY_2026_09_09.md`.
///
///        Records a content hash and an observation time, never the content:
///        this is a tracking table, not a mirror. The distinction from
///        `db_agent_native_memory` above is deliberate — that one stores
///        content because it is a durable copy of files AgentMux may have to
///        serve when the originals are unreachable, whereas these files
///        belong to the repository, are always readable from it, and are
///        never written by AgentMux. Storing their content would create a
///        second copy of somebody else's file with no one to keep it honest.
///
///        Purely additive: nothing reads it at launch, and an agent with no
///        row simply has no prior observation to compare against.
///   v34 — `db_agent_skills_ref` / `db_agent_mcp_ref` / `db_bundle_skills_ref`
///        / `db_bundle_mcp_ref` drop their `skill_id`/`mcp_id` FK to this
///        store's LOCAL `db_skills`/`db_mcp_servers`. Phase 2 of
///        `SPEC_DURABLE_BINDINGS_2026_09_10.md` §5.4 (Codex P1, PR #3182):
///        once catalog writes redirect to `identity_store`, this channel's
///        local catalog copies stop gaining new rows, so binding a
///        post-redirect skill/server would otherwise hit a hard `FOREIGN KEY
///        constraint failed` (`PRAGMA foreign_keys=ON` in production, see
///        `Store::configure_and_migrate`). The `agent_id → db_agents` FK on
///        the agent-level pair is kept verbatim — only the catalog-pointing
///        FK is dropped. Same shape `db_bundle_skills_ref`/`db_bundle_mcp_ref`
///        already used for `db_bundles` (see those tables' own doc comment
///        below): existence moves to the application layer
///        (`managed_bind_agent`/`managed_bind_bundle`'s new `catalog: &Store`
///        check). SQLite has no `ALTER TABLE ... DROP CONSTRAINT`, so an
///        existing store's four tables are rebuilt by
///        `m0032_drop_catalog_fk_from_ref_tables`; a fresh install gets the
///        trimmed FK directly from the CREATE TABLE statements below.
///   v35 — db_bundle_versions: append-only version history for Global
///        Memory entry content (`name` + `instructions`), one row per
///        write through the new agent-facing Global Memory API
///        (`docs/specs/SPEC_AGENT_FACING_GLOBAL_MEMORY_API_2026_09_15.md`
///        Phase 0) — mirrors `db_agent_native_memory_versions`'s shape
///        exactly, keyed by a single `bundle_id` instead of that table's
///        composite `(agent_id, filename)`, since a Bundle's identity is
///        already a single id. Global Memory (`db_bundles`) previously had
///        no audit trail at all — only `updated_at`, no way to see what
///        changed, when, or by which agent, unlike native memory which has
///        had this since v24. Opened specifically because Global Memory is
///        about to gain a write path an agent (not just a human at the
///        Armory UI) can reach. `written_by` is a TRUSTED identity column
///        (an agent's real `AGENTMUX_AGENT_ID`, or `"armory-ui"`),
///        deliberately separate from the caller-describable `source`/
///        `source_detail` pair — an agent could otherwise claim
///        `source: "human"` with nothing to contradict it (codex P2, PR
///        #3237, caught before this table ever shipped).
///   v36 — db_agent_wan_keys: per-agent Ed25519 keypair for general
///        agent-to-agent WAN-tier jekt signing (issue #2586's second half).
///        Mirrors v20's shape; a SEPARATE keypair from db_agent_lan_keys by
///        design, not a reuse — LAN public keys are permanently TOFU-pinned
///        by peers (v21), which is why they cannot rotate
///        (SPEC_JEKT_HOST_KEY_TTL_ROTATION_2026_09_14.md §4), whereas WAN
///        keys are served from muxbus's authoritative ownership registry and
///        are meant to. One key cannot have both lifecycles. `key_version`
///        is reserved for the publish-sync and rotation phases (that spec's
///        §3.2/§3.6) and is not yet read by anything.
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
pub const OBJECT_SCHEMA_VERSION: i64 = 44;
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
/// confirmed via the already-applied `m0013`/`m0014` backfill migrations
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
        let legacy_exists: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
            [legacy],
            |row| row.get(0),
        )?;
        if legacy_exists == 0 {
            continue;
        }
        let current_exists: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
            [current],
            |row| row.get(0),
        )?;
        if current_exists == 1 {
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

    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS db_accounts (
            id           TEXT PRIMARY KEY,
            name         TEXT NOT NULL,
            provider     TEXT NOT NULL,
            kind         TEXT NOT NULL,
            display_name TEXT NOT NULL DEFAULT '',
            secret_ref   TEXT NOT NULL,
            context      TEXT NOT NULL DEFAULT '{}',
            status       TEXT NOT NULL DEFAULT 'unknown',
            created_at   INTEGER NOT NULL DEFAULT 0,
            updated_at   INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_ss_accounts_provider
            ON db_accounts(provider);

        CREATE TABLE IF NOT EXISTS db_agent_identity_links (
            agent_id   TEXT NOT NULL,
            account_id TEXT NOT NULL,
            provider   TEXT NOT NULL,
            PRIMARY KEY (agent_id, provider),
            FOREIGN KEY (account_id) REFERENCES db_accounts(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_ss_agent_identity_links_account
            ON db_agent_identity_links(account_id);

        CREATE TABLE IF NOT EXISTS db_bundles (
            id            TEXT PRIMARY KEY,
            name          TEXT NOT NULL UNIQUE,
            description   TEXT NOT NULL DEFAULT '',
            is_blank      INTEGER NOT NULL DEFAULT 0,
            is_global     INTEGER NOT NULL DEFAULT 0,
            provider      TEXT NOT NULL DEFAULT '',
            model         TEXT NOT NULL DEFAULT '',
            instructions  TEXT NOT NULL DEFAULT '',
            instructions_by_provider TEXT NOT NULL DEFAULT '{}',
            context_files TEXT NOT NULL DEFAULT '[]',
            mcp_servers   TEXT NOT NULL DEFAULT '[]',
            skills        TEXT NOT NULL DEFAULT '[]',
            sort_order    INTEGER NOT NULL DEFAULT 0,
            created_at    INTEGER NOT NULL DEFAULT 0,
            updated_at    INTEGER NOT NULL DEFAULT 0,
            is_system     INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_ss_bundles_is_blank
            ON db_bundles(is_blank);

        CREATE TABLE IF NOT EXISTS db_drone_definitions (
            id          TEXT PRIMARY KEY,
            name        TEXT NOT NULL,
            description TEXT NOT NULL DEFAULT '',
            graph       TEXT NOT NULL DEFAULT '{\"nodes\":[],\"edges\":[]}',
            viewport    TEXT NOT NULL DEFAULT '{\"x\":0,\"y\":0,\"zoom\":1}',
            created_at  INTEGER NOT NULL DEFAULT 0,
            updated_at  INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_ss_drone_definitions_updated
            ON db_drone_definitions(updated_at DESC);

        CREATE TABLE IF NOT EXISTS db_muxbus_credentials (
            id             TEXT PRIMARY KEY DEFAULT 'global',
            cognito_domain TEXT NOT NULL DEFAULT '',
            client_id      TEXT NOT NULL DEFAULT '',
            access_token   TEXT NOT NULL DEFAULT '',
            refresh_token  TEXT NOT NULL DEFAULT '',
            id_token       TEXT NOT NULL DEFAULT '',
            expires_at     INTEGER NOT NULL DEFAULT 0,
            user_email     TEXT NOT NULL DEFAULT '',
            user_sub       TEXT NOT NULL DEFAULT ''
        );

        CREATE TABLE IF NOT EXISTS db_migrations (
            id          TEXT PRIMARY KEY,
            applied_at  TEXT NOT NULL,
            duration_ms INTEGER NOT NULL DEFAULT 0,
            scope       TEXT NOT NULL DEFAULT 'global'
        );

        CREATE TABLE IF NOT EXISTS db_cron_jobs (
            id            TEXT PRIMARY KEY,
            name          TEXT NOT NULL,
            expression    TEXT NOT NULL,
            prompt        TEXT NOT NULL,
            target        TEXT NOT NULL,
            created_by    TEXT NOT NULL DEFAULT '',
            enabled       INTEGER NOT NULL DEFAULT 1,
            last_fired    INTEGER,
            fire_count    INTEGER NOT NULL DEFAULT 0,
            max_fires     INTEGER,
            created_at    INTEGER NOT NULL DEFAULT 0,
            max_age_secs  INTEGER,
            -- v11 (identity M1b): the target's UID (db_agents.id) beside the
            -- slug, resolved ONCE at create time (spec §5.4), dual-written,
            -- not yet read -- a job still fires by `target`. Empty when the
            -- name did not resolve; never guessed.
            target_uid    TEXT NOT NULL DEFAULT '',
            -- v12 (identity M4c-1): the creator's UID from the request's
            -- Caller, beside the created_by name. Empty when unattributed.
            created_by_uid TEXT NOT NULL DEFAULT ''
        );
        CREATE INDEX IF NOT EXISTS idx_ss_cron_jobs_enabled
            ON db_cron_jobs(enabled);

        -- v4: per-agent M2M credential for muxbus Tier-4 binding (see
        -- SHARED_STORE_SCHEMA_VERSION's doc comment above). client_secret is
        -- a Cognito app client secret, not a user password — same trust
        -- tier as MUXBUS_TOKEN in db_muxbus_credentials above, stored
        -- unencrypted in the same shared store.db for the same reason.
        CREATE TABLE IF NOT EXISTS db_agent_credentials (
            agent_id       TEXT PRIMARY KEY,
            client_id      TEXT NOT NULL DEFAULT '',
            client_secret  TEXT NOT NULL DEFAULT '',
            token_endpoint TEXT NOT NULL DEFAULT '',
            access_token   TEXT NOT NULL DEFAULT '',
            expires_at     INTEGER NOT NULL DEFAULT 0,
            created_at     INTEGER NOT NULL DEFAULT 0
        );

        -- v6: durable mirror of each agent's native (Claude Code) memory
        -- files, keyed by the stable AgentDefinition.id rather than any live
        -- filesystem path — the live path is channel-relative by design
        -- (per-build-channel filesystem isolation), so the same agent opened
        -- from two channels resolves two different on-disk memory dirs. This
        -- table is what makes native memory durable and location-consistent
        -- across channels: agent:memory:list/read_file upsert into it on
        -- every read, and merge the live-FS view with it in the response —
        -- see docs/specs/SPEC_NATIVE_MEMORY_DURABLE_SYNC_2026_08_07.md.
        -- Duplicated into run_object_schema (same both-schemas pattern as
        -- db_agent_credentials above) for id_store's per-channel fallback
        -- before 0011_shared_store_backfill has run.
        CREATE TABLE IF NOT EXISTS db_agent_native_memory (
            agent_id           TEXT NOT NULL,
            filename           TEXT NOT NULL,
            content            TEXT NOT NULL,
            metadata_type      TEXT NOT NULL DEFAULT '',
            size_bytes         INTEGER NOT NULL DEFAULT 0,
            updated_at         INTEGER NOT NULL DEFAULT 0,
            last_seen_path     TEXT NOT NULL DEFAULT '',
            last_seen_mtime_ms INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (agent_id, filename)
        );

        -- v8: append-only version history for native memory content — one
        -- row per agent:memory:write_file call (plus reverts), so a prior
        -- value is never lost the way the mirror above loses it on every
        -- overwrite. See docs/specs/SPEC_MEMORY_VERSION_CONTROL_AND_ARMORY_AUDIT_2026_08_19.md.
        CREATE TABLE IF NOT EXISTS db_agent_native_memory_versions (
            id                 TEXT PRIMARY KEY,
            agent_id           TEXT NOT NULL,
            filename           TEXT NOT NULL,
            content            TEXT NOT NULL,
            content_hash       TEXT NOT NULL,
            parent_version_id  TEXT,
            source             TEXT NOT NULL DEFAULT 'agent_inferred',
            source_detail      TEXT NOT NULL DEFAULT '{}',
            session_id         TEXT NOT NULL DEFAULT '',
            created_at         INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_ss_native_memory_versions_lookup
            ON db_agent_native_memory_versions(agent_id, filename, created_at);

        -- v10: append-only version history for Global Memory entry content
        -- — see SHARED_STORE_SCHEMA_VERSION's own doc comment for the full
        -- rationale (SPEC_AGENT_FACING_GLOBAL_MEMORY_API_2026_09_15.md).
        CREATE TABLE IF NOT EXISTS db_bundle_versions (
            id                 TEXT PRIMARY KEY,
            bundle_id          TEXT NOT NULL,
            name               TEXT NOT NULL,
            instructions       TEXT NOT NULL,
            content_hash       TEXT NOT NULL,
            parent_version_id  TEXT,
            source             TEXT NOT NULL DEFAULT 'agent_inferred',
            source_detail      TEXT NOT NULL DEFAULT '{}',
            written_by         TEXT NOT NULL DEFAULT '',
            created_at         INTEGER NOT NULL DEFAULT 0,
            -- v12 (identity M4c-1): the writer's UID from the request's
            -- Caller, beside the written_by name. Empty when unattributed.
            written_by_uid     TEXT NOT NULL DEFAULT ''
        );
        CREATE INDEX IF NOT EXISTS idx_ss_bundle_versions_lookup
            ON db_bundle_versions(bundle_id, created_at);",
    )?;

    // Seed the blank Bundle singleton — same fixed id as objects.db so
    // cross-version reads never see a missing blank row.
    conn.execute_batch(
        "INSERT OR IGNORE INTO db_bundles
            (id, name, description, is_blank, created_at, updated_at)
         VALUES ('blank', '__blank__', 'Vanilla CLI — no instructions, no context', 1, 0, 0);",
    )?;

    // ---- Additive column migrations (schema v5+) ----
    // Same idempotent pattern as run_object_schema's ALTER-TABLE loop below:
    // the flat CREATE batch above covers fresh DBs; these carry an existing
    // shared store forward. "duplicate column" is swallowed so this is safe
    // to run on every startup regardless of whether the column already exists.
    //
    // v5: db_cron_jobs.max_age_secs — optional hard expiry bound (seconds
    //     since created_at), alongside the existing max_fires bound. NULL
    //     for all existing rows = no behavior change for jobs created before
    //     this column existed.
    for stmt in [
        "ALTER TABLE db_cron_jobs ADD COLUMN max_age_secs INTEGER",
        // v7: provider-scoped bundle instructions (ABF v0.2 §2.2).
        "ALTER TABLE db_bundles ADD COLUMN instructions_by_provider TEXT NOT NULL DEFAULT '{}'",
        // v9: AgentMux-controlled, highest-priority Global Bundle tier —
        // see OBJECT_SCHEMA_VERSION's v27 doc comment above.
        "ALTER TABLE db_bundles ADD COLUMN is_system INTEGER NOT NULL DEFAULT 0",
        // v11: identity M1b — the cron target's UID beside its slug.
        "ALTER TABLE db_cron_jobs ADD COLUMN target_uid TEXT NOT NULL DEFAULT ''",
        // v12: identity M4c-1 — the actor's UID beside each actor name.
        "ALTER TABLE db_cron_jobs ADD COLUMN created_by_uid TEXT NOT NULL DEFAULT ''",
        "ALTER TABLE db_bundle_versions ADD COLUMN written_by_uid TEXT NOT NULL DEFAULT ''",
        "ALTER TABLE db_agent_native_memory ADD COLUMN last_seen_mtime_ms INTEGER NOT NULL DEFAULT 0", // a table made before this column
    ].iter().chain(super::cron::JOB_TABLES.iter()) {
        if let Err(e) = conn.execute_batch(stmt) {
            let msg = e.to_string();
            if !msg.contains("duplicate column") {
                return Err(e.into());
            }
        }
    }

    Ok(())
}

/// `user_version` stamped into the new, permanently-global
/// `~/.agentmux/shared/identity-store.db` (distinct file from
/// `store.db` — see [`run_identity_store_schema`]'s own doc comment for
/// why). Independent counter from `SHARED_STORE_SCHEMA_VERSION`.
///   v1 — initial: agent→account links, bundles, drone
///        definitions, muxbus credentials, per-agent M2M credentials,
///        native memory mirror, cron jobs. Everything
///        `run_shared_store_schema` already held EXCEPT `db_accounts` —
///        see docs/specs/SPEC_IDENTITY_STORE_SPLIT_2026_08_17.md §3.1.
///   v2 — db_accounts, as a FALLBACK MIRROR only (reagentx P0 review on
///        PR #2632): the link resolves via this store now, but the
///        account row it points to could still only exist in the
///        per-channel-isolatable `id_store`, which is empty on a fresh
///        channel — the reported bug wasn't actually fixed end to end
///        without this. `id_store` remains the PRIMARY, authoritative,
///        write path (so Armory's disposable-test-account isolation is
///        unchanged); this copy is consulted only when `id_store` doesn't
///        have the row. See `identity::resolver::inject`'s account
///        resolution for the read-with-fallback + write-back-to-source-store
///        logic this enables.
///   v3 — db_agent_native_memory_versions, for schema parity with the v1
///        native memory mirror this store already carries. Same as that
///        table, this store's copy is not an actively-written duplicate
///        (id_store/store.db remains the authoritative write path — see
///        v2's note above) — added here only so a future always-global
///        read path (mirroring what v2 did for db_accounts) is not blocked
///        on a missing table. See
///        docs/specs/SPEC_MEMORY_VERSION_CONTROL_AND_ARMORY_AUDIT_2026_08_19.md.
///   v4 — db_bundles.is_system, for schema parity with OBJECT_SCHEMA_VERSION
///        v27 / SHARED_STORE_SCHEMA_VERSION v9 (same column, same reasoning
///        as v3 above — this store's `db_bundles` copy is not an actively-
///        written duplicate either). Real ALTER TABLE ADD COLUMN added to
///        `run_identity_store_schema` in the same change, so this counter
///        MUST bump alongside it — an unbumped counter would leave an
///        existing identity-store.db stamped at the old user_version
///        forever, silently defeating check_schema_compat/stamp_version's
///        forward-compat lock for this one store (reagent P1, PR #2782).
///        See docs/specs/SPEC_GLOBAL_MEMORY_SYSTEM_TIER_2026_08_24.md.
/// v5: `db_work_queue` — the Muxqueue backlog
///        (`docs/reports/REPORT_UNIVERSAL_AGENT_WORK_QUEUE_2026_09_01.md`).
///        A NEW table, so no ALTER is involved, but the counter still bumps:
///        an identity-store.db stamped v4 that a v5 binary opens must be
///        recognized as older, not "already current" (same reagent P1 on
///        PR #2782 reasoning recorded for v4 above).
///
///        Deliberately in THIS store, not the per-channel one. A queue whose
///        rows are per-channel means "any agent IN THIS CHANNEL can pick it
///        up", which is close to useless given every local/dev/portable build
///        is its own channel — see the report's §6.2, and
///        `docs/analysis/ANALYSIS_PER_CHANNEL_AUTH_BYPASSES_2026_08_31.md`
///        for what the per-channel/global seam has already cost once.
/// v6: `db_skills`, `db_mcp_servers` — Phase 2 of
///        `SPEC_DURABLE_BINDINGS_2026_09_10.md`. These become the
///        AUTHORITATIVE catalogs as of this version; the per-channel copies
///        in `run_object_schema` keep their declarations (never removed —
///        see that spec's §5.1 and §8 item 3) and continue to exist only as
///        the same best-effort degraded-mode fallback
///        `bootstrap/stores.rs` already substitutes for every other
///        identity-store table (`mstore` in
///        place of `identity_store` when the identity store can't be
///        resolved, created, or opened). Deliberately NOT declared with the
///        ref tables (`db_agent_skills_ref`, `db_bundle_skills_ref`, and
///        the MCP equivalents) — those stay channel-scoped until Phase 3/6
///        promotes them; adding them here now, unpaired with a live writer,
///        would be schema for a phase that hasn't shipped.
/// v7: partial UNIQUE indexes on `db_skills(name, skill_type)` and
///        `db_mcp_servers(name)`, each `WHERE is_global = 1` — global rows
///        only; two different owners' private rows sharing a name is still
///        fine. Closes a real race in
///        `m0031_carry_skills_and_mcp_servers_to_identity_store`: two
///        channels' migrations can run concurrently (multiple AgentMux
///        instances is an explicitly supported scenario), both read "no
///        matching global row yet" before either has inserted, and both
///        proceed to insert under different ids — the promised first-wins
///        cross-channel dedup silently fails without a DB-level constraint
///        to arbitrate the race (Codex P1, PR #3181). The migration's
///        insert now handles the resulting `UNIQUE constraint failed` by
///        re-querying for whichever row actually won.
///   v8 — `idx_ids_skills_global_name` replaces `idx_ids_skills_global_name_type`:
///        narrows the skill global-uniqueness index from `(name, skill_type)`
///        to `name` alone, matching the invariant `skill_upsert_unique_global`'s
///        own dup-check has always enforced (`WHERE name = ?1 AND id <> ?2
///        AND is_global = 1` — no `skill_type` filter). Codex P2, PR #3182,
///        `SPEC_DURABLE_BINDINGS_2026_09_10.md` §5.4: the v7 index was
///        narrower than the invariant it was meant to back, so a
///        same-name-different-`skill_type` race between two channels could
///        insert two rows the application itself considers duplicates. MCP
///        servers don't have this mismatch — `idx_ids_mcp_servers_global_name`
///        is already name-only — so this bump is skills-only.
///        `m0033_narrow_skill_global_uniqueness_index` defensively collapses
///        any existing same-name/different-type pair `m0031`'s own
///        `(name, skill_type)`-keyed dedup may already have produced, to the
///        earlier-`created_at` row, before the new index is created.
///   v9 — db_bundle_versions, for schema parity with OBJECT_SCHEMA_VERSION
///        v35 / SHARED_STORE_SCHEMA_VERSION v10 (same table, same reasoning
///        as v3 above — this store's copy is not an actively-written
///        duplicate either). See
///        docs/specs/SPEC_AGENT_FACING_GLOBAL_MEMORY_API_2026_09_15.md.
///   v10 — db_work_queue.target_agent_uid / claimed_by_uid, and
///        db_cron_jobs.target_uid (parity with SHARED_STORE_SCHEMA_VERSION
///        v11): the UID (`db_agents.id`) beside each slug column, dual-written
///        and not yet read. Identity M1b of
///        SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED_2026_09_23.md §9.1.
///        `target_agent_uid` is resolved ONCE at enqueue (spec §5.4);
///        `claimed_by_uid` is CARRIED from the claimer's own
///        `AGENTMUX_AGENT_UID` (M1a, #3548), falling back to resolving its
///        `agent_id` with the fallback counted (spec §9.2). Empty when
///        unknown; never guessed.
///   v11 — db_work_queue.created_by_uid, and db_cron_jobs.created_by_uid /
///        db_bundle_versions.written_by_uid (parity with
///        SHARED_STORE_SCHEMA_VERSION v12): the actor's UID beside each
///        actor-name column, taken from the request's `Caller` (its
///        `X-Agent-Token`), never from a body field. Empty for an
///        Unattributed request; never guessed. Dual-written, not yet read.
///        Identity M4c-1 of
///        SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED_2026_09_23.md §6.5.9.
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

/// Initialize (or re-validate) the `~/.agentmux/shared/identity-store.db`
/// schema — the permanently-global store introduced by
/// `docs/specs/SPEC_IDENTITY_STORE_SPLIT_2026_08_17.md` to fix
/// conversation-continuity-across-a-version-switch
/// (`docs/specs/REPORT_HISTORY_CONTINUITY_ACROSS_VERSION_UPGRADE_2026_08_17.md`).
///
/// This is a **near-superset** of [`run_shared_store_schema`]'s tables
/// (v2): every one of them, INCLUDING `db_accounts` as of v2 — but
/// `db_accounts` here is a read-through FALLBACK MIRROR only, not the
/// authoritative write path. Unlike agent→account links, bundles,
/// drone definitions, muxbus credentials, and native memory,
/// `db_accounts` has one genuine, intentional isolation need (Armory's
/// disposable delete-account testing flow): a real per-account
/// creation-time scope split (SPEC §3.2) hasn't shipped yet, so
/// `id_store`/`resolve_shared_store_path()` stays the primary read/write
/// path for accounts, preserving Armory's isolation exactly as before.
/// This store's copy exists ONLY so a link resolved here (always global)
/// doesn't dead-end on an account row that's stuck in a fresh, per-
/// channel-isolated `id_store` — see `identity::resolver::inject`'s
/// account resolution (falls back to this store's `identity_get` only
/// when `id_store`'s own lookup misses) and
/// `migrations::m0022_identity_store_links_backfill` (backfills this
/// mirror the same way it backfills links). `db_agent_identity_links.account_id`
/// still has NO foreign key here (same reasoning `run_shared_store_schema`
/// already uses to drop its FK to `db_agent_definitions`: cross-database
/// FK enforcement is impossible in SQLite, and this table's `db_accounts`
/// is itself just a mirror, not guaranteed complete — application code
/// enforces referential integrity, tolerating a miss, instead).
///
/// A dedicated store (not a subset of the existing shared store reached
/// via a new always-global path) is deliberate, not just a naming choice:
/// it makes "can this table ever be reached from a per-channel-isolated
/// codepath" a structural fact (a totally separate file/connection)
/// rather than a convention someone could violate by calling
/// `resolve_shared_store_path()` for a new call site later — the exact
/// failure mode `docs/specs/CHECKLIST_AGENT_DATA_SCOPE_ROUTING_2026_08_17.md`
/// items 2-4 already document as having bitten this codebase more than
/// once.
///
/// Idempotent — safe to call on every startup.
pub fn run_identity_store_schema(conn: &Connection) -> Result<(), StoreError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS db_agent_identity_links (
            agent_id   TEXT NOT NULL,
            account_id TEXT NOT NULL,
            provider   TEXT NOT NULL,
            PRIMARY KEY (agent_id, provider)
        );
        CREATE INDEX IF NOT EXISTS idx_ids_agent_identity_links_account
            ON db_agent_identity_links(account_id);

        -- v2: fallback mirror only — see this function's own doc comment
        -- above. id_store stays the authoritative read/write path.
        CREATE TABLE IF NOT EXISTS db_accounts (
            id           TEXT PRIMARY KEY,
            name         TEXT NOT NULL,
            provider     TEXT NOT NULL,
            kind         TEXT NOT NULL,
            display_name TEXT NOT NULL DEFAULT '',
            secret_ref   TEXT NOT NULL,
            context      TEXT NOT NULL DEFAULT '{}',
            status       TEXT NOT NULL DEFAULT 'unknown',
            created_at   INTEGER NOT NULL DEFAULT 0,
            updated_at   INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_ids_accounts_provider
            ON db_accounts(provider);

        CREATE TABLE IF NOT EXISTS db_bundles (
            id            TEXT PRIMARY KEY,
            name          TEXT NOT NULL UNIQUE,
            description   TEXT NOT NULL DEFAULT '',
            is_blank      INTEGER NOT NULL DEFAULT 0,
            is_global     INTEGER NOT NULL DEFAULT 0,
            provider      TEXT NOT NULL DEFAULT '',
            model         TEXT NOT NULL DEFAULT '',
            instructions  TEXT NOT NULL DEFAULT '',
            instructions_by_provider TEXT NOT NULL DEFAULT '{}',
            context_files TEXT NOT NULL DEFAULT '[]',
            mcp_servers   TEXT NOT NULL DEFAULT '[]',
            skills        TEXT NOT NULL DEFAULT '[]',
            sort_order    INTEGER NOT NULL DEFAULT 0,
            created_at    INTEGER NOT NULL DEFAULT 0,
            updated_at    INTEGER NOT NULL DEFAULT 0,
            is_system     INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_ids_bundles_is_blank
            ON db_bundles(is_blank);

        CREATE TABLE IF NOT EXISTS db_drone_definitions (
            id          TEXT PRIMARY KEY,
            name        TEXT NOT NULL,
            description TEXT NOT NULL DEFAULT '',
            graph       TEXT NOT NULL DEFAULT '{\"nodes\":[],\"edges\":[]}',
            viewport    TEXT NOT NULL DEFAULT '{\"x\":0,\"y\":0,\"zoom\":1}',
            created_at  INTEGER NOT NULL DEFAULT 0,
            updated_at  INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_ids_drone_definitions_updated
            ON db_drone_definitions(updated_at DESC);

        CREATE TABLE IF NOT EXISTS db_muxbus_credentials (
            id             TEXT PRIMARY KEY DEFAULT 'global',
            cognito_domain TEXT NOT NULL DEFAULT '',
            client_id      TEXT NOT NULL DEFAULT '',
            access_token   TEXT NOT NULL DEFAULT '',
            refresh_token  TEXT NOT NULL DEFAULT '',
            id_token       TEXT NOT NULL DEFAULT '',
            expires_at     INTEGER NOT NULL DEFAULT 0,
            user_email     TEXT NOT NULL DEFAULT '',
            user_sub       TEXT NOT NULL DEFAULT ''
        );

        CREATE TABLE IF NOT EXISTS db_migrations (
            id          TEXT PRIMARY KEY,
            applied_at  TEXT NOT NULL,
            duration_ms INTEGER NOT NULL DEFAULT 0,
            scope       TEXT NOT NULL DEFAULT 'global'
        );

        CREATE TABLE IF NOT EXISTS db_cron_jobs (
            id            TEXT PRIMARY KEY,
            name          TEXT NOT NULL,
            expression    TEXT NOT NULL,
            prompt        TEXT NOT NULL,
            target        TEXT NOT NULL,
            created_by    TEXT NOT NULL DEFAULT '',
            enabled       INTEGER NOT NULL DEFAULT 1,
            last_fired    INTEGER,
            fire_count    INTEGER NOT NULL DEFAULT 0,
            max_fires     INTEGER,
            created_at    INTEGER NOT NULL DEFAULT 0,
            max_age_secs  INTEGER,
            -- v10 (identity M1b): UID beside the slug, resolved at create
            -- time, dual-written, not yet read. Parity with the shared-store
            -- copy of this table, which is the one the handlers use today.
            target_uid    TEXT NOT NULL DEFAULT '',
            -- v11 (identity M4c-1): the creator's UID, parity with the
            -- shared-store copy.
            created_by_uid TEXT NOT NULL DEFAULT ''
        );
        CREATE INDEX IF NOT EXISTS idx_ids_cron_jobs_enabled
            ON db_cron_jobs(enabled);

        -- Muxqueue: the universal agent work queue. One row = one unit of work
        -- that ANY agent may claim. See
        -- docs/reports/REPORT_UNIVERSAL_AGENT_WORK_QUEUE_2026_09_01.md.
        --
        -- Deliberately placed directly after db_cron_jobs, because it is that
        -- table's sibling: cron is this row with a TIME trigger, this is the
        -- same row with a READINESS trigger. `not_before` makes the
        -- relationship literal.
        --
        -- It belongs in THIS schema (the always-global identity store) and not
        -- in run_shared_store_schema. A per-channel queue would only ever mean
        -- any agent IN THIS CHANNEL can pick it up, which is close to useless
        -- when every local/dev/portable build is its own channel -- see the
        -- report's section 6.2.
        --
        -- Editing note: this whole schema is ONE Rust string literal passed to
        -- execute_batch, so an UNESCAPED double quote anywhere in it --
        -- including inside an SQL comment like this one -- terminates that
        -- literal and breaks the build with errors that point at prose rather
        -- than at code. Escaped ones are fine and already appear above (see
        -- db_drone_definitions default JSON). Apostrophes are fine too: they
        -- are ordinary characters inside a Rust double-quoted string, and
        -- SQLite does not lex quotes inside a comment.
        CREATE TABLE IF NOT EXISTS db_work_queue (
            id            TEXT PRIMARY KEY,
            title         TEXT NOT NULL,
            payload       TEXT NOT NULL,
            kind          TEXT NOT NULL DEFAULT '',
            target_agent  TEXT NOT NULL DEFAULT '',
            target_group  TEXT NOT NULL DEFAULT '',
            priority      INTEGER NOT NULL DEFAULT 0,
            state         TEXT NOT NULL DEFAULT 'open',
            claimed_by    TEXT NOT NULL DEFAULT '',
            claim_expires INTEGER,
            attempts      INTEGER NOT NULL DEFAULT 0,
            max_attempts  INTEGER NOT NULL DEFAULT 3,
            created_by    TEXT NOT NULL DEFAULT '',
            created_at    INTEGER NOT NULL DEFAULT 0,
            updated_at    INTEGER NOT NULL DEFAULT 0,
            not_before    INTEGER,
            result        TEXT NOT NULL DEFAULT '',
            -- v10 (identity M1b): the UID (db_agents.id) beside each slug
            -- column, dual-written, not yet read. target_agent_uid is
            -- resolved ONCE at enqueue (spec §5.4: resolution happens at
            -- authoring time, when a human or model was present, never at
            -- claim time); claimed_by_uid is CARRIED from the claimer's own
            -- AGENTMUX_AGENT_UID. Empty when unknown -- never guessed.
            target_agent_uid TEXT NOT NULL DEFAULT '',
            claimed_by_uid   TEXT NOT NULL DEFAULT '',
            -- v11 (identity M4c-1): the enqueuer's UID from the request's
            -- Caller, beside the created_by name. Empty when unattributed.
            created_by_uid   TEXT NOT NULL DEFAULT ''
        );
        -- The claim query's exact predicate: state + readiness, ordered by
        -- priority then age. Without this the claim UPDATE degrades to a table
        -- scan under contention, which is when it matters most.
        CREATE INDEX IF NOT EXISTS idx_ids_work_queue_claimable
            ON db_work_queue(state, not_before, priority DESC, created_at);
        -- The reaper's predicate (expired leases).
        CREATE INDEX IF NOT EXISTS idx_ids_work_queue_claim_expires
            ON db_work_queue(claim_expires)
            WHERE claim_expires IS NOT NULL;

        CREATE TABLE IF NOT EXISTS db_agent_credentials (
            agent_id       TEXT PRIMARY KEY,
            client_id      TEXT NOT NULL DEFAULT '',
            client_secret  TEXT NOT NULL DEFAULT '',
            token_endpoint TEXT NOT NULL DEFAULT '',
            access_token   TEXT NOT NULL DEFAULT '',
            expires_at     INTEGER NOT NULL DEFAULT 0,
            created_at     INTEGER NOT NULL DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS db_agent_native_memory (
            agent_id           TEXT NOT NULL,
            filename           TEXT NOT NULL,
            content            TEXT NOT NULL,
            metadata_type      TEXT NOT NULL DEFAULT '',
            size_bytes         INTEGER NOT NULL DEFAULT 0,
            updated_at         INTEGER NOT NULL DEFAULT 0,
            last_seen_path     TEXT NOT NULL DEFAULT '',
            last_seen_mtime_ms INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (agent_id, filename)
        );

        -- v3: schema parity with the v1 native memory mirror above — see
        -- IDENTITY_STORE_SCHEMA_VERSION's own doc comment for why this
        -- store's copy is not (yet) an actively-written duplicate.
        CREATE TABLE IF NOT EXISTS db_agent_native_memory_versions (
            id                 TEXT PRIMARY KEY,
            agent_id           TEXT NOT NULL,
            filename           TEXT NOT NULL,
            content            TEXT NOT NULL,
            content_hash       TEXT NOT NULL,
            parent_version_id  TEXT,
            source             TEXT NOT NULL DEFAULT 'agent_inferred',
            source_detail      TEXT NOT NULL DEFAULT '{}',
            session_id         TEXT NOT NULL DEFAULT '',
            created_at         INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_ids_native_memory_versions_lookup
            ON db_agent_native_memory_versions(agent_id, filename, created_at);

        -- v9: schema parity with the db_bundle_versions table above (own
        -- store) — see IDENTITY_STORE_SCHEMA_VERSION's own doc comment.
        CREATE TABLE IF NOT EXISTS db_bundle_versions (
            id                 TEXT PRIMARY KEY,
            bundle_id          TEXT NOT NULL,
            name               TEXT NOT NULL,
            instructions       TEXT NOT NULL,
            content_hash       TEXT NOT NULL,
            parent_version_id  TEXT,
            source             TEXT NOT NULL DEFAULT 'agent_inferred',
            source_detail      TEXT NOT NULL DEFAULT '{}',
            written_by         TEXT NOT NULL DEFAULT '',
            created_at         INTEGER NOT NULL DEFAULT 0,
            -- v11 (identity M4c-1): parity with the shared-store copy.
            written_by_uid     TEXT NOT NULL DEFAULT ''
        );
        CREATE INDEX IF NOT EXISTS idx_ids_bundle_versions_lookup
            ON db_bundle_versions(bundle_id, created_at);

        -- v6: authoritative skill/MCP-server catalogs — see this constant's
        -- v6 doc comment above. Same column shapes as run_object_schema's
        -- copies; no ref tables here yet (Phase 3/6), so no FK targets exist
        -- to reference them from this store.
        CREATE TABLE IF NOT EXISTS db_skills (
            id          TEXT PRIMARY KEY,
            name        TEXT NOT NULL,
            trigger     TEXT NOT NULL DEFAULT '',
            skill_type  TEXT NOT NULL DEFAULT 'prompt',
            description TEXT NOT NULL DEFAULT '',
            content     TEXT NOT NULL DEFAULT '',
            is_global   INTEGER NOT NULL DEFAULT 0,
            created_at  INTEGER NOT NULL DEFAULT 0,
            updated_at  INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_ids_skills_is_global ON db_skills(is_global);
        -- v8: name-only, matching skill_upsert_unique_global's own dup-check
        -- — see this constant's v8 doc comment. Replaces
        -- idx_ids_skills_global_name_type (dropped just below for an
        -- existing store; a fresh store never creates it in the first
        -- place, since this whole batch runs as one execute_batch).
        CREATE UNIQUE INDEX IF NOT EXISTS idx_ids_skills_global_name
            ON db_skills(name) WHERE is_global = 1;
        DROP INDEX IF EXISTS idx_ids_skills_global_name_type;

        CREATE TABLE IF NOT EXISTS db_mcp_servers (
            id          TEXT PRIMARY KEY,
            name        TEXT NOT NULL,
            transport   TEXT NOT NULL DEFAULT 'stdio',
            config      TEXT NOT NULL DEFAULT '{}',
            is_global   INTEGER NOT NULL DEFAULT 0,
            created_at  INTEGER NOT NULL DEFAULT 0,
            updated_at  INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_ids_mcp_servers_is_global ON db_mcp_servers(is_global);
        CREATE UNIQUE INDEX IF NOT EXISTS idx_ids_mcp_servers_global_name
            ON db_mcp_servers(name) WHERE is_global = 1;
        -- Who a catalog server reaches beyond its binds. Added without a
        -- version bump — see IDENTITY_STORE_SCHEMA_VERSION's doc comment.
        CREATE TABLE IF NOT EXISTS db_mcp_server_reach (
            mcp_id        TEXT PRIMARY KEY,
            all_agents    INTEGER NOT NULL DEFAULT 0,
            in_containers INTEGER NOT NULL DEFAULT 0,
            updated_at    INTEGER NOT NULL DEFAULT 0
        );",
    )?;
    conn.execute_batch(super::vault::VAULT_SCHEMA)?; // The Vault's tables, no version bump: storage/vault.rs.
    // Seed the blank Bundle singleton — same fixed id as objects.db/store.db
    // so cross-version reads never see a missing blank row.
    conn.execute_batch(
        "INSERT OR IGNORE INTO db_bundles
            (id, name, description, is_blank, created_at, updated_at)
         VALUES ('blank', '__blank__', 'Vanilla CLI — no instructions, no context', 1, 0, 0);",
    )?;

    // Additive columns added after this store's tables were first created —
    // same idempotent pattern as run_shared_store_schema's own ALTER loop.
    for stmt in [
        // db_bundles.is_system: see OBJECT_SCHEMA_VERSION's v27 doc comment.
        "ALTER TABLE db_bundles ADD COLUMN is_system INTEGER NOT NULL DEFAULT 0",
        // v10: identity M1b — UID columns beside the slug columns, dual-written.
        "ALTER TABLE db_work_queue ADD COLUMN target_agent_uid TEXT NOT NULL DEFAULT ''",
        "ALTER TABLE db_work_queue ADD COLUMN claimed_by_uid TEXT NOT NULL DEFAULT ''",
        "ALTER TABLE db_cron_jobs ADD COLUMN target_uid TEXT NOT NULL DEFAULT ''",
        // v11: identity M4c-1 — the actor's UID beside each actor name.
        "ALTER TABLE db_work_queue ADD COLUMN created_by_uid TEXT NOT NULL DEFAULT ''",
        "ALTER TABLE db_cron_jobs ADD COLUMN created_by_uid TEXT NOT NULL DEFAULT ''",
        "ALTER TABLE db_bundle_versions ADD COLUMN written_by_uid TEXT NOT NULL DEFAULT ''",
        "ALTER TABLE db_agent_native_memory ADD COLUMN last_seen_mtime_ms INTEGER NOT NULL DEFAULT 0", // a table made before this column
    ].iter().chain(super::cron::JOB_TABLES.iter()).chain(super::vault::VAULT_ALTERS.iter()) {
        if let Err(e) = conn.execute_batch(stmt) {
            let msg = e.to_string();
            if !msg.contains("duplicate column") {
                return Err(e.into());
            }
        }
    }

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
    for stmt in &[
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
    ] {
        if let Err(e) = conn.execute_batch(stmt) {
            if !e.to_string().contains("duplicate column") {
                return Err(e.into());
            }
        }
    }
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

/// `PRAGMA user_version` tripwire (AUDIT_SQLITE_SYSTEMS §8.5).
///
/// Compare the file's `user_version` to `current` and refuse to open
/// the database if it was written by a NEWER binary.
///
/// This is the forward-compat **safety lock** from the channels design
/// (`SPEC_DATA_CHANNELS_2026_05_24.md` §3.3). Within a channel,
/// multiple released AgentMux versions share one data dir; the lock
/// keeps an older binary from writing into a schema laid down by a
/// newer binary. Same discipline as Chrome's profile-too-new check
/// and Postgres's catalog version mismatch.
///
/// **Must be called BEFORE `run_*_schema` / `run_*_migrations`.** The
/// `run_*` functions include mutating steps (legacy-table renames,
/// seed inserts, `CREATE TABLE` for new tables the older binary
/// doesn't have), so if we ran them first and only then checked the
/// version, a downgraded binary could still alter a newer database
/// before the error fires — breaking the "reject without touching
/// disk" invariant the lock is meant to guarantee (codex P1 on PR
/// #1029).
///
/// Read-only: only `PRAGMA user_version` query, no writes.
/// [`stamp_version`] does the corresponding write AFTER migrations
/// complete successfully.
///
/// Recovery for the user when this returns `SchemaTooNew`: upgrade
/// AgentMux to a version ≥ `found`, or set
/// `AGENTMUX_CHANNEL=<other>` to land in a different channel dir.
/// The data on disk is preserved either way — this function never
/// modifies the database on the rejected path.
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
