// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Component plumbing shared by the per-agent export and import paths:
//! purging dangling component refs, resolving a bundle's components to their
//! backing rows, and splicing history/instructions/memory into a manifest.

use super::*;

/// Drop a deleted bundle's component refs.
///
/// Called from **both** delete paths — `bundle.delete` here and `deletememory`
/// in `agent_handlers/bundle.rs`, which is the one the Armory actually calls.
/// Fixing only the first left the product path orphaning refs (Codex, PR
/// #3153), so this exists to make "both" mean one implementation.
///
/// `bundle_delete` runs on the store that owns `db_bundles`, which has no ref
/// tables — they sit beside the catalog tables they key into, in the other
/// database, which is also why no foreign key reaches across
/// (`migrations.rs:721-741`). So the purge cannot live inside it and has to be
/// driven from a handler, where both stores are in scope.
///
/// Best-effort by design: the bundle row is already gone by the time this
/// runs, so a failure here must not turn a successful delete into an error.
pub(crate) fn purge_bundle_component_refs(
    mstore: &crate::backend::storage::store::Store,
    bundle_id: &str,
) {
    match mstore.bundle_unbind_all_components(bundle_id) {
        Ok((skills, mcp)) if skills + mcp > 0 => {
            tracing::info!(bundle_id, skills, mcp, "bundle delete: purged component refs")
        }
        Ok(_) => {}
        Err(e) => tracing::warn!(
            bundle_id, error = %e,
            "bundle delete: component refs left behind"
        ),
    }
}

/// Bind an imported bundle's components into the ref tables.
///
/// Phase 0b. The ref tables are authoritative for what a bundle contains, so
/// an import that only wrote the inline columns would produce a bundle that
/// exports empty and, for MCP servers, never materialises at spawn either —
/// the `SPEC_BUNDLE_AS_CONTAINER_V2_2026_08_17.md` "inert at runtime" state
/// the ref tables exist to end.
///
/// **Must run after `bundle_upsert`.** Both bind paths check the bundle exists
/// in `id_store` first (`storage/managed.rs:291`) and refuse otherwise.
///
/// Returns warnings rather than failing: by this point the bundle row is
/// already committed, so aborting would leave a half-imported bundle behind. A
/// component that could not be bound is reported and the import continues.
pub(super) fn bind_imported_components(
    mstore: &crate::backend::storage::store::Store,
    id_store: &crate::backend::storage::store::Store,
    identity_store: &crate::backend::storage::store::Store,
    bundle_id: &str,
    imported_skill_ids: &[String],
    mcp_configs: &[serde_json::Value],
) -> Vec<String> {
    let mut warnings: Vec<String> = Vec::new();

    for skill_id in imported_skill_ids {
        if let Err(e) = mstore.bundle_skill_bind(identity_store, id_store, bundle_id, skill_id) {
            warnings.push(format!(
                "skills: imported skill {skill_id} could not be bound to the bundle ({e}) — it will not be exported or reach the agent"
            ));
        }
    }

    // Same shared path the m0030 backfill uses, so a server that arrives by
    // import and one recovered from an old inline column end up identical,
    // including duplicate-name handling.
    let (_created, mcp_warnings) =
        mstore.bundle_mcp_bind_inline_entries(identity_store, id_store, bundle_id, mcp_configs, now_ms());
    warnings.extend(mcp_warnings);

    warnings
}

/// A bundle's components, resolved from the tables that are authoritative for
/// them.
pub(crate) struct ResolvedComponents {
    pub(crate) skills: Vec<crate::backend::storage::Skill>,
    /// Exporter entry shape: each server's config object with its `name`
    /// alongside, which is what `bundle_export::redact_mcp_entry` reads.
    pub(crate) mcp_entries: Vec<serde_json::Value>,
    pub(crate) warnings: Vec<String>,
}

/// Resolve what a bundle actually contains, from `db_bundle_skills_ref` /
/// `db_bundle_mcp_ref`.
///
/// Phase 0b of `SPEC_INSTRUCTION_AND_MEMORY_PORTABILITY_2026_09_09.md`. Before
/// this, export read the inline `bundle.skills` / `bundle.mcp_servers` columns
/// while agent launch read the ref tables (`app_api/agent_open.rs:793`, `:812`),
/// and nothing kept the two in step — so a skill bound in the Armory ran at
/// launch but exported as an empty `skills/` directory, with no warning. One
/// resolver, used by every export path, is what stops that recurring.
///
/// **Scope note:** this is deliberately narrower than `effective_skills` /
/// `effective_mcp_servers`, which additionally union an *agent's* own binds,
/// globals and the legacy blob. Those answer "what does this agent run with";
/// this answers "what is in this bundle", which is what an ABF describes. The
/// two are not meant to be equal, only to agree about the bundle's own
/// contribution.
///
/// `managed_list` returns the whole catalog with a `bound_to_bundle` flag
/// rather than just the bound rows, hence the filter; and because it joins
/// through the catalog table, a ref whose catalog row has been deleted
/// resolves to nothing rather than reporting itself. `skill_delete` /
/// `mcp_server_delete` both purge refs (`storage/managed.rs:232`), so that
/// state is not reachable through the app — but the FK that would enforce it
/// is inert, since `PRAGMA foreign_keys` is only ever set ON in tests.
pub(crate) fn resolve_bundle_components(
    mstore: &crate::backend::storage::store::Store,
    identity_store: &crate::backend::storage::store::Store,
    bundle_id: &str,
) -> Result<ResolvedComponents, String> {
    let skills: Vec<crate::backend::storage::Skill> = mstore
        .bundle_skill_list(identity_store, bundle_id)
        .map_err(|e| format!("failed to resolve bundle skills: {e}"))?
        .into_iter()
        .filter(|item| item.bound_to_bundle)
        .map(|item| item.skill)
        .collect();

    let mut warnings: Vec<String> = Vec::new();
    let mut mcp_entries: Vec<serde_json::Value> = Vec::new();
    for item in mstore
        .bundle_mcp_list(identity_store, bundle_id)
        .map_err(|e| format!("failed to resolve bundle MCP servers: {e}"))?
        .into_iter()
        .filter(|item| item.bound_to_bundle)
    {
        let server = item.server;
        // `config` is the same JSON object the launch path writes into
        // `.mcp.json` keyed by `server.name` (`agent_config.rs`
        // build_mcp_config_from_refs). The exporter wants that object with the
        // name inline, so the row's name is authoritative over any `name` the
        // config blob happens to carry.
        match serde_json::from_str::<serde_json::Value>(&server.config) {
            Ok(serde_json::Value::Object(mut obj)) => {
                obj.insert("name".to_string(), json!(server.name));
                mcp_entries.push(serde_json::Value::Object(obj));
            }
            Ok(_) => warnings.push(format!(
                "mcp server {}: config is not a JSON object — skipped",
                server.name
            )),
            Err(e) => warnings.push(format!(
                "mcp server {}: invalid config JSON ({e}) — skipped",
                server.name
            )),
        }
    }

    Ok(ResolvedComponents { skills, mcp_entries, warnings })
}

/// Warning the agent-less `bundle.export` pushes when the bundle it is
/// exporting is bound to an agent that actually has native memory.
///
/// Mirror of [`crate::backend::bundle_import::MEMORY_COMPONENT_IGNORED_WARNING`]
/// on the import side, and a constant for the same reason that one is: the push
/// site and the test that pins the wording must not drift apart.
///
/// `SPEC_INSTRUCTION_AND_MEMORY_PORTABILITY_2026_09_09.md` §3.1/§5.1. The
/// asymmetry worth fixing was never that `bundle.export` omits memory — a
/// bundle detached from any agent has no memory to carry, and omitting it is
/// correct. It is that import *announces* the omission and export did not,
/// while every `components` key is optional (`bundle_export.rs:586-606`), so a
/// missing `memory` key is indistinguishable from "this bundle had none".
pub(crate) const MEMORY_NOT_EXPORTED_WARNING: &str =
    "memory: this bundle is bound to an agent with native memory, which bundle.export does not carry — use bundle.export_for_agent to include it";

/// Does any agent bound to `bundle_id` have mirrored native memory?
///
/// **Both bindings count.** A bundle is reachable two ways, and they are
/// different columns: `AgentDefinition.memory_id` is the agent's own dedicated
/// bundle (stored in `db_agents.default_memory_id`), while
/// `AgentInstance.memory_id` is one *launch* deliberately pointed at some other
/// bundle (`storage/agents.rs:153-163`). Checking only the definition would
/// miss exactly the case the operator is most likely to hit — exporting the
/// bundle a running instance was launched with. Native memory is keyed on the
/// definition id either way (`build_export_for_agent` resolves the agent with
/// `agent_def_get` before reading memory), so the instance path resolves
/// through `definition_id`.
///
/// Reads the `db_agent_native_memory` mirror and deliberately does NOT call
/// `refresh_memory_mirror_from_live_fs` first, unlike `build_export_for_agent`
/// (which must, because it is about to *copy* the files —
/// `app_api/bundle.rs:558`). Refreshing writes to the store, and an agent-less
/// export has no business mutating per-agent state as a side effect of
/// producing a warning.
///
/// **Known limitation — a cross-channel binding is invisible here, and cannot
/// currently be made visible.** Both lookups resolve the binding from
/// channel-local SQLite: `instance_list` reads this channel's rows, and
/// `agent_def_list`'s global overlay only preserves `memory_id` when a local
/// row exists (`storage/agents.rs:462`). An agent created in another channel
/// has no local row, so it comes back with an empty `memory_id` — because
/// `DefinitionRecordV1` does not carry the field at all
/// (`storage/def_registry_mirror.rs:128-133`). The bundle and the memory mirror
/// are global; only the binding between them is not. So exporting a shared
/// bundle from a channel other than the one its agent was created in will not
/// warn, which is exactly the portability case the warning is for (Codex, PR
/// #3147).
///
/// Closing it means adding `memory_id` to the registry wire format, which is
/// already a tracked gap for bigger reasons than this warning — the same
/// missing field makes a cross-channel reopen start unbound and limits m0021's
/// backfill to local SQLite. Tracked in #3148; deliberately not widened here,
/// because warning whenever the binding is merely *unknown* would fire on
/// unrelated bundles and train operators to ignore it.
///
/// The other cost is under-warning on a stale mirror: an agent whose memory
/// exists on disk but was never mirrored is missed. That is the residual gap
/// `SPEC_NATIVE_MEMORY_DURABLE_SYNC_2026_08_07.md` already documents and
/// accepts for the mirror generally. Both costs are one-directional and this
/// warning is advisory — it changes no bytes in the archive.
pub(super) fn bound_agent_has_native_memory(
    id_store: &crate::backend::storage::store::Store,
    mstore: &crate::backend::storage::store::Store,
    bundle_id: &str,
) -> bool {
    // A blank bundle_id is the "blank singleton" sentinel that every unbound
    // agent carries — treating it as a binding would warn on nearly every
    // export.
    if bundle_id.is_empty() {
        return false;
    }

    let has_memory = |agent_id: &str| {
        id_store
            .agent_native_memory_list_meta(agent_id)
            .map(|rows| !rows.is_empty())
            .unwrap_or(false)
    };

    // Best-effort throughout: a store error here must not fail an export that
    // is otherwise fine. Neither table is read by the export itself.
    let bound_by_definition = mstore
        .agent_def_list()
        .map(|agents| {
            agents
                .iter()
                .filter(|a| a.memory_id == bundle_id)
                .any(|a| has_memory(&a.id))
        })
        .unwrap_or(false);
    if bound_by_definition {
        return true;
    }

    mstore
        .instance_list(None, None)
        .map(|instances| {
            instances
                .iter()
                .filter(|i| i.memory_id == bundle_id)
                .any(|i| has_memory(&i.definition_id))
        })
        .unwrap_or(false)
}

/// Register `entries` under `components.<key>` in the export's `armory.json`.
///
/// Takes JSON values rather than paths: most components are a list of archive
/// paths, but `projectInstructions` carries an object per file, because a bare
/// path could not express the `owner` discriminator that keeps import from
/// installing somebody else's instructions.
///
/// The manifest is built as an inline `json!` literal in `bundle_export.rs`
/// with no struct behind it (`bundle_export.rs:615`), so every agent-aware
/// component has to reopen and rewrite it. Shared by the memory splice below
/// and the history splice, which otherwise duplicated the find-parse-rewrite
/// dance byte for byte.
///
/// No-ops on an empty list: `components` keys are all optional, and writing
/// `"history": []` would assert "this archive has an empty history" where
/// absence already says "none".
pub(super) fn set_manifest_component(
    export: &mut crate::backend::bundle_export::BundleExport,
    key: &str,
    entries: Vec<serde_json::Value>,
) -> Result<(), String> {
    if entries.is_empty() {
        return Ok(());
    }
    let manifest_idx = export
        .files
        .iter()
        .position(|f| f.path == "armory.json")
        .ok_or_else(|| "bundle export is missing armory.json".to_string())?;
    let mut manifest: serde_json::Value = serde_json::from_str(&export.files[manifest_idx].content)
        .map_err(|e| format!("armory.json: {e}"))?;
    manifest["components"][key] = json!(entries);
    export.files[manifest_idx].content =
        serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?;
    Ok(())
}

/// Register the transcript files `bundle.export_for_agent_with_history` already
/// wrote into the archive under `components.history`.
///
/// `SPEC_INSTRUCTION_AND_MEMORY_PORTABILITY_2026_09_09.md` §3.5: the files were
/// pushed into `export.files` (`app_api/bundle.rs:797`) and counted in the RPC
/// response, but the manifest was never touched after the memory splice — so a
/// consumer reading `components.*` to learn what an archive holds could not see
/// history at all. The paths are caller-supplied rather than re-derived here so
/// this cannot drift from what was actually written.
pub(super) fn splice_history_component(
    export: &mut crate::backend::bundle_export::BundleExport,
    history_paths: Vec<String>,
) -> Result<(), String> {
    set_manifest_component(export, "history", history_paths.into_iter().map(|p| json!(p)).collect())
}

/// Splice native-memory files into an already-built bundle export's
/// `armory.json` manifest and files list, adding `components.memory` (ABF
/// v0.2 §2.3). Kept OUTSIDE `bundle_export.rs` deliberately — that
/// module's `export_bundle()` is scoped to a bundle's own components
/// (instructions/skills/MCP/accounts) with no concept of "agent" or
/// native memory at all; memory is agent-scoped, not bundle-scoped, so
/// splicing it in here (the RPC-handler layer, which already resolves
/// other agent-scoped data like skill rows) keeps that module's
/// documented scope intact rather than growing it a fifth, unrelated
/// component category. A no-op when `memory_files` is empty — matches the
/// existing omit-empty-components convention used elsewhere in the
/// manifest.
/// Carry the agent's project instructions into the export as
/// `components.projectInstructions`.
///
/// Phase 3 of `SPEC_INSTRUCTION_AND_MEMORY_PORTABILITY_2026_09_09.md`. Files
/// land under `instructions/project/<sanitized-path>`, and each manifest entry
/// carries its original `path`, a `contentHash`, and an `owner`.
///
/// **`owner` is the load-bearing field.** It is what stops this from becoming a
/// way to install one repository's instructions into another: an importer that
/// treated a `foreign` entry as content to write would be doing precisely what
/// `SPEC_CLAUDE_MD_OWNERSHIP_PROTECTION_2026_08_22.md` exists to prevent.
/// Import surfaces these and never applies them (spec §5.2), so an operator who
/// wants them acts deliberately.
///
/// Only files that exist and were readable are carried — there is nothing to
/// put in an archive otherwise. Their absence from the manifest is therefore
/// not a claim that the source agent had none; `agent.project_instructions` is
/// where the fuller picture, including absent and unreadable files, lives.
///
/// "Readable" is decided by `error`/`truncated`, NOT by whether the content is
/// empty (Codex P2, PR #3163). A real, readable, zero-byte `CLAUDE.md` is a
/// fact about the source agent — dropping it would make the export
/// indistinguishable from one where the file was absent, which is the exact
/// kind of quiet lie this component exists to remove.
///
/// Paths go through the same sanitizer the exporter uses for context files, so
/// a scanned path can never write outside the archive's own tree.
pub(super) fn splice_project_instructions_component(
    export: &mut crate::backend::bundle_export::BundleExport,
    files: &[crate::backend::project_instructions::ProjectInstructionFile],
) -> Result<(), String> {
    use crate::backend::project_instructions::InstructionOwner;
    use std::collections::HashSet;

    let mut entries: Vec<serde_json::Value> = Vec::new();
    // Case-INSENSITIVE, for the same reason the context-file loop in
    // `bundle_export.rs` is (reagent P2, PR #2333): a case-sensitive source
    // filesystem can hold `.github/instructions/A.md` and `.../a.md`, and the
    // scan in `project_instructions.rs` will find both — but they collide on
    // extraction on Windows/macOS, where one snapshot silently overwrites the
    // other. Scanned directories make this reachable here in a way it isn't
    // for the fixed registry paths (Codex P2, PR #3163). The written path
    // keeps its original case; only the check is folded.
    let mut used_paths: HashSet<String> = HashSet::new();
    for file in files {
        if !file.exists || file.error.is_some() || file.truncated {
            continue;
        }
        let Some(safe) = crate::backend::bundle_export::sanitize_context_relative_path(&file.path)
        else {
            // Warn rather than dropping in silence — a backup tool that loses
            // an input without saying so defeats its own purpose (reagent P1,
            // PR #2333, on the context-file equivalent).
            export.warnings.push(format!(
                "projectInstructions: \"{}\" could not be sanitized into an \
                 archive-relative path; skipped",
                file.path
            ));
            continue;
        };
        let out_path = format!("instructions/project/{safe}");
        if !used_paths.insert(out_path.to_lowercase()) {
            export.warnings.push(format!(
                "projectInstructions: \"{}\" normalizes to the same path as an \
                 earlier entry ({out_path}); skipped to avoid overwriting it",
                file.path
            ));
            continue;
        }
        export.files.push(crate::backend::bundle_export::BundleExportFile {
            path: out_path.clone(),
            content: file.content.clone(),
        });
        entries.push(json!({
            "path": file.path,
            "file": out_path,
            "contentHash": file.content_hash,
            "owner": match file.owner {
                InstructionOwner::Agentmux => "agentmux",
                InstructionOwner::Foreign => "foreign",
            },
        }));
    }
    set_manifest_component(export, "projectInstructions", entries)
}

pub(super) fn splice_memory_component(
    export: &mut crate::backend::bundle_export::BundleExport,
    memory_files: &[(String, String)],
) -> Result<(), String> {
    if memory_files.is_empty() {
        return Ok(());
    }
    let mut manifest_memory: Vec<String> = Vec::new();
    for (filename, content) in memory_files {
        // Filenames here always came from db_agent_native_memory, which
        // only ever accepts app-validated names (validate_filename in
        // native_memory_handlers.rs: alphanumeric + "-_", ends ".md", no
        // separators) — unlike instructions_by_provider's keys (§2.2),
        // there's no untrusted-manifest path that could smuggle an unsafe
        // segment in here, so no additional sanitization is needed.
        let out_path = format!("memory/{filename}");
        export.files.push(crate::backend::bundle_export::BundleExportFile {
            path: out_path.clone(),
            content: content.clone(),
        });
        manifest_memory.push(out_path);
    }
    set_manifest_component(export, "memory", manifest_memory.into_iter().map(|p| json!(p)).collect())
}
