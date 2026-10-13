// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Armory Bundle Format (ABF) import — Phase 3
// (crates/srv/src/server/app_api/bundle/import.rs). Window-scoped, no agent_id
// gate, same as the rest of `bundle.*`. See
// docs/specs/SPEC_ABF_IMPORT_UI_PHASE3_2026_08_02.md.

// The bundle CRUD shapes below are GENERATED from their Rust definitions by
// ts-rs. See docs/specs/SPEC_RPC_BINDINGS_CODEGEN_2026_09_07.md §3.4 step 2.
// `BundleImportApi` at the bottom is NOT migrated yet — those three commands
// need their private `Req` structs promoted out of the handler file first.
export type { Bundle } from "@/types/rpc/Bundle";
// Global Memory version history (globalmemory:history/diff/revert) — see
// docs/specs/SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.3.
export type { GlobalMemoryImportReport } from "@/types/rpc/GlobalMemoryImportReport";
export type { GlobalMemoryImportSource } from "@/types/rpc/GlobalMemoryImportSource";
export type { GlobalMemoryImportSources } from "@/types/rpc/GlobalMemoryImportSources";
export type { GlobalMemorySection } from "@/types/rpc/GlobalMemorySection";
export type { GlobalMemoryVersionMeta } from "@/types/rpc/GlobalMemoryVersionMeta";

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
export type { BundleImportCommitResponse } from "@/types/rpc/BundleImportCommitResponse";
export type { BundleImportContextFilePreview } from "@/types/rpc/BundleImportContextFilePreview";
export type { BundleImportFileEntry } from "@/types/rpc/BundleImportFileEntry";
export type { BundleImportMcpServerDisplay } from "@/types/rpc/BundleImportMcpServerDisplay";
export type { BundleImportMcpServerPreview } from "@/types/rpc/BundleImportMcpServerPreview";
export type { BundleImportPreviewResponse } from "@/types/rpc/BundleImportPreviewResponse";
export type { BundleImportProjectInstructionPreview } from "@/types/rpc/BundleImportProjectInstructionPreview";
export type { BundleImportRequirementPreview } from "@/types/rpc/BundleImportRequirementPreview";
export type { BundleImportSkillPreview } from "@/types/rpc/BundleImportSkillPreview";
export type { BundleImportSkillSelection } from "@/types/rpc/BundleImportSkillSelection";
export type { BundleImportUnresolvedRequirement } from "@/types/rpc/BundleImportUnresolvedRequirement";
export type { IssueSeverity as BundleValidationSeverity } from "@/types/rpc/IssueSeverity";
export type { ValidationIssue as BundleValidationIssue } from "@/types/rpc/ValidationIssue";

import type { Bundle as BundleT } from "@/types/rpc/Bundle";
import type { CommandUpsertBundleData } from "@/types/rpc/CommandUpsertBundleData";

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

// What `upsertmemory`/`upsertsystemmemory` actually accept: the above plus the
// optional `base_sha256` from the generated `CommandUpsertBundleData` (which
// flattens `Bundle`, so its own generated shape marks every Bundle field
// required and can't serve as the input type directly). With a base, a save
// whose entry has moved is refused with a `conflict:` error — SHA-256 of
// `name + "\0" + instructions`. See
// docs/specs/SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.4.
export type BundleUpsertRequest = BundleUpsertInput & Pick<CommandUpsertBundleData, "base_sha256">;

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
