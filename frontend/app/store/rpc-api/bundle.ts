// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Armory Bundle Format (ABF) import — Phase 3
// (agentmux-srv/src/server/app_api/bundle.rs). Window-scoped, no agent_id
// gate, same as the rest of `bundle.*`. See
// docs/specs/SPEC_ABF_IMPORT_UI_PHASE3_2026_08_02.md.

import { RpcClient } from "../rpc-client";

// The bundle CRUD shapes below are GENERATED from their Rust definitions by
// ts-rs. See docs/specs/SPEC_RPC_BINDINGS_CODEGEN_2026_09_07.md §3.4 step 2.
// `BundleImportApi` at the bottom is NOT migrated yet — those three commands
// need their private `Req` structs promoted out of the handler file first.
export type { Bundle } from "@/types/rpc/Bundle";

// The validation report shapes are GENERATED too. The validate HANDLER stays
// on `register_handler` on purpose (it normalizes its payload before
// deserializing, which `register_typed` cannot express -- see the comment on
// `register_bundle_validate`), but its response was always the part at risk of
// drift, and that part is now generated.
export type { ValidationReport as BundleValidationReport } from "@/types/rpc/ValidationReport";

// The import preview/commit shapes are generated too, as of this change. They
// previously had NO Rust type at all -- both handlers built their responses as
// ad-hoc `json!` literals -- so the declarations here were hand-maintained
// against nothing.
export type { BundleImportPreviewResponse } from "@/types/rpc/BundleImportPreviewResponse";
export type { BundleImportCommitResponse } from "@/types/rpc/BundleImportCommitResponse";
export type { BundleImportContextFilePreview } from "@/types/rpc/BundleImportContextFilePreview";
export type { BundleImportSkillPreview } from "@/types/rpc/BundleImportSkillPreview";
export type { BundleImportMcpServerPreview } from "@/types/rpc/BundleImportMcpServerPreview";
export type { BundleImportMcpServerDisplay } from "@/types/rpc/BundleImportMcpServerDisplay";
export type { BundleImportRequirementPreview } from "@/types/rpc/BundleImportRequirementPreview";
export type { BundleImportProjectInstructionPreview } from "@/types/rpc/BundleImportProjectInstructionPreview";
export type { BundleImportUnresolvedRequirement } from "@/types/rpc/BundleImportUnresolvedRequirement";
export type { BundleImportFileEntry } from "@/types/rpc/BundleImportFileEntry";
export type { BundleImportSkillSelection } from "@/types/rpc/BundleImportSkillSelection";
export type { ValidationIssue as BundleValidationIssue } from "@/types/rpc/ValidationIssue";
export type { IssueSeverity as BundleValidationSeverity } from "@/types/rpc/IssueSeverity";

import type { Bundle as BundleT } from "@/types/rpc/Bundle";
import type { CommandListBundlesData } from "@/types/rpc/CommandListBundlesData";
import type { CommandGetBundleData } from "@/types/rpc/CommandGetBundleData";
import type { CommandDeleteBundleData } from "@/types/rpc/CommandDeleteBundleData";
import type { DeleteBundleResult } from "@/types/rpc/DeleteBundleResult";
import type { CommandReorderGlobalBundlesData } from "@/types/rpc/CommandReorderGlobalBundlesData";
import type { ReorderGlobalBundlesResult } from "@/types/rpc/ReorderGlobalBundlesResult";
import type { CommandGetClaudeGlobalConfigData } from "@/types/rpc/CommandGetClaudeGlobalConfigData";
import type { ClaudeGlobalConfig } from "@/types/rpc/ClaudeGlobalConfig";
import type { ValidationReport } from "@/types/rpc/ValidationReport";
import type { CommandBundleImportPreviewData } from "@/types/rpc/CommandBundleImportPreviewData";
import type { CommandBundleImportCommitData } from "@/types/rpc/CommandBundleImportCommitData";
import type { BundleImportPreviewResponse as BundleImportPreviewResponseT } from "@/types/rpc/BundleImportPreviewResponse";
import type { BundleImportCommitResponse as BundleImportCommitResponseT } from "@/types/rpc/BundleImportCommitResponse";

// The accurate request shape for the three upsert/validate commands, DERIVED
// from the generated `Bundle` rather than hand-listed, so a new Rust field
// flows through automatically and this cannot drift.
//
// This replaces `Partial<Bundle>`, which was wrong in a way that mattered:
// `Bundle.id` and `Bundle.name` are the only two fields WITHOUT
// `#[serde(default)]` on the Rust side, so a payload omitting `name` is
// rejected by serde at runtime — but `Partial<Bundle>` told callers omitting
// it was fine. Every other field defaults server-side, which is exactly what
// `Partial<Omit<...>>` says.
//
// (`Bundle` itself is all-required because the server always writes every
// field: they are `#[serde(default)]` but have no `skip_serializing_if`, so
// none is ever omitted from a response.)
export type BundleUpsertInput = Pick<BundleT, "id" | "name"> & Partial<Omit<BundleT, "id" | "name">>;

// `bundle.validate` takes the same payload EXCEPT that `id` is optional too,
// so an unsaved draft can be checked before it has one. That is not a guess
// from the method comment: `bundle_validate_impl` runs its input through
// `normalize_bundle_upsert_input`, which inserts `id: ""` when the key is
// missing or null, and then branches on `memory.id.is_empty()` to skip the
// component-binding lookups for a draft.
//
// The upsert commands genuinely do NOT share that leniency, which is why this
// is a separate type rather than one shared "bundle input": `upsertmemory` and
// `upsertsystemmemory` deserialize `Bundle` straight from the payload with no
// normalize step, so `id` is required there. (The app_api `bundle.upsert`
// command does normalize, but that is a different command and not the one this
// stub calls.)
export type BundleValidateInput = Pick<BundleT, "name"> & Partial<Omit<BundleT, "name">>;

// Bundle CRUD (db_bundles rows; Rust: Store::bundle_*). The wire command
// names below (`listmemories`, `getmemory`, ...) are the legacy aliases the
// backend still registers alongside `bundle.*` — the strings must not change
// until Phase 3 of SPEC_ARMORY_NAMING_CONSOLIDATION_2026_09_09.md retires them.
export const BundleApi = {
    ListBundlesCommand(
        client: RpcClient,
        data: CommandListBundlesData = {},
        opts?: RpcOpts,
    ): Promise<BundleT[]> {
        return client.rpcCall("listmemories", data, opts);
    },

    GetBundleCommand(
        client: RpcClient,
        data: CommandGetBundleData,
        opts?: RpcOpts,
    ): Promise<BundleT> {
        return client.rpcCall("getmemory", data, opts);
    },

    UpsertBundleCommand(
        client: RpcClient,
        data: BundleUpsertInput,
        opts?: RpcOpts,
    ): Promise<BundleT> {
        return client.rpcCall("upsertmemory", data, opts);
    },

    DeleteBundleCommand(
        client: RpcClient,
        data: CommandDeleteBundleData,
        opts?: RpcOpts,
    ): Promise<DeleteBundleResult> {
        return client.rpcCall("deletememory", data, opts);
    },

    // `ids` is the full ordered list of global bundle ids.
    ReorderGlobalBundlesCommand(
        client: RpcClient,
        data: CommandReorderGlobalBundlesData,
        opts?: RpcOpts,
    ): Promise<ReorderGlobalBundlesResult> {
        return client.rpcCall("reorderglobalbrain", data, opts);
    },

    // System-tier Global Memory — see
    // docs/specs/SPEC_GLOBAL_MEMORY_SYSTEM_TIER_2026_08_24.md. The ONLY
    // commands that can write is_system=true; deliberately separate from
    // UpsertBundleCommand/DeleteBundleCommand.
    UpsertSystemBundleCommand(
        client: RpcClient,
        data: BundleUpsertInput,
        opts?: RpcOpts,
    ): Promise<BundleT> {
        return client.rpcCall("upsertsystemmemory", data, opts);
    },

    DeleteSystemBundleCommand(
        client: RpcClient,
        data: CommandDeleteBundleData,
        opts?: RpcOpts,
    ): Promise<DeleteBundleResult> {
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
        data: CommandGetClaudeGlobalConfigData = {},
        opts?: RpcOpts,
    ): Promise<ClaudeGlobalConfig> {
        return client.rpcCall("getclaudeglobalconfig", data, opts);
    },

};


export const BundleImportApi = {
    // `data` takes all three input modes the server actually supports
    // (`file_path`, `zip_base64`, `files`), which the previous hand-written
    // `{ file_path: string }` did not -- two of them were unreachable through
    // this client even though `resolve_import_input` has always accepted them.
    BundleImportPreviewCommand(
        client: RpcClient,
        data: CommandBundleImportPreviewData,
        opts?: RpcOpts,
    ): Promise<BundleImportPreviewResponseT> {
        return client.rpcCall("bundle.import.preview", data, opts);
    },

    BundleImportCommitCommand(
        client: RpcClient,
        data: CommandBundleImportCommitData,
        opts?: RpcOpts,
    ): Promise<BundleImportCommitResponseT> {
        return client.rpcCall("bundle.import.commit", data, opts);
    },

    // Structural-only check (agentmux-srv/src/backend/bundle_validate.rs) —
    // read-only, no Store write. Takes `BundleValidateInput`, which is
    // `BundleUpsertInput` minus the `id` requirement, so it can validate an
    // unsaved draft (including a brand-new bundle with no id yet), not just
    // what's already persisted.
    ValidateBundleCommand(
        client: RpcClient,
        data: BundleValidateInput,
        opts?: RpcOpts,
    ): Promise<ValidationReport> {
        return client.rpcCall("bundle.validate", data, opts);
    },
};
