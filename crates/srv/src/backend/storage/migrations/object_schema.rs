// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The steps of [`run_object_schema`](super::run_object_schema), one function each, in the order they
//! ran: the generic object tables, typed input history, the flat table batch, the additive column
//! migrations, the blank Bundle seed and the channel migration table. Moved out of `run_object_schema`
//! without a change in behaviour. The `doc comment above` notes inside the SQL point at
//! `OBJECT_SCHEMA_VERSION` in `migrations.rs`.

use super::{Connection, StoreError, WSTORE_OTYPES};

/// The canonical `objects.db` table batch: agents, identities, memory, drones and the rest.
pub(super) const OBJECT_TABLES_SQL: &str = "CREATE TABLE IF NOT EXISTS db_agent_content (
            agent_id     TEXT NOT NULL,
            content_type TEXT NOT NULL,
            content      TEXT NOT NULL DEFAULT '',
            updated_at   INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (agent_id, content_type),
            -- v30: targets db_agents(id), not db_agent_definitions(id) — see
            -- OBJECT_SCHEMA_VERSION's v30 doc comment above.
            FOREIGN KEY (agent_id) REFERENCES db_agents(id) ON DELETE CASCADE
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
            -- v30: see OBJECT_SCHEMA_VERSION's v30 doc comment above.
            FOREIGN KEY (agent_id) REFERENCES db_agents(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS db_agent_history (
            id           INTEGER PRIMARY KEY AUTOINCREMENT,
            agent_id     TEXT NOT NULL,
            session_date TEXT NOT NULL,
            entry        TEXT NOT NULL,
            timestamp    INTEGER NOT NULL DEFAULT 0,
            -- v30: see OBJECT_SCHEMA_VERSION's v30 doc comment above.
            FOREIGN KEY (agent_id) REFERENCES db_agents(id) ON DELETE CASCADE
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
            -- v30: see OBJECT_SCHEMA_VERSION's v30 doc comment above.
            FOREIGN KEY (agent_id)   REFERENCES db_agents(id) ON DELETE CASCADE,
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
            updated_at    INTEGER NOT NULL DEFAULT 0,
            is_system     INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_bundles_is_blank
            ON db_bundles(is_blank);

        -- Phase 3a consolidation: `db_agents` collapses the legacy
        -- definitions + instances tables into one table per
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

            -- Latest launch's block: pointer to the most-recent session's
            -- block so the consolidated read can locate the conversation
            -- snapshot without joining db_agent_instances. Was the only
            -- per-launch field db_agents retained until v29 added the
            -- four launch-state columns right below it.
            last_block_id        TEXT NOT NULL DEFAULT '',

            -- Latest launch's state (v29, consolidation Phase 3b/3c). The
            -- original 3a design left status/session_id/started_at/ended_at
            -- to the block; in practice the recent-sessions picker, the
            -- pane close/reopen continuity guarantee
            -- (SPEC_PANE_CLOSE_REOPEN_CONTINUITY_GUARANTEE_2026_07_27.md §4.1)
            -- and the status-filtered instance list all still read them from
            -- db_agent_instances, which is what kept that table alive. One
            -- row per agent means one launch state per agent: a continuation
            -- moves these to the new block/session rather than adding a row.
            session_id           TEXT NOT NULL DEFAULT '',
            status               TEXT NOT NULL DEFAULT '',
            started_at           INTEGER NOT NULL DEFAULT 0,
            ended_at             INTEGER NOT NULL DEFAULT 0,

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

            -- Warden Supervisor auto-continue opt-in (schema v17). RETIRED,
            -- unused; kept because older builds sharing the data dir select it.
            auto_continue_enabled INTEGER NOT NULL DEFAULT 0,

            -- The DEFINITION's own dedicated ABF bundle (schema v19) —
            -- mirrors db_agent_definitions.memory_id. Deliberately a
            -- DIFFERENT column from memory_id above: that one is
            -- instance-scoped (only meaningful when is_template=0,
            -- written exclusively by the instance dual-write helpers) and
            -- this migration doesn't touch its semantics. default_memory_id
            -- is what agent_def_list's db_agents-backed read actually
            -- surfaces as AgentDefinition.memory_id (aliased in the SELECT
            -- below) -- readers that already consult db_agents for listing
            -- need this to see the definition-level binding at all.
            default_memory_id    TEXT NOT NULL DEFAULT ''
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
            error        TEXT NOT NULL DEFAULT ''
            -- No FK to db_drone_definitions: definitions live in the shared store (m0038).
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
        -- v44: who a catalog server reaches beyond its
        -- binds — see OBJECT_SCHEMA_VERSION's v44 doc comment.
        CREATE TABLE IF NOT EXISTS db_mcp_server_reach (
            mcp_id        TEXT PRIMARY KEY,
            all_agents    INTEGER NOT NULL DEFAULT 0,
            in_containers INTEGER NOT NULL DEFAULT 0,
            updated_at    INTEGER NOT NULL DEFAULT 0
        );

        -- v34: skill_id's FK to this store's LOCAL db_skills is gone — see
        -- OBJECT_SCHEMA_VERSION's v34 doc comment above. agent_id's FK to
        -- db_agents (v30) is unchanged.
        CREATE TABLE IF NOT EXISTS db_agent_skills_ref (
            agent_id TEXT NOT NULL,
            skill_id TEXT NOT NULL,
            PRIMARY KEY (agent_id, skill_id),
            FOREIGN KEY (agent_id) REFERENCES db_agents(id) ON DELETE CASCADE
        );

        -- v34: mcp_id's FK to this store's LOCAL db_mcp_servers is gone —
        -- see OBJECT_SCHEMA_VERSION's v34 doc comment above. agent_id's FK
        -- to db_agents (v30) is unchanged.
        CREATE TABLE IF NOT EXISTS db_agent_mcp_ref (
            agent_id TEXT NOT NULL,
            mcp_id   TEXT NOT NULL,
            PRIMARY KEY (agent_id, mcp_id),
            FOREIGN KEY (agent_id) REFERENCES db_agents(id) ON DELETE CASCADE
        );

        -- v23: Bundle-level MCP/skill references (composable model v2,
        -- docs/specs/SPEC_BUNDLE_AS_CONTAINER_V2_2026_08_17.md) — same shape
        -- as the agent-level ref tables above, keyed by bundle_id instead of
        -- agent_id. A bundle's existing `mcp_servers`/`skills` JSON columns
        -- stay untouched (still consumed by .abf export/import); these ref
        -- tables are the new live-resolution path, wired into launch-time
        -- config generation alongside the agent-level refs.
        --
        -- reagentx P0 review on PR #2639: NO foreign key to db_bundles here,
        -- unlike the agent-level ref tables' FK to db_agents (v30: was
        -- db_agent_definitions — see OBJECT_SCHEMA_VERSION's v30 doc comment).
        -- Bundles are authoritatively written through `id_store` (the
        -- shared store in a normal production install — see
        -- `bundle.rs::register_bundle_upsert`), not `mstore`/objects.db,
        -- where this table lives (it must live here to FK to
        -- db_mcp_servers/db_skills, which used to be mstore-local too (now
        -- authoritatively identity_store as of Phase 2 — see
        -- OBJECT_SCHEMA_VERSION's v34 doc comment). mstore's own copy of
        -- db_bundles is a schema-compatible but essentially always-
        -- empty local mirror in that case — an FK against it would make
        -- every real bind fail. Same no-FK-to-a-table-living-in-the-wrong-
        -- store reasoning as `db_agent_identity_links.account_id`'s
        -- deliberate lack of an account FK (see that table's own doc
        -- comment). Existence is checked at the application layer instead
        -- — see `Store::bundle_skill_bind`'s `id_store` parameter.
        --
        -- v34: skill_id's FK to db_skills is ALSO gone now (it was
        -- mstore-local before v34; see OBJECT_SCHEMA_VERSION's v34 doc
        -- comment) — this table has never had a bundle_id FK, so it is now
        -- FK-free entirely, existence checked purely at the application
        -- layer for both columns.
        CREATE TABLE IF NOT EXISTS db_bundle_skills_ref (
            bundle_id TEXT NOT NULL,
            skill_id  TEXT NOT NULL,
            PRIMARY KEY (bundle_id, skill_id)
        );

        -- v34: mcp_id's FK to db_mcp_servers is gone — see the
        -- db_bundle_skills_ref comment above; same reasoning, MCP side.
        -- No longer written or read: a bundle carries no MCP servers, and
        -- m0036_drop_bundle_mcp emptied it. Still created, because an older
        -- build sharing the channel expects it.
        CREATE TABLE IF NOT EXISTS db_bundle_mcp_ref (
            bundle_id TEXT NOT NULL,
            mcp_id    TEXT NOT NULL,
            PRIMARY KEY (bundle_id, mcp_id)
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

        -- v33: what each agent was last observed to READ as project
        -- instructions. Hash and timestamp only, never content — these files
        -- belong to the repository and are always readable from it, unlike
        -- db_agent_native_memory above, which mirrors content precisely
        -- because the originals can become unreachable. See
        -- SPEC_INSTRUCTION_AND_MEMORY_PORTABILITY_2026_09_09.md Phase 3.
        CREATE TABLE IF NOT EXISTS db_agent_project_instructions (
            agent_id      TEXT NOT NULL,
            path          TEXT NOT NULL,
            content_hash  TEXT NOT NULL DEFAULT '',
            size_bytes    INTEGER NOT NULL DEFAULT 0,
            owner         TEXT NOT NULL DEFAULT 'foreign',
            existed       INTEGER NOT NULL DEFAULT 0,
            observed_at   INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (agent_id, path)
        );

        -- v24: append-only version history for native memory content — see
        -- db_agent_native_memory_versions in run_shared_store_schema's doc
        -- comment for the full rationale
        -- (SPEC_MEMORY_VERSION_CONTROL_AND_ARMORY_AUDIT_2026_08_19.md).
        -- Duplicated here for the same id_store-per-channel-fallback reason
        -- as db_agent_native_memory itself, immediately above.
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
        CREATE INDEX IF NOT EXISTS idx_native_memory_versions_lookup
            ON db_agent_native_memory_versions(agent_id, filename, created_at);

        -- v35: append-only version history for Global Memory entry content
        -- — see OBJECT_SCHEMA_VERSION's own doc comment above for the full
        -- rationale (SPEC_AGENT_FACING_GLOBAL_MEMORY_API_2026_09_15.md).
        -- Single `bundle_id` key, not a composite one — a Bundle's identity
        -- is already a single id, unlike native memory's (agent_id, filename).
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
            -- v39 (identity M4c-1): the writer's UID from the request's
            -- Caller, beside the written_by name. Empty when unattributed.
            written_by_uid     TEXT NOT NULL DEFAULT ''
        );
        CREATE INDEX IF NOT EXISTS idx_bundle_versions_lookup
            ON db_bundle_versions(bundle_id, created_at);

        -- v18: per-agent HMAC-SHA256 signing key for host-tier jekt sender
        -- verification (SPEC_JEKT_TRUST_LAYER_COMPLETION_2026_08_13.md §2.2).
        -- hmac_key is base64-encoded, 32 random bytes, minted on first use
        -- (agent_jekt_key_ensure). As of
        -- SPEC_JEKT_HOST_KEY_TTL_ROTATION_2026_09_14.md, a key past
        -- JEKT_KEY_TTL_SECS (24h) old is rotated the next time
        -- agent_jekt_key_ensure runs for it (that agent's next spawn) — not
        -- invalidated in place, so no expires_at column is needed; TTL is
        -- computed from created_at at read time. Local to this instance's
        -- data dir only — not synced anywhere, not the same secret as any
        -- Armory/GitHub credential.
        CREATE TABLE IF NOT EXISTS db_agent_jekt_keys (
            agent_id   TEXT PRIMARY KEY,
            hmac_key   TEXT NOT NULL,
            created_at INTEGER NOT NULL DEFAULT 0
        );

        -- v20: per-agent Ed25519 keypair for LAN-tier jekt sender
        -- verification (SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md §2.1).
        -- public_key is not secret (distributed to LAN peers on demand);
        -- private_key is base64, 32-byte seed, minted on first use
        -- (agent_lan_key_ensure) and never rotated automatically — and,
        -- unlike db_agent_jekt_keys, deliberately NOT given a TTL by
        -- SPEC_JEKT_HOST_KEY_TTL_ROTATION_2026_09_14.md §4: rotating a LAN
        -- key would desync it from any peer's permanent
        -- db_lan_peer_pubkey_pins trust-on-first-use pin of the OLD key,
        -- turning every future signature into an unconditional forced-
        -- sensitive 'mismatch' for that peer with no re-pin mechanism to
        -- recover. Same local-to-this-instance-only guarantee as
        -- db_agent_jekt_keys otherwise.
        CREATE TABLE IF NOT EXISTS db_agent_lan_keys (
            agent_id    TEXT PRIMARY KEY,
            public_key  TEXT NOT NULL,
            private_key TEXT NOT NULL,
            created_at  INTEGER NOT NULL DEFAULT 0
        );

        -- v36: per-agent Ed25519 keypair for general agent-to-agent WAN-tier
        -- jekt signing.
        -- Same local-to-this-instance guarantee as db_agent_jekt_keys and
        -- db_agent_lan_keys. Imported once into wan.db (W3-S); srv signs with
        -- that copy and no agent holds one (SPEC_WAN_JEKT_SIGNED_BY_SRV_2026_10_14
        -- §3.3), never over any RPC. The public half is not secret — it is
        -- what gets published to muxbus's agent-ownership registry so a
        -- WAN counterpart can verify this agent's signatures.
        --
        -- Deliberately NOT a reuse of db_agent_lan_keys: see the v36 doc
        -- comment above for why the two tiers' keys must have independent
        -- lifecycles. `key_version` starts at 1 and is reserved for the
        -- version-synchronised publish (§3.2) and rotation (§3.6) phases;
        -- nothing reads it yet.
        CREATE TABLE IF NOT EXISTS db_agent_wan_keys (
            agent_id    TEXT PRIMARY KEY,
            public_key  TEXT NOT NULL,
            private_key TEXT NOT NULL,
            key_version INTEGER NOT NULL DEFAULT 1,
            created_at  INTEGER NOT NULL DEFAULT 0
        );

        -- v37: per-agent local identity token, keyed by the agent's UID
        -- (db_agents.id) rather than by display name -- identity M1,
        -- SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED_2026_09_23.md §6/§9.1.
        -- Minted on first spawn, injected as AGENTMUX_AGENT_TOKEN into that
        -- one agent's process, revoked by purge_agent_dependents on delete.
        -- Not read by anything until M4; see storage/agent_tokens.rs.
        -- No FOREIGN KEY on purpose: a token must be mintable in the same
        -- critical section as the row it belongs to without ordering the
        -- two inserts, and purge_agent_dependents already owns cleanup.
        CREATE TABLE IF NOT EXISTS db_agent_tokens (
            agent_id   TEXT PRIMARY KEY,
            token      TEXT NOT NULL,
            created_at INTEGER NOT NULL DEFAULT 0
        );

        -- v38: names of deleted agents, lowercased -- identity M4a,
        -- SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED_2026_09_23.md §6.5.4. A
        -- name-keyed signing key under a tombstoned name is never carried to
        -- a new agent's UID (M4d); the reuser gets a fresh keypair. Keyed by
        -- name because that is what the legacy key tables are keyed by.
        CREATE TABLE IF NOT EXISTS db_agent_key_tombstones (
            name        TEXT PRIMARY KEY,
            deleted_uid TEXT NOT NULL,
            deleted_at  INTEGER NOT NULL DEFAULT 0
        );

        -- Identity M4d-2: each agent's LAN and WAN signing keypairs filed
        -- under its UID (db_agents.id), copied from the name-keyed tables on
        -- ownership evidence or minted fresh (storage/agent_uid_keys.rs,
        -- SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED_2026_09_23.md §6.5.10).
        -- copied_from is the name a key was copied from, '' when minted.
        -- Purged by UID in purge_agent_dependents. Additive, applied on
        -- every open, so no version bump.
        CREATE TABLE IF NOT EXISTS db_agent_lan_keys_by_uid (
            uid         TEXT PRIMARY KEY,
            public_key  TEXT NOT NULL,
            private_key TEXT NOT NULL,
            created_at  INTEGER NOT NULL DEFAULT 0,
            copied_from TEXT NOT NULL DEFAULT ''
        );
        CREATE TABLE IF NOT EXISTS db_agent_wan_keys_by_uid (
            uid         TEXT PRIMARY KEY,
            public_key  TEXT NOT NULL,
            private_key TEXT NOT NULL,
            created_at  INTEGER NOT NULL DEFAULT 0,
            copied_from TEXT NOT NULL DEFAULT ''
        );
        -- Identity M4d-6: a remote agent's UID key, pinned per (name, uid) on
        -- first sight beside a matching name pin (storage/lan_peer_uid_pins.rs).
        -- Additive, applied on every open, so no version bump.
        CREATE TABLE IF NOT EXISTS db_lan_peer_uid_pins (
            agent_id      TEXT NOT NULL,
            uid           TEXT NOT NULL,
            public_key    TEXT NOT NULL,
            first_seen_at INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (agent_id, uid)
        );

        -- Identity M4d-1: every display / instance name an agent has had.
        -- Keys are filed under names (and their fallback ids), and every
        -- write path that renames an agent — rename, the continuation fold,
        -- anything later — leaves keys under the old name, so the purge must
        -- still find them. Recorded by trigger so no write path can miss it;
        -- never capped; deleted with the row. Additive, applied on every
        -- open, so no version bump.
        CREATE TABLE IF NOT EXISTS db_agent_former_names (
            agent_id    TEXT NOT NULL,
            name        TEXT NOT NULL,
            recorded_at INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (agent_id, name),
            FOREIGN KEY (agent_id) REFERENCES db_agents(id) ON DELETE CASCADE
        );
        CREATE TRIGGER IF NOT EXISTS trg_agents_former_name
            AFTER UPDATE OF name ON db_agents
            WHEN old.is_template = 0 AND old.name <> new.name AND trim(old.name) <> ''
        BEGIN
            INSERT OR IGNORE INTO db_agent_former_names (agent_id, name, recorded_at)
            VALUES (old.id, old.name, CAST(strftime('%s', 'now') AS INTEGER) * 1000);
        END;
        CREATE TRIGGER IF NOT EXISTS trg_agents_former_instance_name
            AFTER UPDATE OF instance_name ON db_agents
            WHEN old.is_template = 0
             AND COALESCE(old.instance_name, '') <> COALESCE(new.instance_name, '')
             AND trim(COALESCE(old.instance_name, '')) <> ''
        BEGIN
            INSERT OR IGNORE INTO db_agent_former_names (agent_id, name, recorded_at)
            VALUES (old.id, old.instance_name, CAST(strftime('%s', 'now') AS INTEGER) * 1000);
        END;

        -- v40: jekts held for an absent agent of this channel
        -- (SPEC_DURABLE_JEKT_DELIVERY_2026_09_24.md). Verdict columns are
        -- NULL (unchecked), 0 (failed) or 1 (verified).
        CREATE TABLE IF NOT EXISTS db_jekt_held (
            request_id       TEXT PRIMARY KEY,
            target_uid       TEXT NOT NULL,
            target_agent     TEXT NOT NULL DEFAULT '',
            source_agent     TEXT NOT NULL DEFAULT '',
            audit_source_uid TEXT NOT NULL DEFAULT '',
            message          TEXT NOT NULL,
            priority         TEXT NOT NULL DEFAULT '',
            jekt_tier        TEXT NOT NULL DEFAULT '',
            delivery_tier    TEXT NOT NULL DEFAULT '',
            sig_verified     INTEGER,
            reagent_verified INTEGER,
            lan_verified     INTEGER,
            channel_verified INTEGER,
            is_transcript_request              INTEGER NOT NULL DEFAULT 0,
            transcript_request_escalate_forced INTEGER NOT NULL DEFAULT 0,
            sent_at_ms       INTEGER NOT NULL,
            expires_at_ms    INTEGER NOT NULL,
            attempts         INTEGER NOT NULL DEFAULT 0,
            last_error       TEXT NOT NULL DEFAULT ''
        );
        CREATE INDEX IF NOT EXISTS idx_jekt_held_target ON db_jekt_held (target_uid, sent_at_ms);

        -- v43: devices paired with the viewer listener. See
        -- OBJECT_SCHEMA_VERSION's v43 doc comment above.
        CREATE TABLE IF NOT EXISTS db_viewer_devices (
            device_id    TEXT PRIMARY KEY,
            token_hash   TEXT NOT NULL UNIQUE,
            device_name  TEXT NOT NULL,
            device_key   TEXT NOT NULL DEFAULT '',
            created_ms   INTEGER NOT NULL,
            last_seen_ms INTEGER NOT NULL DEFAULT 0
        );

        -- The bundles picked for an agent, in order; its own bundle
        -- (db_agents.default_memory_id) is not listed here
        -- (SPEC_RENAME_KNOWLEDGE_TO_MEMORY_2026_10_06.md §3.6). No FK: a
        -- bundle lives in the identity store, and an agent's rows go with
        -- it in purge_agent_dependents. Additive, applied on every open,
        -- so no version bump.
        CREATE TABLE IF NOT EXISTS db_agent_bundles (
            agent_id  TEXT NOT NULL,
            bundle_id TEXT NOT NULL,
            position  INTEGER NOT NULL,
            PRIMARY KEY (agent_id, bundle_id)
        );

        -- A widget package's `storage` (SPEC_USER_WIDGETS_AND_WIDGET_API_2026_10_09.md §6.3), JSON values; additive, no version bump.
        CREATE TABLE IF NOT EXISTS db_widget_storage (widget_id TEXT NOT NULL, key TEXT NOT NULL, value TEXT NOT NULL, PRIMARY KEY (widget_id, key));

        -- v21: trust-on-first-use pin of a remote agent_id's LAN public key
        -- (SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md §2.2, reagentx P0).
        -- Distinct from db_agent_lan_keys (this instance's OWN agents'
        -- keypairs, private half included) — this table holds only public
        -- keys OBSERVED from LAN peers for agent_ids this instance does not
        -- itself host, pinned on first sight so a later, different key
        -- claiming the same agent_id is treated as a mismatch rather than
        -- silently trusted.
        CREATE TABLE IF NOT EXISTS db_lan_peer_pubkey_pins (
            agent_id     TEXT PRIMARY KEY,
            public_key   TEXT NOT NULL,
            first_seen_at INTEGER NOT NULL DEFAULT 0
        );

        -- v22: durable declared-long-running-task registry — see
        -- OBJECT_SCHEMA_VERSION's v22 doc comment above. `id` mirrors the
        -- frontend's dock node_id (usually the originating tool_use_id) so
        -- rows join cleanly with DockSnapshotCache/ActivityDock data.
        -- `status` is one of 'running' | 'done' | 'error' | 'stopped',
        -- matching the vocabulary tool-adapter.ts already uses for a
        -- background launch's terminal states.
        CREATE TABLE IF NOT EXISTS db_background_tasks (
            id            TEXT PRIMARY KEY,
            block_id      TEXT NOT NULL,
            label         TEXT NOT NULL,
            pid           INTEGER,
            started_at_ms INTEGER NOT NULL,
            status        TEXT NOT NULL DEFAULT 'running',
            last_seen_ms  INTEGER NOT NULL,
            ended_at_ms   INTEGER
        );
        CREATE INDEX IF NOT EXISTS idx_background_tasks_block
            ON db_background_tasks(block_id);
        CREATE INDEX IF NOT EXISTS idx_background_tasks_status
            ON db_background_tasks(status);

        -- v25: saved fleet-control target groups — see OBJECT_SCHEMA_VERSION's
        -- v25 doc comment above. member_ids is a JSON array of block ids
        -- (Swarm's native per-agent selection unit); resolved to concrete
        -- ids by the frontend before either fleet.broadcast or
        -- fleet.bulk-stop executes, so a group's membership can't drift
        -- between when a bulk action is confirmed and when it runs.
        CREATE TABLE IF NOT EXISTS db_agent_groups (
            id         TEXT PRIMARY KEY,
            name       TEXT NOT NULL,
            member_ids TEXT NOT NULL DEFAULT '[]',
            created_at INTEGER NOT NULL DEFAULT 0
        );

        -- v26: allowlist for a responding agent's 'trusted_peers'
        -- conversation_visibility mode — see OBJECT_SCHEMA_VERSION's v26
        -- doc comment above. Mirrors db_lan_peer_pubkey_pins's exact shape
        -- (own module, get/set-style methods, case-insensitive agent_id
        -- lookups). `agent_id` is the RESPONDING agent (whose transcript
        -- may be disclosed); `granted_peer_agent_id` is the requester it
        -- trusts; `tier` records which delivery tier the grant applies to
        -- ('lan'/'wan') since a grant for one tier's cryptographic
        -- identity guarantee should not be silently assumed to hold for a
        -- weaker one.
        CREATE TABLE IF NOT EXISTS db_conversation_trust_grants (
            agent_id             TEXT NOT NULL,
            granted_peer_agent_id TEXT NOT NULL,
            tier                 TEXT NOT NULL,
            granted_at           INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (agent_id, granted_peer_agent_id, tier)
        );

        -- v28: one-shot, on-demand activity summary per definition — see
        -- OBJECT_SCHEMA_VERSION's v28 doc comment above. `summary` is empty
        -- only transiently (a row is only ever inserted once generation
        -- succeeds); there is no 'pending' state stored here — an absent
        -- row IS the pending state.
        CREATE TABLE IF NOT EXISTS db_agent_activity_summaries (
            definition_id TEXT PRIMARY KEY,
            summary       TEXT NOT NULL DEFAULT '',
            updated_at    INTEGER NOT NULL DEFAULT 0
        );";

/// One `db_<otype>` table for each generic StoreObj object type.
pub(super) fn create_generic_tables(conn: &Connection) -> Result<(), StoreError> {
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
    Ok(())
}

/// Typed input history and renamed-from slugs; entries forgotten more than 7 days ago are purged.
pub(super) fn create_input_history_tables(conn: &Connection) -> Result<(), StoreError> {
    for sql in [crate::backend::storage::agent_input_history::SCHEMA, crate::backend::storage::agents::FORMER_SLUGS_SCHEMA] { conn.execute_batch(sql)?; } crate::backend::storage::agent_input_history::purge_trash(conn, agentmux_common::time::now_ms())?; // typed input (forgotten entries past 7 days go at start); renamed-from slugs
    Ok(())
}

/// Additive column migrations, part one: the fork-rename trigger and the muxbus credentials table.
pub(super) fn add_object_columns(conn: &Connection) -> Result<(), StoreError> {
    // ---- Additive column migrations (schema v2+) ----
    // `OBJECT_TABLES_SQL` covers fresh databases. These ALTERs
    // carry an existing database (e.g. a developer's dev DB that persists
    // across builds) forward. Idempotent: the "duplicate column" error is
    // swallowed. New additive columns append here + bump OBJECT_SCHEMA_VERSION.
    //
    // db_agent_definitions' own v2/v3+ ALTERs (updated_at, user_hidden,
    // container_image/volumes/name, use_ambient_login, model_vendor_base_url,
    // auto_continue_enabled, memory_id, conversation_visibility) are run
    // separately by `add_legacy_definition_columns`, conditionally on the table
    // still existing — see its own comment for why they can't simply be deleted even though
    // the table itself is dropped as of v32 (OBJECT_SCHEMA_VERSION's v32 doc
    // comment).
    // v5: db_agents.last_block_id — most-recent launch's block (Phase 3c).
    //     Defaults to '' for existing rows; the dual-write populates it on
    //     the next launch/continuation. Read side (3b.1b) treats '' as
    //     "no snapshot" (same as the current empty-block_id fallback).
    // v6: Container support (Phase 0 of SPEC_CONTAINER_PANE_SUPPORT_2026_06_11.md).
    //     container_image / container_volumes / container_name on both tables.
    //     Host-agent rows default to '', '[]', '' respectively — ContainerManager
    //     populates them on first container spawn.
    // v7: db_muxbus_credentials, a new table (CREATE TABLE IF NOT EXISTS).
    // v10: standalone skill + MCP server tables (new tables, handled via
    //     CREATE TABLE IF NOT EXISTS in `OBJECT_TABLES_SQL`; no ALTER needed).
    conn.execute_batch(crate::backend::storage::agents::FORMER_BRANCH_LABEL_TRIGGER)?; // fork renames join the former names
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
    Ok(())
}

/// `db_agent_definitions`' own additive columns, while that table still exists.
pub(super) fn add_legacy_definition_columns(conn: &Connection) -> Result<(), StoreError> {
    // db_agent_definitions' own additive columns, conditional on the table
    // still existing. Removing these outright (rather than gating them) broke
    // a real upgrade path: several pending migrations (m0007, m0025, m0026,
    // m0028) call Store::open() internally as part of their own up() logic —
    // which runs THIS function, under the CURRENT binary's code — before
    // m0029 (numbered last) ever drops the table in the same pass. A store
    // old enough to still be missing one of these columns (e.g. a genuine
    // pre-v2 install jumping straight to the latest binary) would hit a live
    // "no such column" error from m0007's consolidate pass or m0025/m0028's
    // frozen_repair_def_gaps/project_template_launch_row — both of which
    // SELECT several of these columns from db_agent_definitions — well
    // before m0029 ever runs. Gating on table existence keeps this a no-op
    // once the table is gone (fresh v32+ installs never create it at all;
    // upgraded stores lose it only after m0029 runs later in the same pass)
    // without reintroducing an unconditional dependency on a table this
    // release otherwise no longer declares. Found in review of PR #3096
    // (Codex P1).
    let legacy_defs_exist: bool = conn.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='db_agent_definitions'",
        [],
        |row| row.get::<_, i64>(0),
    )? == 1;
    if legacy_defs_exist {
        apply_additive_columns(conn, &[
            "ALTER TABLE db_agent_definitions ADD COLUMN updated_at INTEGER NOT NULL DEFAULT 0",
            "ALTER TABLE db_agent_definitions ADD COLUMN user_hidden INTEGER NOT NULL DEFAULT 0",
            "ALTER TABLE db_agent_definitions ADD COLUMN container_image TEXT NOT NULL DEFAULT ''",
            "ALTER TABLE db_agent_definitions ADD COLUMN container_volumes TEXT NOT NULL DEFAULT '[]'",
            "ALTER TABLE db_agent_definitions ADD COLUMN container_name TEXT NOT NULL DEFAULT ''",
            "ALTER TABLE db_agent_definitions ADD COLUMN use_ambient_login INTEGER NOT NULL DEFAULT 0",
            "ALTER TABLE db_agent_definitions ADD COLUMN model_vendor_base_url TEXT NOT NULL DEFAULT ''",
            "ALTER TABLE db_agent_definitions ADD COLUMN auto_continue_enabled INTEGER NOT NULL DEFAULT 0",
            "ALTER TABLE db_agent_definitions ADD COLUMN memory_id TEXT NOT NULL DEFAULT ''",
            "ALTER TABLE db_agent_definitions ADD COLUMN conversation_visibility TEXT NOT NULL DEFAULT 'private'",
        ])?;
    }
    Ok(())
}

/// Additive columns on `db_agents`, `db_bundles` and the other tables that grew after their first release.
pub(super) fn add_agent_columns(conn: &Connection) -> Result<(), StoreError> {
    apply_additive_columns(conn, &[
        "ALTER TABLE db_agents ADD COLUMN last_block_id TEXT NOT NULL DEFAULT ''",
        "ALTER TABLE db_agents ADD COLUMN container_image TEXT NOT NULL DEFAULT ''",
        "ALTER TABLE db_agents ADD COLUMN container_volumes TEXT NOT NULL DEFAULT '[]'",
        "ALTER TABLE db_agents ADD COLUMN container_name TEXT NOT NULL DEFAULT ''",
        // NOTE: targets db_bundles, not db_memory_bundles — adopt_legacy_table_names
        // (called at the top of `run_object_schema`) has already renamed the table
        // by the time this runs, on every DB that goes through the Phase 4
        // rename. See SPEC_ARMORY_PHASE4_STORAGE_RENAME_COMPLETION_2026_07_12.md.
        "ALTER TABLE db_bundles ADD COLUMN is_global INTEGER NOT NULL DEFAULT 0",
        "ALTER TABLE db_bundles ADD COLUMN sort_order INTEGER NOT NULL DEFAULT 0",
        // v12: explicit per-agent ambient-login opt-in (fail-by-default spawn
        // gating — SPEC_ACCOUNT_DELETE_DEAUTH_LAYERS_2_4_2026_07_14.md §2.3).
        "ALTER TABLE db_agents ADD COLUMN use_ambient_login INTEGER NOT NULL DEFAULT 0",
        // v15: model vendor override — redirects a harness at a non-default
        // backend (e.g. ANTHROPIC_BASE_URL). Formalizes harness vs. model
        // vendor as distinct concepts; see ProviderConfig::base_url_env_var.
        "ALTER TABLE db_agents ADD COLUMN model_vendor_base_url TEXT NOT NULL DEFAULT ''",
        // v16: provider-scoped bundle instructions (ABF v0.2 §2.2) — JSON
        // object of {provider_id: content} variants, additive to the
        // existing flat `instructions` column.
        "ALTER TABLE db_bundles ADD COLUMN instructions_by_provider TEXT NOT NULL DEFAULT '{}'",
        // v17: Warden Supervisor auto-continue opt-in. RETIRED: unused since
        // the Warden Supervisor was removed; the ALTER stays so a pre-v17 DB
        // still gets the column older builds sharing the data dir select.
        "ALTER TABLE db_agents ADD COLUMN auto_continue_enabled INTEGER NOT NULL DEFAULT 0",
        // v19: the agent's own dedicated ABF bundle, set once at creation.
        // db_agents gets a SEPARATE default_memory_id column (not the
        // existing instance-scoped memory_id) — see its own CREATE TABLE
        // comment for why, and OBJECT_SCHEMA_VERSION's v19 entry.
        "ALTER TABLE db_agents ADD COLUMN default_memory_id TEXT NOT NULL DEFAULT ''",
        // v26: muxspect Phase B/C per-agent conversation-disclosure policy —
        // see OBJECT_SCHEMA_VERSION's v26 doc comment in migrations.rs. Dual-written
        // to db_agents under the SAME column name (a simple opt-in-style
        // setting, like auto_continue_enabled above — not a differently-
        // named-on-db_agents case like memory_id/default_memory_id).
        "ALTER TABLE db_agents ADD COLUMN conversation_visibility TEXT NOT NULL DEFAULT 'private'",
        // v27: AgentMux-controlled, highest-priority Global Bundle tier —
        // see OBJECT_SCHEMA_VERSION's v27 doc comment in migrations.rs.
        "ALTER TABLE db_bundles ADD COLUMN is_system INTEGER NOT NULL DEFAULT 0",
        // v29: the agent's latest launch state on db_agents — the four
        // db_agent_instances columns the consolidated row still lacked. See
        // OBJECT_SCHEMA_VERSION's v29 doc comment in migrations.rs and the column doc in
        // db_agents' CREATE TABLE. Backfilled from db_agent_instances by
        // m0025_agents_launch_state_backfill; kept current by the instance
        // dual-write until the legacy table is dropped.
        "ALTER TABLE db_agents ADD COLUMN session_id TEXT NOT NULL DEFAULT ''",
        "ALTER TABLE db_agents ADD COLUMN status TEXT NOT NULL DEFAULT ''",
        "ALTER TABLE db_agents ADD COLUMN started_at INTEGER NOT NULL DEFAULT 0",
        "ALTER TABLE db_agents ADD COLUMN ended_at INTEGER NOT NULL DEFAULT 0",
        // v39: identity M4c-1 — the Global Memory writer's UID beside its
        // name. See OBJECT_SCHEMA_VERSION's v39 doc comment in migrations.rs.
        "ALTER TABLE db_bundle_versions ADD COLUMN written_by_uid TEXT NOT NULL DEFAULT ''",
        // v41: the subagent that owns a background task. See
        // OBJECT_SCHEMA_VERSION's v41 doc comment in migrations.rs.
        "ALTER TABLE db_background_tasks ADD COLUMN owner_tool_use_id TEXT",
        // v42: the Runtime menu choices the user last made for the agent. See
        // OBJECT_SCHEMA_VERSION's v42 doc comment in migrations.rs.
        "ALTER TABLE db_agents ADD COLUMN last_runtime TEXT NOT NULL DEFAULT ''",
        // v43 (see its doc comment), then a device's send grant (no bump: older builds ignore it).
        "ALTER TABLE db_agents ADD COLUMN hide_from_devices INTEGER NOT NULL DEFAULT 0",
        "ALTER TABLE db_viewer_devices ADD COLUMN can_send INTEGER NOT NULL DEFAULT 0",
        // A held jekt's files as manifest JSON (SPEC_JEKT_FILE_TRANSFER_2026_10_05.md §4.5), and
        // why it is held ('' or `idle`). No bump: an older build ignores them, replays text only.
        "ALTER TABLE db_jekt_held ADD COLUMN files_json TEXT NOT NULL DEFAULT ''",
        "ALTER TABLE db_jekt_held ADD COLUMN hold_reason TEXT NOT NULL DEFAULT ''",
        // A held WAN message's verdict, kept for its replay (no bump: an older build ignores it).
        "ALTER TABLE db_jekt_held ADD COLUMN wan_json TEXT NOT NULL DEFAULT ''",
        "ALTER TABLE db_agent_native_memory ADD COLUMN last_seen_mtime_ms INTEGER NOT NULL DEFAULT 0", // a table made before this column
    ])?;
    Ok(())
}

/// The blank Bundle singleton the launch UI offers as its default.
pub(super) fn seed_blank_bundle(conn: &Connection) -> Result<(), StoreError> {
    // ---- Seed blank Bundle singleton ----
    // The launch UI renders this as the default option in its Bundle
    // dropdown. Fixed id so tests + dev seed data can hard-code
    // references.
    conn.execute_batch(
        "INSERT OR IGNORE INTO db_bundles
            (id, name, description, is_blank, created_at, updated_at)
         VALUES ('blank', '__blank__', 'Vanilla CLI — no instructions, no context', 1, 0, 0);",
    )?;
    Ok(())
}

/// Channel-scoped migration tracking.
pub(super) fn create_channel_migrations(conn: &Connection) -> Result<(), StoreError> {
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

/// Run each additive `ALTER`, ignoring a column that is already there.
fn apply_additive_columns(conn: &Connection, stmts: &[&str]) -> Result<(), StoreError> {
    for stmt in stmts {
        if let Err(e) = conn.execute_batch(stmt) {
            let msg = e.to_string();
            if !msg.contains("duplicate column") {
                return Err(e.into());
            }
        }
    }
    Ok(())
}
