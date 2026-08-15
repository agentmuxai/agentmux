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
//! tripwire (`stamp_and_check_version`).

use rusqlite::Connection;
use tracing::warn;

use super::error::StoreError;

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
///        (see agentmux-cloud's PLAN_PER_AGENT_CREDENTIAL_BINDING_2026_07_06.md)
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
pub const SHARED_STORE_SCHEMA_VERSION: i64 = 7;

/// `user_version` value stamped into `objects.db` after `run_object_schema`.
/// The flat schema reset the counter to 1 (the pre-flatten chain never set
/// `user_version`, so legacy files read 0). Bumped per additive migration:
///   v1 — flat schema baseline
///   v2 — db_agent_definitions.updated_at
///   v3 — db_agent_definitions.user_hidden (Phase 2 hide templates,
///        SPEC_AGENT_PICKER_TWO_TIER_2026_05_24.md Q2 Decision Y)
///   v4 — db_agents consolidation table (Phase 3a; dual-write only,
///        reads still on db_agent_definitions / db_agent_instances)
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
///        global brain (controls CLAUDE.md injection order). Existing
///        rows default to 0; the Brain tab assigns positions via reorder.
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
///        per-agent opt-in letting a Warden Supervisor watcher agent
///        auto-continue this agent's session on turn-end (subject to a
///        server-side consecutive-nudge ceiling). Defaults to 0 (opt-in
///        required — fail-by-default, same posture as use_ambient_login).
///        See docs/analysis/ANALYSIS_WARDEN_AUTO_CONTROLLER_CONTINUATION_WATCHER_2026_08_12.md.
///   v18 — db_agent_jekt_keys: per-agent HMAC-SHA256 signing key for
///        host-tier jekt sender verification. One row per agent_id, minted
///        on first use and injected into that agent's own MCP server
///        process env (AGENTMUX_JEKT_KEY) at spawn — never into any other
///        agent's env, so a signature can only be produced by the agent it
///        claims to be from. See
///        docs/specs/SPEC_JEKT_TRUST_LAYER_COMPLETION_2026_08_13.md §2.2.
///   v19 — db_agent_lan_keys: per-agent Ed25519 keypair for LAN-tier jekt
///        sender verification — mirrors v18, asymmetric instead of HMAC
///        (LAN is multi-party: a receiving peer must verify without being
///        able to forge, which a shared secret can't provide). public_key
///        is distributed to LAN peers on demand (not secret); private_key
///        is injected into that one agent's own MCP process env
///        (AGENTMUX_LAN_KEY) at spawn, same never-over-RPC guarantee as
///        v18. See docs/specs/SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md §2.1.
pub const OBJECT_SCHEMA_VERSION: i64 = 19;
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
/// resolution path (`identity/resolver.rs::resolve_bindings_for_instance`),
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
/// 3. Seeds the blank Memory singleton row.
///
/// A database stuck at a pre-v11 intermediate schema cannot be fully
/// adopted (its tables predate later columns). The adopt step still
/// renames what it finds; the first query referencing a missing column
/// then fails loudly with `no such column` — a hard error, not silent
/// empty state, with the data preserved on disk. Per-version data dirs
/// make that case unreachable for released builds.
pub fn run_object_schema(conn: &Connection) -> Result<(), StoreError> {
    adopt_legacy_table_names(conn)?;

    // ---- Generic StoreObj object tables ----
    for otype in WSTORE_OTYPES {
        conn.execute_batch(&format!(
            "CREATE TABLE IF NOT EXISTS db_{otype} (
                oid     TEXT PRIMARY KEY,
                version INTEGER NOT NULL DEFAULT 1,
                data    TEXT NOT NULL
            );"
        ))?;
    }

    // ---- Agent + identity + memory + drone schema ----
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS db_agent_definitions (
            id                   TEXT PRIMARY KEY,
            slug                 TEXT NOT NULL DEFAULT '',
            name                 TEXT NOT NULL,
            icon                 TEXT NOT NULL DEFAULT '✦',
            provider             TEXT NOT NULL,
            description          TEXT NOT NULL DEFAULT '',
            working_directory    TEXT NOT NULL DEFAULT '',
            shell                TEXT NOT NULL DEFAULT '',
            provider_flags       TEXT NOT NULL DEFAULT '',
            auto_start           INTEGER NOT NULL DEFAULT 0,
            restart_on_crash     INTEGER NOT NULL DEFAULT 0,
            idle_timeout_minutes INTEGER NOT NULL DEFAULT 0,
            agent_type           TEXT NOT NULL DEFAULT 'standalone',
            environment          TEXT NOT NULL DEFAULT '',
            agent_bus_id         TEXT NOT NULL DEFAULT '',
            is_seeded            INTEGER NOT NULL DEFAULT 0,
            accounts             TEXT NOT NULL DEFAULT '',
            parent_id            TEXT NOT NULL DEFAULT '',
            branch_label         TEXT NOT NULL DEFAULT '',
            created_at           INTEGER NOT NULL DEFAULT 0,
            updated_at           INTEGER NOT NULL DEFAULT 0,
            user_hidden          INTEGER NOT NULL DEFAULT 0,
            container_image      TEXT NOT NULL DEFAULT '',
            container_volumes    TEXT NOT NULL DEFAULT '[]',
            container_name       TEXT NOT NULL DEFAULT '',
            use_ambient_login    INTEGER NOT NULL DEFAULT 0,
            model_vendor_base_url TEXT NOT NULL DEFAULT '',
            auto_continue_enabled INTEGER NOT NULL DEFAULT 0
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_agent_definitions_slug
            ON db_agent_definitions(slug);

        CREATE TABLE IF NOT EXISTS db_agent_content (
            agent_id     TEXT NOT NULL,
            content_type TEXT NOT NULL,
            content      TEXT NOT NULL DEFAULT '',
            updated_at   INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (agent_id, content_type),
            FOREIGN KEY (agent_id) REFERENCES db_agent_definitions(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS db_agent_skills (
            id          TEXT PRIMARY KEY,
            agent_id    TEXT NOT NULL,
            name        TEXT NOT NULL,
            trigger     TEXT NOT NULL DEFAULT '',
            skill_type  TEXT NOT NULL DEFAULT 'prompt',
            description TEXT NOT NULL DEFAULT '',
            content     TEXT NOT NULL DEFAULT '',
            created_at  INTEGER NOT NULL DEFAULT 0,
            FOREIGN KEY (agent_id) REFERENCES db_agent_definitions(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS db_agent_history (
            id           INTEGER PRIMARY KEY AUTOINCREMENT,
            agent_id     TEXT NOT NULL,
            session_date TEXT NOT NULL,
            entry        TEXT NOT NULL,
            timestamp    INTEGER NOT NULL DEFAULT 0,
            FOREIGN KEY (agent_id) REFERENCES db_agent_definitions(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_agent_history_agent_date
            ON db_agent_history(agent_id, session_date);

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
        CREATE INDEX IF NOT EXISTS idx_accounts_provider
            ON db_accounts(provider);

        CREATE TABLE IF NOT EXISTS db_agent_identity_links (
            agent_id   TEXT NOT NULL,
            account_id TEXT NOT NULL,
            provider   TEXT NOT NULL,
            PRIMARY KEY (agent_id, provider),
            FOREIGN KEY (agent_id)   REFERENCES db_agent_definitions(id) ON DELETE CASCADE,
            FOREIGN KEY (account_id) REFERENCES db_accounts(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_agent_identity_links_account
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
            updated_at    INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_bundles_is_blank
            ON db_bundles(is_blank);

        CREATE TABLE IF NOT EXISTS db_agent_instances (
            id                 TEXT PRIMARY KEY,
            definition_id      TEXT NOT NULL,
            parent_instance_id TEXT NOT NULL DEFAULT '',
            block_id           TEXT NOT NULL DEFAULT '',
            session_id         TEXT NOT NULL DEFAULT '',
            status             TEXT NOT NULL DEFAULT 'running',
            github_context     TEXT NOT NULL DEFAULT '',
            identity_id        TEXT NOT NULL DEFAULT '',
            memory_id          TEXT NOT NULL DEFAULT '',
            instance_name      TEXT NOT NULL DEFAULT '',
            working_directory  TEXT NOT NULL DEFAULT '',
            display_hidden     INTEGER NOT NULL DEFAULT 0,
            started_at         INTEGER NOT NULL DEFAULT 0,
            ended_at           INTEGER NOT NULL DEFAULT 0,
            created_at         INTEGER NOT NULL DEFAULT 0,
            FOREIGN KEY (definition_id) REFERENCES db_agent_definitions(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_agent_instances_definition
            ON db_agent_instances(definition_id);
        CREATE INDEX IF NOT EXISTS idx_agent_instances_block
            ON db_agent_instances(block_id);
        CREATE INDEX IF NOT EXISTS idx_agent_instances_status
            ON db_agent_instances(status);
        CREATE INDEX IF NOT EXISTS idx_agent_instances_parent
            ON db_agent_instances(parent_instance_id);
        CREATE INDEX IF NOT EXISTS idx_agent_instances_name_recent
            ON db_agent_instances(instance_name, started_at DESC)
            WHERE display_hidden = 0 AND instance_name != '';

        -- Phase 3a consolidation: `db_agents` collapses `db_agent_definitions`
        -- + `db_agent_instances` into one table per
        -- `docs/specs/SPEC_AGENT_CONCEPT_CONSOLIDATION_2026_05_24.md`.
        -- WRITE-ONLY in 3a: every old-table mutation dual-writes here, but
        -- every read still hits the old tables. Phase 3b migrates readers,
        -- Phase 3c drops the old tables. Column names align with the live
        -- `db_agent_definitions` shape (provider, working_directory, …) —
        -- NOT the inline draft in the spec (provider_id, cmd, cmd_args, …) —
        -- because the existing storage layer never grew the cmd-template
        -- columns the spec sketched; carrying the names we actually have
        -- avoids inventing data we don't store. See the PR body for the
        -- field-by-field mapping.
        CREATE TABLE IF NOT EXISTS db_agents (
            id                   TEXT PRIMARY KEY,
            name                 TEXT NOT NULL,
            icon                 TEXT NOT NULL DEFAULT '',
            description          TEXT NOT NULL DEFAULT '',

            -- Template vs user agent
            is_template          INTEGER NOT NULL DEFAULT 0,
            parent_template_id   TEXT NOT NULL DEFAULT '',

            -- Provider/cmd config (was on definition; named to match the
            -- live `db_agent_definitions` columns).
            provider             TEXT NOT NULL,
            provider_flags       TEXT NOT NULL DEFAULT '',
            shell                TEXT NOT NULL DEFAULT '',
            environment          TEXT NOT NULL DEFAULT '',
            agent_type           TEXT NOT NULL DEFAULT 'standalone',
            agent_bus_id         TEXT NOT NULL DEFAULT '',
            accounts             TEXT NOT NULL DEFAULT '',
            auto_start           INTEGER NOT NULL DEFAULT 0,
            restart_on_crash     INTEGER NOT NULL DEFAULT 0,
            idle_timeout_minutes INTEGER NOT NULL DEFAULT 0,
            slug                 TEXT NOT NULL DEFAULT '',
            branch_label         TEXT NOT NULL DEFAULT '',

            -- Bindings (was on instance — only meaningful when is_template=0).
            -- For template rows these stay empty.
            identity_id          TEXT NOT NULL DEFAULT '',
            memory_id            TEXT NOT NULL DEFAULT '',
            working_directory    TEXT NOT NULL DEFAULT '',
            github_context       TEXT NOT NULL DEFAULT '',
            instance_name        TEXT NOT NULL DEFAULT '',

            -- Latest launch's block (Phase 3c): pointer to the most-recent
            -- session's block so the consolidated read can locate the
            -- conversation snapshot without joining db_agent_instances. The
            -- only transient per-launch field db_agents retains; the rest
            -- (status/session_id/started_at/ended_at) live on the block and
            -- retire with db_agent_instances.
            last_block_id        TEXT NOT NULL DEFAULT '',

            -- Provenance
            created_at           INTEGER NOT NULL DEFAULT 0,
            updated_at           INTEGER NOT NULL DEFAULT 0,
            is_seeded            INTEGER NOT NULL DEFAULT 0,
            user_hidden          INTEGER NOT NULL DEFAULT 0,

            -- Container support (Schema v6 / Phase 0).
            -- Empty for host agents; populated by ContainerManager.
            container_image      TEXT NOT NULL DEFAULT '',
            container_volumes    TEXT NOT NULL DEFAULT '[]',
            container_name       TEXT NOT NULL DEFAULT '',

            -- Explicit per-agent opt-in to the CLI's global (ambient) login
            -- when no oauth-class account resolves at spawn (schema v12,
            -- SPEC_ACCOUNT_DELETE_DEAUTH_LAYERS_2_4_2026_07_14.md §2.3).
            use_ambient_login    INTEGER NOT NULL DEFAULT 0,

            -- Redirects this agent's harness at a non-default model vendor
            -- backend (schema v15) — see db_agent_definitions' column doc.
            model_vendor_base_url TEXT NOT NULL DEFAULT '',

            -- Warden Supervisor auto-continue opt-in (schema v17) — see
            -- db_agent_definitions' column doc.
            auto_continue_enabled INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_agents_is_template
            ON db_agents(is_template);
        CREATE INDEX IF NOT EXISTS idx_agents_parent_template_id
            ON db_agents(parent_template_id);
        CREATE INDEX IF NOT EXISTS idx_agents_is_seeded
            ON db_agents(is_seeded);

        CREATE TABLE IF NOT EXISTS db_drone_definitions (
            id          TEXT PRIMARY KEY,
            name        TEXT NOT NULL,
            description TEXT NOT NULL DEFAULT '',
            graph       TEXT NOT NULL DEFAULT '{\"nodes\":[],\"edges\":[]}',
            viewport    TEXT NOT NULL DEFAULT '{\"x\":0,\"y\":0,\"zoom\":1}',
            created_at  INTEGER NOT NULL DEFAULT 0,
            updated_at  INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_drone_definitions_updated
            ON db_drone_definitions(updated_at DESC);

        CREATE TABLE IF NOT EXISTS db_drone_runs (
            id           TEXT PRIMARY KEY,
            drone_id     TEXT NOT NULL,
            status       TEXT NOT NULL DEFAULT 'running',
            started_at   INTEGER NOT NULL DEFAULT 0,
            ended_at     INTEGER NOT NULL DEFAULT 0,
            block_states TEXT NOT NULL DEFAULT '{}',
            output       TEXT NOT NULL DEFAULT '',
            error        TEXT NOT NULL DEFAULT '',
            FOREIGN KEY (drone_id) REFERENCES db_drone_definitions(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_drone_runs_drone_started
            ON db_drone_runs(drone_id, started_at DESC);
        CREATE INDEX IF NOT EXISTS idx_drone_runs_status
            ON db_drone_runs(status);

        -- v10: Standalone skill and MCP server primitives (composable model).
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
        CREATE INDEX IF NOT EXISTS idx_skills_is_global ON db_skills(is_global);

        CREATE TABLE IF NOT EXISTS db_mcp_servers (
            id          TEXT PRIMARY KEY,
            name        TEXT NOT NULL,
            transport   TEXT NOT NULL DEFAULT 'stdio',
            config      TEXT NOT NULL DEFAULT '{}',
            is_global   INTEGER NOT NULL DEFAULT 0,
            created_at  INTEGER NOT NULL DEFAULT 0,
            updated_at  INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_mcp_servers_is_global ON db_mcp_servers(is_global);

        CREATE TABLE IF NOT EXISTS db_agent_skills_ref (
            agent_id TEXT NOT NULL,
            skill_id TEXT NOT NULL,
            PRIMARY KEY (agent_id, skill_id),
            FOREIGN KEY (agent_id) REFERENCES db_agent_definitions(id) ON DELETE CASCADE,
            FOREIGN KEY (skill_id) REFERENCES db_skills(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS db_agent_mcp_ref (
            agent_id TEXT NOT NULL,
            mcp_id   TEXT NOT NULL,
            PRIMARY KEY (agent_id, mcp_id),
            FOREIGN KEY (agent_id) REFERENCES db_agent_definitions(id) ON DELETE CASCADE,
            FOREIGN KEY (mcp_id)   REFERENCES db_mcp_servers(id) ON DELETE CASCADE
        );

        -- v7: MuxBus cloud connectivity — global singleton PKCE token store.
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

        -- v13: per-agent M2M muxbus credential — see db_agent_credentials in
        -- run_shared_store_schema's doc comment for the full rationale.
        CREATE TABLE IF NOT EXISTS db_agent_credentials (
            agent_id       TEXT PRIMARY KEY,
            client_id      TEXT NOT NULL DEFAULT '',
            client_secret  TEXT NOT NULL DEFAULT '',
            token_endpoint TEXT NOT NULL DEFAULT '',
            access_token   TEXT NOT NULL DEFAULT '',
            expires_at     INTEGER NOT NULL DEFAULT 0,
            created_at     INTEGER NOT NULL DEFAULT 0
        );

        -- v14: per-agent native-memory durable mirror — see
        -- db_agent_native_memory in run_shared_store_schema's doc comment for
        -- the full rationale (SPEC_NATIVE_MEMORY_DURABLE_SYNC_2026_08_07.md).
        -- No FK to db_agent_definitions: this table is duplicated into the
        -- shared store below (same pattern as db_agent_credentials, for
        -- id_store's per-channel fallback before 0011_shared_store_backfill),
        -- and SQLite can't enforce a FK across separate database files.
        -- Orphan rows after an agent's deleted are inert: every RPC handler
        -- that would read this table first 404s on agent_def_get, so a
        -- dangling mirror row is never reached, let alone surfaced.
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

        -- v18: per-agent HMAC-SHA256 signing key for host-tier jekt sender
        -- verification (SPEC_JEKT_TRUST_LAYER_COMPLETION_2026_08_13.md §2.2).
        -- hmac_key is base64-encoded, 32 random bytes, minted on first use
        -- (agent_jekt_key_ensure) and never rotated automatically. Local to
        -- this instance's data dir only — not synced anywhere, not the same
        -- secret as any Armory/GitHub credential.
        CREATE TABLE IF NOT EXISTS db_agent_jekt_keys (
            agent_id   TEXT PRIMARY KEY,
            hmac_key   TEXT NOT NULL,
            created_at INTEGER NOT NULL DEFAULT 0
        );

        -- v19: per-agent Ed25519 keypair for LAN-tier jekt sender
        -- verification (SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md §2.1).
        -- public_key is not secret (distributed to LAN peers on demand);
        -- private_key is base64, 32-byte seed, minted on first use
        -- (agent_lan_key_ensure) and never rotated automatically. Same
        -- local-to-this-instance-only guarantee as db_agent_jekt_keys.
        CREATE TABLE IF NOT EXISTS db_agent_lan_keys (
            agent_id    TEXT PRIMARY KEY,
            public_key  TEXT NOT NULL,
            private_key TEXT NOT NULL,
            created_at  INTEGER NOT NULL DEFAULT 0
        );",
    )?;

    // ---- Additive column migrations (schema v2+) ----
    // The flat CREATE batch above covers fresh databases. These ALTERs
    // carry an existing database (e.g. a developer's dev DB that persists
    // across builds) forward. Idempotent: the "duplicate column" error is
    // swallowed. New additive columns append here + bump OBJECT_SCHEMA_VERSION.
    //
    // v2: db_agent_definitions.updated_at — last-modified timestamp
    //     (created_at already existed; updates now stamp updated_at).
    // v3: db_agent_definitions.user_hidden — per-user hide flag for
    //     templates (Phase 2 of the two-tier picker spec, Q2 Decision Y).
    //     Defaults to 0 (visible) for all existing rows so a migration
    //     never silently hides previously-visible templates.
    // v5: db_agents.last_block_id — most-recent launch's block (Phase 3c).
    //     Defaults to '' for existing rows; the dual-write populates it on
    //     the next launch/continuation. Read side (3b.1b) treats '' as
    //     "no snapshot" (same as the current empty-block_id fallback).
    // v6: Container support (Phase 0 of SPEC_CONTAINER_PANE_SUPPORT_2026_06_11.md).
    //     container_image / container_volumes / container_name on both tables.
    //     Host-agent rows default to '', '[]', '' respectively — ContainerManager
    //     populates them on first container spawn.
    // v7: create db_muxbus_credentials if it doesn't exist yet (existing DBs
    //     won't have it since it's a new table, not an added column).
    // v10: standalone skill + MCP server tables (new tables, handled via
    //     CREATE TABLE IF NOT EXISTS in the flat batch above; no ALTER needed).
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS db_muxbus_credentials (
            id             TEXT PRIMARY KEY DEFAULT 'global',
            cognito_domain TEXT NOT NULL DEFAULT '',
            client_id      TEXT NOT NULL DEFAULT '',
            access_token   TEXT NOT NULL DEFAULT '',
            refresh_token  TEXT NOT NULL DEFAULT '',
            id_token       TEXT NOT NULL DEFAULT '',
            expires_at     INTEGER NOT NULL DEFAULT 0,
            user_email     TEXT NOT NULL DEFAULT '',
            user_sub       TEXT NOT NULL DEFAULT ''
        )",
    )?;

    for stmt in &[
        "ALTER TABLE db_agent_definitions ADD COLUMN updated_at INTEGER NOT NULL DEFAULT 0",
        "ALTER TABLE db_agent_definitions ADD COLUMN user_hidden INTEGER NOT NULL DEFAULT 0",
        "ALTER TABLE db_agents ADD COLUMN last_block_id TEXT NOT NULL DEFAULT ''",
        "ALTER TABLE db_agent_definitions ADD COLUMN container_image TEXT NOT NULL DEFAULT ''",
        "ALTER TABLE db_agent_definitions ADD COLUMN container_volumes TEXT NOT NULL DEFAULT '[]'",
        "ALTER TABLE db_agent_definitions ADD COLUMN container_name TEXT NOT NULL DEFAULT ''",
        "ALTER TABLE db_agents ADD COLUMN container_image TEXT NOT NULL DEFAULT ''",
        "ALTER TABLE db_agents ADD COLUMN container_volumes TEXT NOT NULL DEFAULT '[]'",
        "ALTER TABLE db_agents ADD COLUMN container_name TEXT NOT NULL DEFAULT ''",
        // NOTE: targets db_bundles, not db_memory_bundles — adopt_legacy_table_names
        // (called at the top of this function) has already renamed the table
        // by the time this runs, on every DB that goes through the Phase 4
        // rename. See SPEC_ARMORY_PHASE4_STORAGE_RENAME_COMPLETION_2026_07_12.md.
        "ALTER TABLE db_bundles ADD COLUMN is_global INTEGER NOT NULL DEFAULT 0",
        "ALTER TABLE db_bundles ADD COLUMN sort_order INTEGER NOT NULL DEFAULT 0",
        // v12: explicit per-agent ambient-login opt-in (fail-by-default spawn
        // gating — SPEC_ACCOUNT_DELETE_DEAUTH_LAYERS_2_4_2026_07_14.md §2.3).
        "ALTER TABLE db_agent_definitions ADD COLUMN use_ambient_login INTEGER NOT NULL DEFAULT 0",
        "ALTER TABLE db_agents ADD COLUMN use_ambient_login INTEGER NOT NULL DEFAULT 0",
        // v15: model vendor override — redirects a harness at a non-default
        // backend (e.g. ANTHROPIC_BASE_URL). Formalizes harness vs. model
        // vendor as distinct concepts; see ProviderConfig::base_url_env_var.
        "ALTER TABLE db_agent_definitions ADD COLUMN model_vendor_base_url TEXT NOT NULL DEFAULT ''",
        "ALTER TABLE db_agents ADD COLUMN model_vendor_base_url TEXT NOT NULL DEFAULT ''",
        // v16: provider-scoped bundle instructions (ABF v0.2 §2.2) — JSON
        // object of {provider_id: content} variants, additive to the
        // existing flat `instructions` column.
        "ALTER TABLE db_bundles ADD COLUMN instructions_by_provider TEXT NOT NULL DEFAULT '{}'",
        // v17: Warden Supervisor auto-continue opt-in (fail-by-default,
        // same posture as use_ambient_login) — see
        // ANALYSIS_WARDEN_AUTO_CONTROLLER_CONTINUATION_WATCHER_2026_08_12.md.
        "ALTER TABLE db_agent_definitions ADD COLUMN auto_continue_enabled INTEGER NOT NULL DEFAULT 0",
        "ALTER TABLE db_agents ADD COLUMN auto_continue_enabled INTEGER NOT NULL DEFAULT 0",
    ] {
        if let Err(e) = conn.execute_batch(stmt) {
            let msg = e.to_string();
            if !msg.contains("duplicate column") {
                return Err(e.into());
            }
        }
    }

    // ---- Seed blank Memory singleton ----
    // The launch UI renders this as the default option in its Memory
    // dropdown. Fixed id so tests + dev seed data can hard-code
    // references.
    conn.execute_batch(
        "INSERT OR IGNORE INTO db_bundles
            (id, name, description, is_blank, created_at, updated_at)
         VALUES ('blank', '__blank__', 'Vanilla CLI — no instructions, no context', 1, 0, 0);",
    )?;

    // Channel-scoped migration tracking (parallel to the global db_migrations
    // in store.db). MigrationScope::Channel migrations record completion here
    // so each channel tracks its own migration state independently.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS db_migrations (
            id          TEXT PRIMARY KEY,
            applied_at  TEXT NOT NULL,
            duration_ms INTEGER NOT NULL DEFAULT 0,
            scope       TEXT NOT NULL DEFAULT 'channel'
        );",
    )?;

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
            updated_at    INTEGER NOT NULL DEFAULT 0
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
            max_age_secs  INTEGER
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
        );",
    )?;

    // Seed the blank Memory singleton — same fixed id as objects.db so
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
    for stmt in &[
        "ALTER TABLE db_cron_jobs ADD COLUMN max_age_secs INTEGER",
        // v7: provider-scoped bundle instructions (ABF v0.2 §2.2).
        "ALTER TABLE db_bundles ADD COLUMN instructions_by_provider TEXT NOT NULL DEFAULT '{}'",
    ] {
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
mod tests {
    use super::*;

    /// Every table the flat `objects.db` schema must contain.
    const EXPECTED_TABLES: &[&str] = &[
        "db_client",
        "db_window",
        "db_workspace",
        "db_tab",
        "db_layout",
        "db_block",
        "db_temp",
        "db_agent_definitions",
        "db_agent_content",
        "db_agent_skills",
        "db_agent_history",
        "db_accounts",
        "db_agent_identity_links",
        "db_bundles",
        "db_agent_instances",
        "db_agents",
        "db_drone_definitions",
        "db_drone_runs",
    ];

    fn table_exists(conn: &Connection, name: &str) -> bool {
        let count: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1",
                [name],
                |row| row.get(0),
            )
            .unwrap();
        count == 1
    }

    fn index_exists(conn: &Connection, name: &str) -> bool {
        let count: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='index' AND name=?1",
                [name],
                |row| row.get(0),
            )
            .unwrap();
        count == 1
    }

    fn column_exists(conn: &Connection, table: &str, column: &str) -> bool {
        let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})")).unwrap();
        let names: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        names.iter().any(|n| n == column)
    }

    #[test]
    fn test_object_schema_creates_all_tables_and_singletons() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        run_object_schema(&conn).unwrap();

        for table in EXPECTED_TABLES {
            assert!(table_exists(&conn, table), "{table} should exist");
        }
        // De-forged + bundle indexes.
        for idx in &[
            "idx_agent_definitions_slug",
            "idx_agent_history_agent_date",
            "idx_agent_identity_links_account",
            "idx_accounts_provider",
            "idx_bundles_is_blank",
            "idx_agent_instances_definition",
            "idx_agent_instances_name_recent",
            "idx_agents_is_template",
            "idx_agents_parent_template_id",
            "idx_agents_is_seeded",
            "idx_drone_definitions_updated",
            "idx_drone_runs_status",
        ] {
            assert!(index_exists(&conn, idx), "{idx} should exist");
        }

        // Blank singleton seeded.
        let mem_blank: i64 = conn
            .query_row(
                "SELECT count(*) FROM db_bundles WHERE id='blank' AND is_blank=1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(mem_blank, 1, "blank Memory singleton should be seeded");
    }

    #[test]
    fn test_object_schema_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        run_object_schema(&conn).unwrap();
        run_object_schema(&conn).unwrap(); // second pass must not error

        // Singleton stays unique.
        let mem_count: i64 = conn
            .query_row(
                "SELECT count(*) FROM db_bundles WHERE id='blank'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(mem_count, 1);
    }

    #[test]
    fn test_object_schema_has_model_vendor_base_url_on_both_tables() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        run_object_schema(&conn).unwrap();
        // Second pass: the ALTER TABLE array's duplicate-column guard must
        // not error when the column already exists (fresh installs get it
        // via CREATE TABLE; the ALTER is for upgrading pre-v15 databases).
        run_object_schema(&conn).unwrap();

        assert!(column_exists(&conn, "db_agent_definitions", "model_vendor_base_url"));
        assert!(column_exists(&conn, "db_agents", "model_vendor_base_url"));
    }

    #[test]
    fn test_object_schema_has_auto_continue_enabled_on_both_tables() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        run_object_schema(&conn).unwrap();
        // Second pass: the ALTER TABLE array's duplicate-column guard must
        // not error when the column already exists (fresh installs get it
        // via CREATE TABLE; the ALTER is for upgrading pre-v17 databases).
        run_object_schema(&conn).unwrap();

        assert!(column_exists(&conn, "db_agent_definitions", "auto_continue_enabled"));
        assert!(column_exists(&conn, "db_agents", "auto_continue_enabled"));
    }

    #[test]
    fn test_db_bundles_has_instructions_by_provider_in_both_schemas() {
        // ABF v0.2 §2.2 (v16): db_bundles lives in both objects.db (via
        // run_object_schema) and the shared store.db (via
        // run_shared_store_schema) — mirrors the db_agent_credentials /
        // db_agent_native_memory both-schemas precedent (v13/v14).
        let object_conn = Connection::open_in_memory().unwrap();
        object_conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        run_object_schema(&object_conn).unwrap();
        run_object_schema(&object_conn).unwrap(); // idempotent second pass
        assert!(column_exists(&object_conn, "db_bundles", "instructions_by_provider"));

        let shared_conn = Connection::open_in_memory().unwrap();
        shared_conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        run_shared_store_schema(&shared_conn).unwrap();
        run_shared_store_schema(&shared_conn).unwrap();
        assert!(column_exists(&shared_conn, "db_bundles", "instructions_by_provider"));
    }

    #[test]
    fn test_object_schema_omits_dead_tables() {
        let conn = Connection::open_in_memory().unwrap();
        run_object_schema(&conn).unwrap();
        for dead in DEAD_TABLE_DROPS {
            assert!(!table_exists(&conn, dead), "{dead} must not be created");
        }
        // Legacy forge names are never created either.
        for (legacy, _) in LEGACY_TABLE_RENAMES {
            assert!(
                !table_exists(&conn, legacy),
                "legacy {legacy} must not be created by the flat schema"
            );
        }
    }

    #[test]
    fn test_adopt_legacy_renames_forge_tables() {
        // Simulate a pre-flatten (post-v11) dev DB: legacy forge table
        // names + a dead workflow table, with seeded rows.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        conn.execute_batch(
            "CREATE TABLE db_forge_agents (
                id TEXT PRIMARY KEY, slug TEXT NOT NULL DEFAULT '', name TEXT NOT NULL,
                icon TEXT NOT NULL DEFAULT '✦', provider TEXT NOT NULL,
                description TEXT NOT NULL DEFAULT '', working_directory TEXT NOT NULL DEFAULT '',
                shell TEXT NOT NULL DEFAULT '', provider_flags TEXT NOT NULL DEFAULT '',
                auto_start INTEGER NOT NULL DEFAULT 0, restart_on_crash INTEGER NOT NULL DEFAULT 0,
                idle_timeout_minutes INTEGER NOT NULL DEFAULT 0,
                agent_type TEXT NOT NULL DEFAULT 'standalone', environment TEXT NOT NULL DEFAULT '',
                agent_bus_id TEXT NOT NULL DEFAULT '', is_seeded INTEGER NOT NULL DEFAULT 0,
                accounts TEXT NOT NULL DEFAULT '',
                parent_id TEXT NOT NULL DEFAULT '', branch_label TEXT NOT NULL DEFAULT '',
                created_at INTEGER NOT NULL DEFAULT 0
            );
            CREATE UNIQUE INDEX idx_forge_agents_slug ON db_forge_agents(slug);
            INSERT INTO db_forge_agents (id, slug, name, provider)
                VALUES ('a1', 'coder', 'Coder', 'claude');

            CREATE TABLE db_workflow_definitions (id TEXT PRIMARY KEY);",
        )
        .unwrap();

        run_object_schema(&conn).unwrap();

        // Renamed, data preserved.
        assert!(table_exists(&conn, "db_agent_definitions"));
        assert!(!table_exists(&conn, "db_forge_agents"));
        let name: String = conn
            .query_row(
                "SELECT name FROM db_agent_definitions WHERE id='a1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(name, "Coder");
        // Old index dropped, new index present.
        assert!(!index_exists(&conn, "idx_forge_agents_slug"));
        assert!(index_exists(&conn, "idx_agent_definitions_slug"));
        // Dead table dropped.
        assert!(!table_exists(&conn, "db_workflow_definitions"));
    }

    #[test]
    fn test_adopt_legacy_is_noop_on_fresh_db() {
        let conn = Connection::open_in_memory().unwrap();
        run_object_schema(&conn).unwrap();
        // Re-running schema (which re-runs adopt) on the already-flat DB
        // leaves the de-forged tables intact and creates no legacy names.
        run_object_schema(&conn).unwrap();
        assert!(table_exists(&conn, "db_agent_definitions"));
        assert!(!table_exists(&conn, "db_forge_agents"));
    }

    #[test]
    fn test_adopt_legacy_both_tables_present_is_non_destructive() {
        // Downgrade-roundtrip: a flat DB (db_agent_definitions) where a
        // pre-flatten build later re-created db_forge_agents and wrote a
        // row. The adopt step must NOT drop the legacy table — silent
        // data loss is the bug class behind PR #933's Codex P1.
        let conn = Connection::open_in_memory().unwrap();
        run_object_schema(&conn).unwrap(); // creates db_agent_definitions
        conn.execute_batch(
            "CREATE TABLE db_forge_agents (id TEXT PRIMARY KEY, name TEXT NOT NULL);
             INSERT INTO db_forge_agents (id, name) VALUES ('downgrade-era', 'Recover Me');",
        )
        .unwrap();

        run_object_schema(&conn).unwrap();

        // Legacy table left intact — data recoverable, not dropped.
        assert!(table_exists(&conn, "db_forge_agents"));
        let name: String = conn
            .query_row(
                "SELECT name FROM db_forge_agents WHERE id='downgrade-era'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(name, "Recover Me");
        // Flat table still present and authoritative.
        assert!(table_exists(&conn, "db_agent_definitions"));
    }

    #[test]
    fn test_adopt_legacy_fk_cascade_survives_rename() {
        // A renamed parent must keep cascading into renamed children.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        conn.execute_batch(
            "CREATE TABLE db_forge_agents (
                id TEXT PRIMARY KEY, slug TEXT NOT NULL DEFAULT '', name TEXT NOT NULL,
                icon TEXT NOT NULL DEFAULT '✦', provider TEXT NOT NULL,
                description TEXT NOT NULL DEFAULT '', working_directory TEXT NOT NULL DEFAULT '',
                shell TEXT NOT NULL DEFAULT '', provider_flags TEXT NOT NULL DEFAULT '',
                auto_start INTEGER NOT NULL DEFAULT 0, restart_on_crash INTEGER NOT NULL DEFAULT 0,
                idle_timeout_minutes INTEGER NOT NULL DEFAULT 0,
                agent_type TEXT NOT NULL DEFAULT 'standalone', environment TEXT NOT NULL DEFAULT '',
                agent_bus_id TEXT NOT NULL DEFAULT '', is_seeded INTEGER NOT NULL DEFAULT 0,
                accounts TEXT NOT NULL DEFAULT '',
                parent_id TEXT NOT NULL DEFAULT '', branch_label TEXT NOT NULL DEFAULT '',
                created_at INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE db_forge_content (
                agent_id TEXT NOT NULL, content_type TEXT NOT NULL,
                content TEXT NOT NULL DEFAULT '', updated_at INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (agent_id, content_type),
                FOREIGN KEY (agent_id) REFERENCES db_forge_agents(id) ON DELETE CASCADE
            );
            INSERT INTO db_forge_agents (id, name, provider) VALUES ('a1', 'Coder', 'claude');
            INSERT INTO db_forge_content (agent_id, content_type, content)
                VALUES ('a1', 'soul', 'hello');",
        )
        .unwrap();

        run_object_schema(&conn).unwrap();

        conn.execute("DELETE FROM db_agent_definitions WHERE id='a1'", [])
            .unwrap();
        let remaining: i64 = conn
            .query_row(
                "SELECT count(*) FROM db_agent_content WHERE agent_id='a1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            remaining, 0,
            "FK cascade must survive the forge→agent table rename"
        );
    }

    #[test]
    fn test_user_hidden_column_present_on_fresh_db() {
        // Schema v3 (Phase 2 hide-templates) adds db_agent_definitions
        // .user_hidden. A fresh database lands the column via the flat
        // CREATE statement; an existing-but-stale database lands it via
        // the additive ALTER below.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        run_object_schema(&conn).unwrap();

        // Column exists with the documented default — INSERT without
        // user_hidden must succeed and read back as 0.
        conn.execute_batch(
            "INSERT INTO db_agent_definitions (id, name, provider)
             VALUES ('a-fresh', 'Fresh', 'claude');",
        )
        .unwrap();
        let hidden: i64 = conn
            .query_row(
                "SELECT user_hidden FROM db_agent_definitions WHERE id='a-fresh'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(hidden, 0);
    }

    #[test]
    fn test_user_hidden_column_added_to_existing_db_via_alter() {
        // Simulate an existing dev database created before Phase 2:
        // db_agent_definitions exists but lacks the user_hidden column.
        // run_object_schema must ALTER it in, preserving every existing
        // row at the default 0. Idempotent on subsequent runs.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        conn.execute_batch(
            "CREATE TABLE db_agent_definitions (
                id TEXT PRIMARY KEY, slug TEXT NOT NULL DEFAULT '',
                name TEXT NOT NULL, icon TEXT NOT NULL DEFAULT '✦',
                provider TEXT NOT NULL,
                description TEXT NOT NULL DEFAULT '',
                working_directory TEXT NOT NULL DEFAULT '',
                shell TEXT NOT NULL DEFAULT '',
                provider_flags TEXT NOT NULL DEFAULT '',
                auto_start INTEGER NOT NULL DEFAULT 0,
                restart_on_crash INTEGER NOT NULL DEFAULT 0,
                idle_timeout_minutes INTEGER NOT NULL DEFAULT 0,
                agent_type TEXT NOT NULL DEFAULT 'standalone',
                environment TEXT NOT NULL DEFAULT '',
                agent_bus_id TEXT NOT NULL DEFAULT '',
                is_seeded INTEGER NOT NULL DEFAULT 0,
                accounts TEXT NOT NULL DEFAULT '',
                parent_id TEXT NOT NULL DEFAULT '',
                branch_label TEXT NOT NULL DEFAULT '',
                created_at INTEGER NOT NULL DEFAULT 0
            );
            INSERT INTO db_agent_definitions (id, name, provider, is_seeded)
                VALUES ('pre-existing', 'Old Template', 'claude', 1);",
        )
        .unwrap();

        run_object_schema(&conn).unwrap();
        // Idempotent — second pass must not error and must not
        // re-default existing rows.
        run_object_schema(&conn).unwrap();

        let hidden: i64 = conn
            .query_row(
                "SELECT user_hidden FROM db_agent_definitions WHERE id='pre-existing'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            hidden, 0,
            "ALTER must default existing rows to 0 (visible), never to 1",
        );
    }

    #[test]
    fn stamp_version_writes_pragma() {
        let conn = Connection::open_in_memory().unwrap();
        stamp_version(&conn, OBJECT_SCHEMA_VERSION).unwrap();
        let v: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, OBJECT_SCHEMA_VERSION);
    }

    #[test]
    fn check_schema_compat_refuses_newer_db_without_writing() {
        // `SPEC_DATA_CHANNELS_2026_05_24.md` §3.3 safety lock — if the
        // DB on disk was stamped by a newer AgentMux binary, this one
        // MUST refuse to open it. The split into
        // check_schema_compat + stamp_version (codex P1 on #1029)
        // ensures the check runs BEFORE any migration side effects:
        // legacy-table rename + seed-insert in `run_object_schema` are
        // mutating, and if we ran them first we'd partially alter a
        // newer DB before the error fired.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA user_version = 99;").unwrap();
        let err = check_schema_compat(&conn, OBJECT_SCHEMA_VERSION, "objects.db")
            .expect_err("expected refusal");
        match err {
            StoreError::SchemaTooNew { db, found, expected } => {
                assert_eq!(db, "objects.db");
                assert_eq!(found, 99);
                assert_eq!(expected, OBJECT_SCHEMA_VERSION);
            }
            other => panic!("expected SchemaTooNew, got {other:?}"),
        }
        // Crucially, `check_schema_compat` made NO writes. The
        // `user_version` is still 99. (Before the split — when the
        // single function combined the check with the write — this
        // invariant held only by virtue of the early `return Err`
        // skipping the stamp; with the split the function is
        // structurally read-only, eliminating the risk that a
        // future refactor reintroduces a downgrade-corrupt path.)
        let v: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, 99, "rejected DB must not have its user_version stamp overwritten");
    }

    #[test]
    fn check_schema_compat_accepts_equal_or_lower_without_writing() {
        // check_schema_compat must NEVER write — it just gates
        // migrations. The on-disk `user_version` is untouched by it,
        // regardless of whether the verdict is accept or reject.
        // stamp_version is the only thing that writes, and only after
        // migrations succeed.

        // Equal: check passes silently.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(&format!("PRAGMA user_version = {};", OBJECT_SCHEMA_VERSION))
            .unwrap();
        check_schema_compat(&conn, OBJECT_SCHEMA_VERSION, "objects.db").unwrap();
        let v: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, OBJECT_SCHEMA_VERSION);

        // Lower (forward-migration path): check passes, version stays
        // at the OLD value — stamp_version is what bumps it after
        // migrations.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA user_version = 1;").unwrap();
        check_schema_compat(&conn, OBJECT_SCHEMA_VERSION, "objects.db").unwrap();
        let v: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, 1, "check_schema_compat must not write to user_version");

        // Then stamp_version bumps it as the post-migration step.
        stamp_version(&conn, OBJECT_SCHEMA_VERSION).unwrap();
        let v: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, OBJECT_SCHEMA_VERSION);
    }

    #[test]
    fn test_filestore_migrations_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        run_filestore_migrations(&conn).unwrap();
        run_filestore_migrations(&conn).unwrap();
        assert!(table_exists(&conn, "db_wave_file"));
        assert!(table_exists(&conn, "db_file_data"));
    }

    #[test]
    fn test_saga_log_migrations_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        run_saga_log_migrations(&conn).unwrap();
        run_saga_log_migrations(&conn).unwrap();
        assert!(table_exists(&conn, "saga"));
        assert!(table_exists(&conn, "saga_step"));
    }
}
