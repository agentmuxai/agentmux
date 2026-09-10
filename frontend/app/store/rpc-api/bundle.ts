// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Armory Bundle Format (ABF) import — Phase 3
// (agentmux-srv/src/server/app_api/bundle.rs). Window-scoped, no agent_id
// gate, same as the rest of `bundle.*`. See
// docs/specs/SPEC_ABF_IMPORT_UI_PHASE3_2026_08_02.md.

import { RpcClient } from "../rpc-client";
// Bundle CRUD (db_bundles rows; Rust: Store::bundle_*). The wire command
// names below (`listmemories`, `getmemory`, ...) are the legacy aliases the
// backend still registers alongside `bundle.*` — the strings must not change
// until Phase 3 of SPEC_ARMORY_NAMING_CONSOLIDATION_2026_09_09.md retires them.
export const BundleApi = {
    ListBundlesCommand(
        client: RpcClient,
        data: Record<string, never> = {},
        opts?: RpcOpts,
    ): Promise<Bundle[]> {
        return client.rpcCall("listmemories", data, opts);
    },

    GetBundleCommand(
        client: RpcClient,
        data: { id: string },
        opts?: RpcOpts,
    ): Promise<Bundle> {
        return client.rpcCall("getmemory", data, opts);
    },

    UpsertBundleCommand(
        client: RpcClient,
        data: Partial<Bundle>,
        opts?: RpcOpts,
    ): Promise<Bundle> {
        return client.rpcCall("upsertmemory", data, opts);
    },

    DeleteBundleCommand(
        client: RpcClient,
        data: { id: string },
        opts?: RpcOpts,
    ): Promise<{ deleted: boolean }> {
        return client.rpcCall("deletememory", data, opts);
    },

    // `ids` is the full ordered list of global bundle ids.
    ReorderGlobalBundlesCommand(
        client: RpcClient,
        data: { ids: string[] },
        opts?: RpcOpts,
    ): Promise<{ updated: number }> {
        return client.rpcCall("reorderglobalbrain", data, opts);
    },

    // System-tier Global Memory — see
    // docs/specs/SPEC_GLOBAL_MEMORY_SYSTEM_TIER_2026_08_24.md. The ONLY
    // commands that can write is_system=true; deliberately separate from
    // UpsertBundleCommand/DeleteBundleCommand.
    UpsertSystemBundleCommand(
        client: RpcClient,
        data: Partial<Bundle>,
        opts?: RpcOpts,
    ): Promise<Bundle> {
        return client.rpcCall("upsertsystemmemory", data, opts);
    },

    DeleteSystemBundleCommand(
        client: RpcClient,
        data: { id: string },
        opts?: RpcOpts,
    ): Promise<{ deleted: boolean }> {
        return client.rpcCall("deletesystemmemory", data, opts);
    },

    // Read-only. Returns the CLAUDE.md at AgentMux's shared Claude
    // provider config dir (~/.agentmux/shared/providers/claude/CLAUDE.md
    // via DataPaths::provider_auth_dir — the CLAUDE_CONFIG_DIR a
    // non-identity-bound spawned Claude agent actually gets). NOTE: this
    // is Claude Code's own home-relocation path, NOT the file AgentMux's
    // Global Memory actually composes into (that's
    // <agent working_directory>/CLAUDE.md, a per-agent path this doesn't
    // cover) — this is a "Claude Code provider config" reference display
    // only. See docs/specs/SPEC_SURFACE_CLAUDE_GLOBAL_CONFIG_2026_08_24.md
    // §5, §7. No parameters, no write counterpart.
    GetClaudeGlobalConfigCommand(
        client: RpcClient,
        data: Record<string, never> = {},
        opts?: RpcOpts,
    ): Promise<{ path: string; content: string | null; exists: boolean }> {
        return client.rpcCall("getclaudeglobalconfig", data, opts);
    },

};


export const BundleImportApi = {
    BundleImportPreviewCommand(
        client: RpcClient,
        data: { file_path: string },
        opts?: RpcOpts,
    ): Promise<BundleImportPreviewResponse> {
        return client.rpcCall("bundle.import.preview", data, opts);
    },

    BundleImportCommitCommand(
        client: RpcClient,
        data: {
            file_path: string;
            expected_content_digest: string;
            bundle_name?: string;
            include_instructions: boolean;
            include_context_files: number[];
            include_skills: { source_dir: string; import_as?: string }[];
            include_mcp_servers: string[];
        },
        opts?: RpcOpts,
    ): Promise<BundleImportCommitResponse> {
        return client.rpcCall("bundle.import.commit", data, opts);
    },

    // Structural-only check (agentmux-srv/src/backend/bundle_validate.rs) —
    // read-only, no Store write. Accepts the same payload shape
    // `UpsertBundleCommand` does, so it can validate an unsaved draft
    // (including a brand-new bundle with no id yet), not just what's
    // already persisted.
    ValidateBundleCommand(
        client: RpcClient,
        data: Partial<Bundle>,
        opts?: RpcOpts,
    ): Promise<BundleValidationReport> {
        return client.rpcCall("bundle.validate", data, opts);
    },
};
