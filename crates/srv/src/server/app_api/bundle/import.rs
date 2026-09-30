// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `bundle.import`, `bundle.import_preview`, `bundle.import_commit`: the ABF
//! file/bytes intake path, its size and warning budgets, and the two-phase
//! preview/commit flow.

use super::*;

/// `bundle.import` — ABF importer, Phase 2 of
/// docs/specs/SPEC_ABF_V0_1_SINGLE_FILE_AND_IMPORTER_2026_08_01.md. Window-
/// scoped like `bundle.export` (bundles aren't agent-specific). Accepts
/// EITHER `zip_base64` (a `.abf` archive, mirroring `bundle.export`'s own
/// `zip_base64` response field) OR `files` (a raw `[{path, content}]`
/// list) — exactly one must be present. Hands the bytes to the pure
/// `bundle_import` module for parsing/validation, then owns every Store
/// side-effect: creates the bundle row, creates one global Skill row per
/// parsed skill, and performs READ-ONLY account-requirement lookups
/// (never creates an account, never writes a secret anywhere — see the
/// spec's §4.5 correction for why this stopped short of "resolving"
/// placeholders in place).
pub(super) fn register_bundle_import(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let id_store = state.id_store.clone();
    let identity_store = state.identity_store.clone();
    let mstore = state.mstore.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_BUNDLE_IMPORT,
        Box::new(move |data, _ctx| {
            let id_store = id_store.clone();
            let identity_store = identity_store.clone();
            let mstore = mstore.clone();
            let broker = broker.clone();
            Box::pin(async move {
                #[derive(serde::Deserialize, Default)]
                struct Req {
                    #[serde(default)]
                    zip_base64: Option<String>,
                    #[serde(default)]
                    files: Option<Vec<FileEntry>>,
                }
                let req: Req = serde_json::from_value(data)
                    .map_err(|e| format!("bundle.import: {e}"))?;

                let (files, mut warnings): (Vec<crate::backend::bundle_import::BundleImportFile>, Vec<String>) =
                    match (req.zip_base64, req.files) {
                        (Some(_), Some(_)) => {
                            return Err(
                                "bundle.import: exactly one of zip_base64/files must be present, got both"
                                    .to_string(),
                            );
                        }
                        (None, None) => {
                            return Err(
                                "bundle.import: exactly one of zip_base64/files must be present, got neither"
                                    .to_string(),
                            );
                        }
                        (Some(b64), None) => {
                            use base64::Engine as _;
                            let zip_bytes = base64::engine::general_purpose::STANDARD
                                .decode(&b64)
                                .map_err(|e| format!("bundle.import: invalid zip_base64: {e}"))?;
                            crate::backend::bundle_import::unzip_bundle_import(&zip_bytes)
                                .map_err(|e| format!("bundle.import: {e}"))?
                        }
                        (None, Some(files)) => {
                            // reagent P1, PR #2379 round 5: the raw `files`
                            // list previously skipped straight to
                            // parse_bundle_import with zero size/count
                            // enforcement, even though the spec treats it as
                            // an equally untrusted alternate ingestion path
                            // to zip_base64 — bypassing every zip-bomb/DoS
                            // defense the zip path enforces.
                            let raw_files: Vec<crate::backend::bundle_import::BundleImportFile> = files
                                .into_iter()
                                .map(|f| crate::backend::bundle_import::BundleImportFile {
                                    path: f.path,
                                    content: f.content,
                                })
                                .collect();
                            crate::backend::bundle_import::enforce_raw_files_caps(raw_files)
                                .map_err(|e| format!("bundle.import: {e}"))?
                        }
                    };

                let parsed = crate::backend::bundle_import::parse_bundle_import(&files)
                    .map_err(|e| format!("bundle.import: {e}"))?;
                warnings.extend(parsed.warnings.clone());

                // Account requirement resolution (§4.5) — read-only lookup
                // by provider, purely informational. Never creates an
                // account, never writes anything derived from a match.
                // Phase 3 spec §3.1: extracted into resolve_account_requirements,
                // shared with the new preview/commit RPC handlers.
                // Today's existing route keeps its response shape exactly
                // as before -- unbounded, matching its own already-shipped
                // behavior. The bounded-display projection below is a
                // Phase 3 preview/commit-only requirement.
                let resolved = resolve_account_requirements(&id_store, &parsed.requirements)
                    .map_err(|e| format!("bundle.import: {e}"))?;
                let resolved_requirement_ids: Vec<String> =
                    resolved.iter().filter(|r| r.resolved()).map(|r| r.id.clone()).collect();
                let unresolved_requirements: Vec<serde_json::Value> = resolved
                    .iter()
                    .filter(|r| !r.resolved())
                    .map(|r| json!({
                        "id": r.id,
                        "provider": r.provider,
                        "env": r.env,
                        "match_count": r.match_count,
                    }))
                    .collect();

                // Skills — global, not bound to any agent (mirrors
                // skill.catalog.upsert's own is_global: true convention for
                // a shared-resource creation path). A genuine name
                // conflict (the ONLY intentional business error
                // `skill_upsert_unique_global` returns —
                // `StoreError::Other` containing "already exists") is a
                // per-skill warning, not an aborted import. Codex P2, PR
                // #2379: any OTHER error (locked/corrupt store, I/O
                // failure) previously hit this same arm and was silently
                // treated as a skip too — turning an infrastructure
                // failure into an apparently-successful lossy import.
                // Only the confirmed-conflict shape is swallowed; anything
                // else aborts the whole RPC.
                // codex P2, PR #2379 round 4: rollback previously discarded
                // every skill_delete error via `let _ = ...`, so if the SAME
                // failing store also rejects the cleanup deletes, previously-
                // inserted global skills could silently survive an RPC that
                // reports failure. No transaction primitive exists on Store
                // today, so full atomicity is out of scope here; surfacing
                // the rollback failure instead of swallowing it is the fix
                // that fits — the caller at least learns cleanup itself may
                // not have fully succeeded.
                let rollback_skills = |ids: &[String]| -> Vec<String> {
                    ids.iter()
                        .filter_map(|id| mstore.skill_delete(&identity_store, id).err().map(|e| format!("{id}: {e}")))
                        .collect()
                };

                let now = now_ms();
                let mut imported_skill_ids: Vec<String> = Vec::new();
                let mut skipped_skills = parsed.skipped_skills.clone();
                for skill in &parsed.skills {
                    let row = crate::backend::storage::Skill {
                        id: uuid::Uuid::new_v4().to_string(),
                        name: skill.slug.clone(),
                        trigger: skill.slug.clone(),
                        skill_type: crate::backend::agent_config::SKILL_TYPE_AGENT_SKILL.to_string(),
                        description: skill.description.clone(),
                        content: skill.content.clone(),
                        is_global: true,
                        created_at: now,
                        updated_at: now,
                    };
                    match identity_store.skill_upsert_unique_global(&row) {
                        Ok(()) => imported_skill_ids.push(row.id),
                        Err(crate::backend::storage::error::StoreError::Other(msg))
                            if msg.contains("already exists") =>
                        {
                            warnings.push(format!("skill \"{}\": {msg}", skill.slug));
                            skipped_skills.push(skill.slug.clone());
                        }
                        Err(e) => {
                            // Codex P2, PR #2379: roll back every skill this
                            // loop already created before propagating —
                            // otherwise an infra failure partway through
                            // leaves orphaned global skill rows (bound to
                            // no bundle, but visible/bindable by any agent)
                            // behind a failed RPC.
                            let rollback_errors = rollback_skills(&imported_skill_ids);
                            let mut msg = format!(
                                "bundle.import: failed to create skill \"{}\": {e}",
                                skill.slug
                            );
                            if !rollback_errors.is_empty() {
                                msg.push_str(&format!(
                                    "; additionally, rollback of {} previously-created skill(s) failed and may have left orphaned global skill row(s): {}",
                                    rollback_errors.len(),
                                    rollback_errors.join("; ")
                                ));
                            }
                            return Err(msg);
                        }
                    }
                }

                let memory = Bundle {
                    id: uuid::Uuid::new_v4().to_string(),
                    name: parsed.name,
                    description: parsed.description,
                    is_blank: false,
                    // Never imported as global — an import must not
                    // silently start injecting into every agent's
                    // CLAUDE.md without explicit user action (spec §4.4).
                    is_global: false,
                    provider: parsed.provider,
                    model: parsed.model,
                    instructions: parsed.instructions,
                    // ABF v0.2 §2.2: every variant is stored verbatim, no
                    // merge decision made at import time.
                    instructions_by_provider: serde_json::to_string(&parsed.instructions_by_provider)
                        .unwrap_or_else(|_| "{}".to_string()),
                    // Phase 3 spec §3.0: ImportedContextFile gained a
                    // stable `id` selection key alongside `path`/`content`
                    // -- project it away before persisting, matching
                    // `Bundle.context_files`'s existing documented
                    // `[{path, content}]` shape.
                    context_files: serde_json::to_string(
                        &parsed.context_files.iter().map(|cf| json!({"path": cf.path, "content": cf.content})).collect::<Vec<_>>(),
                    )
                    .unwrap_or_else(|_| "[]".to_string()),
                    // Phase 3 spec §3.0, round 2: `parsed.mcp_servers` is now
                    // `Vec<ParsedMcpServer>{source_path, config}` (a stable
                    // selection key alongside the raw config) — every write
                    // site must project to `.config` before serializing, or
                    // this would persist the wrapper object instead of the
                    // raw MCP config every consumer of `Bundle.mcp_servers`
                    // expects.
                    mcp_servers: serde_json::to_string(
                        &parsed.mcp_servers.iter().map(|m| &m.config).collect::<Vec<_>>(),
                    )
                    .unwrap_or_else(|_| "[]".to_string()),
                    skills: serde_json::to_string(&imported_skill_ids)
                        .unwrap_or_else(|_| "[]".to_string()),
                    sort_order: 0,
                    created_at: now,
                    updated_at: now,
                    is_system: false,
                };
                if let Err(e) = id_store.bundle_upsert(&memory) {
                    // Codex P2, PR #2379: same rollback as above — the
                    // skills this RPC just created must not survive a
                    // failed bundle creation as orphaned global rows.
                    let rollback_errors = rollback_skills(&imported_skill_ids);
                    let mut msg = format!("bundle.import: {e}");
                    if !rollback_errors.is_empty() {
                        msg.push_str(&format!(
                            "; additionally, rollback of {} previously-created skill(s) failed and may have left orphaned global skill row(s): {}",
                            rollback_errors.len(),
                            rollback_errors.join("; ")
                        ));
                    }
                    return Err(msg);
                }

                // Ref tables are authoritative — bind after the bundle row
                // exists, or this import exports empty and its servers never
                // reach a spawned agent.
                warnings.extend(bind_imported_components(
                    &mstore,
                    &id_store,
                    &identity_store,
                    &memory.id,
                    &imported_skill_ids,
                    &parsed.mcp_servers.iter().map(|m| m.config.clone()).collect::<Vec<_>>(),
                ));

                broker.publish(crate::backend::mps::MuxEvent {
                    event: "memories:changed".to_string(),
                    scopes: vec![], sender: String::new(), persist: 0, data: None,
                });
                if !imported_skill_ids.is_empty() {
                    broker.publish(crate::backend::mps::MuxEvent {
                        event: "skills:changed".to_string(),
                        scopes: vec![], sender: String::new(), persist: 0, data: None,
                    });
                }

                Ok(Some(json!({
                    "bundle_id": memory.id,
                    "imported_skill_ids": imported_skill_ids,
                    "skipped_skills": skipped_skills,
                    "resolved_requirement_ids": resolved_requirement_ids,
                    "unresolved_requirements": unresolved_requirements,
                    "warnings": warnings,
                })))
            })
        }),
    );
}

/// On-disk size ceiling for `file_path` input (Phase 3 spec §3.0.5, rounds
/// 4/5) — generous headroom above `MAX_TOTAL_UNCOMPRESSED_BYTES`'s 50 MiB
/// for compression/container overhead and imperfectly-compressed content.
pub(super) const MAX_ABF_FILE_SIZE_BYTES: u64 = 100 * 1024 * 1024;

/// Reads a `file_path` input server-side (Phase 3 spec §3.0.5). Opens the
/// path with a no-follow mechanism so a symlink fails to open at all
/// rather than being silently resolved to its target (round 6) — one
/// handle, one continuous read, no second path resolution for anything to
/// race against (round 5's TOCTOU fix): metadata comes from that SAME open
/// handle and is checked against `MAX_ABF_FILE_SIZE_BYTES` BEFORE any read
/// (round 4); the actual read is still hard-bounded via `.take(...)`
/// regardless of what the metadata claimed.
pub(super) fn read_abf_file_path(path: &str) -> Result<Vec<u8>, String> {
    use std::io::Read;

    #[cfg(unix)]
    let mut file = {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
            .map_err(|e| format!("{path}: failed to open: {e}"))?
    };
    #[cfg(windows)]
    let mut file = {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_FLAG_OPEN_REPARSE_POINT (0x0020_0000): opens a symlink/
        // junction AS the reparse point itself, rather than transparently
        // following it to its target — the Windows equivalent of Unix's
        // O_NOFOLLOW, in a single atomic CreateFileW call (no separate
        // path resolution for a TOCTOU window to open in). The metadata
        // check immediately below then rejects the handle outright if it
        // turns out to be a reparse point. Windows symlinks require
        // elevated privileges to create by default, narrowing (not
        // eliminating) the practical risk this guards against.
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
            .map_err(|e| format!("{path}: failed to open: {e}"))?
    };
    #[cfg(not(any(unix, windows)))]
    let mut file = std::fs::File::open(path).map_err(|e| format!("{path}: failed to open: {e}"))?;

    let metadata = file.metadata().map_err(|e| format!("{path}: failed to stat: {e}"))?;
    #[cfg(windows)]
    if metadata.file_type().is_symlink() {
        return Err(format!("{path}: refusing to follow a symlink"));
    }
    if !metadata.is_file() {
        return Err(format!("{path}: not a regular file"));
    }
    if metadata.len() > MAX_ABF_FILE_SIZE_BYTES {
        return Err(format!(
            "{path}: size ({} bytes) exceeds the limit ({MAX_ABF_FILE_SIZE_BYTES} bytes)",
            metadata.len()
        ));
    }

    let mut buf: Vec<u8> = Vec::new();
    let mut limited = (&mut file).take(MAX_ABF_FILE_SIZE_BYTES + 1);
    limited.read_to_end(&mut buf).map_err(|e| format!("{path}: failed to read: {e}"))?;
    if buf.len() as u64 > MAX_ABF_FILE_SIZE_BYTES {
        return Err(format!("{path}: exceeds the size limit while reading"));
    }
    Ok(buf)
}

/// Bounded warning budget shared by `bundle.import.preview`/`.commit`
/// (Phase 3 spec §3.1, round 11) — unlike the existing `bundle.import`
/// route's effectively-unbounded budget, preserved unchanged above.
pub(super) fn preview_commit_warning_budget() -> crate::backend::bundle_import::WarningBudget {
    crate::backend::bundle_import::WarningBudget::bounded(200, 300)
}

/// Final combined-list bound applied to a preview/commit response's
/// `warnings` (Phase 3 spec §3.1 round 8 / round 9's commit-response
/// generalization) — cheap defense-in-depth over an already
/// per-source-bounded list (each of `resolve_import_input`'s intake
/// warnings and `parse_bundle_import_with_budget`'s own warnings is
/// already individually bounded; concatenating two independently-bounded
/// lists can still exceed either bound alone, so one more pass caps the
/// combined total before it's ever serialized).
/// Per-entry content cap for `project_instructions` in an import preview.
///
/// Deliberately far below `MAX_INSTRUCTIONS_PREVIEW_CHARS` (50k): that cap
/// applies to ONE field, whereas a preview can carry up to
/// `MAX_IMPORTED_PROJECT_INSTRUCTIONS` (200) of these, so the same number
/// would put a 10 MB ceiling on a single response. 4k is enough to recognize a
/// file and see how it opens; the untruncated bytes are in the archive itself.
pub(super) const MAX_PROJECT_INSTRUCTION_PREVIEW_CHARS: usize = 4_000;

pub(super) fn bound_warnings_for_response(warnings: Vec<String>) -> (Vec<String>, bool) {
    const MAX_COMBINED_WARNINGS: usize = 200;
    if warnings.len() <= MAX_COMBINED_WARNINGS {
        (warnings, false)
    } else {
        let mut kept: Vec<String> = warnings.into_iter().take(MAX_COMBINED_WARNINGS).collect();
        kept.push("... additional warnings not shown".to_string());
        (kept, true)
    }
}

/// The outcome of resolving one of `file_path`/`zip_base64`/`files` into
/// intake-ready files plus the Phase 3 content digest (§3.0.5) — shared by
/// `bundle.import.preview` and `.commit` so both compute the digest
/// identically. The mode tag baked into `content_digest` (round 7) means a
/// plain digest-equality check at commit is sufficient to enforce "same
/// input mode as preview" (round 6) — no separate mode field is needed.
#[derive(Debug)]
pub(super) struct ResolvedImportInput {
    pub(crate) files: Vec<crate::backend::bundle_import::BundleImportFile>,
    pub(crate) intake_warnings: Vec<String>,
    pub(crate) content_digest: String,
}

pub(super) fn resolve_import_input(
    file_path: Option<String>,
    zip_base64: Option<String>,
    files: Option<Vec<FileEntry>>,
    warning_budget: crate::backend::bundle_import::WarningBudget,
) -> Result<ResolvedImportInput, String> {
    use crate::backend::bundle_import as bi;
    let provided = [file_path.is_some(), zip_base64.is_some(), files.is_some()]
        .iter()
        .filter(|p| **p)
        .count();
    if provided != 1 {
        return Err(format!(
            "exactly one of file_path/zip_base64/files must be present, got {provided}"
        ));
    }
    if let Some(path) = file_path {
        let raw = read_abf_file_path(&path)?;
        let content_digest = bi::content_digest_raw_bytes(bi::ImportInputMode::FilePath, &raw);
        let (files, intake_warnings) = bi::unzip_bundle_import_with_budget(&raw, warning_budget)?;
        return Ok(ResolvedImportInput { files, intake_warnings, content_digest });
    }
    if let Some(b64) = zip_base64 {
        use base64::Engine as _;
        let raw = base64::engine::general_purpose::STANDARD
            .decode(&b64)
            .map_err(|e| format!("invalid zip_base64: {e}"))?;
        let content_digest = bi::content_digest_raw_bytes(bi::ImportInputMode::ZipBase64, &raw);
        let (files, intake_warnings) = bi::unzip_bundle_import_with_budget(&raw, warning_budget)?;
        return Ok(ResolvedImportInput { files, intake_warnings, content_digest });
    }
    let files = files.expect("exactly one input already validated present");
    let raw_files: Vec<bi::BundleImportFile> = files
        .into_iter()
        .map(|f| bi::BundleImportFile { path: f.path, content: f.content })
        .collect();
    let (capped_files, intake_warnings) = bi::enforce_raw_files_caps_with_budget(raw_files, warning_budget)?;
    let content_digest = bi::content_digest_files(&capped_files);
    Ok(ResolvedImportInput { files: capped_files, intake_warnings, content_digest })
}

/// `bundle.import.preview` — Phase 3 of
/// docs/specs/SPEC_ABF_IMPORT_UI_PHASE3_2026_08_02.md §3.1. Pure parse plus
/// read-only collision/name-collision lookups — zero Store writes. Window-
/// scoped like the rest of `bundle.*`. Extracted from the RPC closure into
/// a directly-callable, directly-testable function (the closure itself
/// only deserializes the request and forwards).
pub(super) async fn bundle_import_preview_impl(
    id_store: &crate::backend::storage::store::Store,
    identity_store: &crate::backend::storage::store::Store,
    mstore: &crate::backend::storage::store::Store,
    req: PreviewReq,
) -> Result<crate::backend::rpc_types::BundleImportPreviewResponse, String> {
    use crate::backend::bundle_import as bi;
    use crate::backend::rpc_types::{
        BundleImportContextFilePreview, BundleImportMcpServerDisplay,
        BundleImportMcpServerPreview, BundleImportPreviewResponse,
        BundleImportProjectInstructionPreview, BundleImportRequirementPreview,
        BundleImportSkillPreview,
    };

    let resolved = resolve_import_input(req.file_path, req.zip_base64, req.files, preview_commit_warning_budget())
        .map_err(|e| format!("bundle.import.preview: {e}"))?;

    let parsed = bi::parse_bundle_import_with_budget(&resolved.files, preview_commit_warning_budget())
        .map_err(|e| format!("bundle.import.preview: {e}"))?;

    let mut all_warnings = resolved.intake_warnings;
    all_warnings.extend(parsed.warnings);

    // Skill collision detection (§3.1, two-pass).
    let global_slugs: std::collections::HashSet<String> = mstore
        .skill_list_global(identity_store)
        .map_err(|e| format!("bundle.import.preview: {e}"))?
        .into_iter()
        .map(|item| item.skill.name)
        .collect();
    let in_bundle_dupes = bi::duplicate_in_bundle_slugs(&parsed.skills);

    let skills_json: Vec<BundleImportSkillPreview> = parsed
        .skills
        .iter()
        .map(|skill| {
            let collision = bi::classify_skill_collision(&skill.slug, &global_slugs, &in_bundle_dupes);
            BundleImportSkillPreview {
                source_dir: skill.source_dir.clone(),
                slug: bounded_display(&skill.slug),
                description: bounded_display(&skill.description),
                collision: collision.to_string(),
            }
        })
        .collect();

    let mcp_servers_json: Vec<BundleImportMcpServerPreview> = parsed
        .mcp_servers
        .iter()
        .map(|m| {
            // `mcp_server_display` still returns a Value (it is also used
            // elsewhere and has its own tests asserting it never leaks the full
            // config); read its two known keys back into the typed shape rather
            // than changing that function's contract here.
            let display = bi::mcp_server_display(&m.config);
            BundleImportMcpServerPreview {
                source_path: m.source_path.clone(),
                display: BundleImportMcpServerDisplay {
                    name: display.get("name").and_then(|v| v.as_str()).map(str::to_string),
                    command: display.get("command").and_then(|v| v.as_str()).map(str::to_string),
                },
            }
        })
        .collect();

    let (instructions_preview, instructions_truncated, instructions_total_chars) =
        bi::bounded_instructions_preview(&parsed.instructions);

    let context_files_json: Vec<BundleImportContextFilePreview> = parsed
        .context_files
        .iter()
        .map(|cf| BundleImportContextFilePreview {
            id: cf.id,
            display_path: bounded_display(&cf.path),
            size_bytes: cf.content.len(),
        })
        .collect();

    let resolved_requirements = resolve_account_requirements(id_store, &parsed.requirements)
        .map_err(|e| format!("bundle.import.preview: {e}"))?;
    let requirements_json: Vec<BundleImportRequirementPreview> = resolved_requirements
        .iter()
        .map(|r| BundleImportRequirementPreview {
            id: bounded_display(&r.id),
            provider: bounded_display(&r.provider),
            env: bounded_display(&r.env),
            resolved: r.resolved(),
            match_count: r.match_count,
        })
        .collect();

    // Bundle name collision -- soft, informational (§2: bundle_upsert
    // has no name uniqueness constraint, so this never blocks).
    let existing_names: std::collections::HashSet<String> = id_store
        .bundle_list()
        .map_err(|e| format!("bundle.import.preview: {e}"))?
        .into_iter()
        .map(|b| b.name)
        .collect();
    let name_collision = existing_names.contains(&parsed.name);

    // Phase 3, Codex P1 on PR #3163. Read-only: this is what the SOURCE agent
    // was reading, and `bundle.import.commit` has no counterpart field — the
    // entries exist to be looked at, never to be written into the importing
    // repository (spec §5.2). Each content is bounded the same way
    // `instructions_preview` is, since a snapshot can be an arbitrarily large
    // repository file and the response carries up to
    // MAX_IMPORTED_PROJECT_INSTRUCTIONS of them.
    let project_instructions_json: Vec<BundleImportProjectInstructionPreview> = parsed
        .project_instructions
        .iter()
        .map(|pi| {
            let total_chars = pi.content.chars().count();
            let truncated = total_chars > MAX_PROJECT_INSTRUCTION_PREVIEW_CHARS;
            let preview: String = if truncated {
                pi.content.chars().take(MAX_PROJECT_INSTRUCTION_PREVIEW_CHARS).collect()
            } else {
                pi.content.clone()
            };
            BundleImportProjectInstructionPreview {
                path: bounded_display(&pi.path),
                file: bounded_display(&pi.file),
                content_hash: bounded_display(&pi.content_hash),
                owner: bounded_display(&pi.owner),
                content_preview: preview,
                content_truncated: truncated,
                content_total_chars: total_chars,
            }
        })
        .collect();

    let (warnings, warnings_truncated) = bound_warnings_for_response(all_warnings);

    Ok(BundleImportPreviewResponse {
        project_instructions: project_instructions_json,
        name: parsed.name,
        description: bounded_display(&parsed.description),
        instructions_preview,
        instructions_truncated,
        instructions_total_chars,
        context_files: context_files_json,
        skills: skills_json,
        mcp_servers: mcp_servers_json,
        requirements: requirements_json,
        warnings,
        warnings_truncated,
        name_collision,
        content_digest: resolved.content_digest,
    })
}

pub(super) fn register_bundle_import_preview(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let id_store = state.id_store.clone();
    let identity_store = state.identity_store.clone();
    let mstore = state.mstore.clone();
    engine.register_typed(
        COMMAND_BUNDLE_IMPORT_PREVIEW,
        move |req: PreviewReq, _ctx| {
            let id_store = id_store.clone();
            let identity_store = identity_store.clone();
            let mstore = mstore.clone();
            async move {
                bundle_import_preview_impl(&id_store, &identity_store, &mstore, req).await
            }
        },
    );
}




/// `bundle.import.commit` — Phase 3 §3.2. Re-resolves and re-parses the
/// same input fresh (never trusts client-supplied preview data for
/// anything written), rejects on a content-digest mismatch, then writes
/// only the selected items. Shares `bundle.import`'s existing skill-write/
/// rollback logic; the differences are selection filtering, the
/// `bundle_name`/`import_as` overrides, and the digest gate. Extracted
/// from the RPC closure into a directly-callable, directly-testable
/// function (the closure itself only deserializes the request and
/// forwards).
pub(super) async fn bundle_import_commit_impl(
    id_store: &crate::backend::storage::store::Store,
    identity_store: &crate::backend::storage::store::Store,
    mstore: &crate::backend::storage::store::Store,
    broker: &crate::backend::mps::Broker,
    req: CommitReq,
) -> Result<crate::backend::rpc_types::BundleImportCommitResponse, String> {
    use crate::backend::bundle_import as bi;
    use crate::backend::rpc_types::{BundleImportCommitResponse, BundleImportUnresolvedRequirement};

                let resolved = resolve_import_input(req.file_path, req.zip_base64, req.files, preview_commit_warning_budget())
                    .map_err(|e| format!("bundle.import.commit: {e}"))?;

                // §3.0.5: reject BEFORE doing anything else on a mismatch --
                // never a partial import against whatever content was
                // actually given. The mode tag baked into content_digest
                // (round 7) means this single comparison also enforces
                // "same input mode as preview" (round 6) with no separate
                // mode field needed.
                if resolved.content_digest != req.expected_content_digest {
                    return Err(
                        "bundle.import.commit: content changed since preview (digest mismatch) — re-select and preview again"
                            .to_string(),
                    );
                }

                let parsed = bi::parse_bundle_import_with_budget(&resolved.files, preview_commit_warning_budget())
                    .map_err(|e| format!("bundle.import.commit: {e}"))?;

                let mut warnings = resolved.intake_warnings;
                warnings.extend(parsed.warnings.clone());

                // codex P2, PR #2381 round 11: bundle_name must actually be
                // substituted for parsed.name, never silently ignored.
                // reagentx P2, PR #2382 round 3: unlike parsed.name (bounded
                // at parse time), a client-supplied override had no length
                // cap at all -- bound_bundle_name closes that gap the same
                // way for both paths (a no-op when bundle_name is already
                // parsed.name, since that's already within the cap).
                let bundle_name = bi::bound_bundle_name(&req.bundle_name.unwrap_or_else(|| parsed.name.clone()));

                let instructions = if req.include_instructions { parsed.instructions.clone() } else { String::new() };
                // ABF v0.2 §2.2: provider-scoped variants are part of the
                // same "instructions" component, gated by the same
                // include_instructions flag — no separate per-variant
                // selection exists in the Phase 3 preview/commit UI.
                let instructions_by_provider = if req.include_instructions {
                    parsed.instructions_by_provider.clone()
                } else {
                    std::collections::HashMap::new()
                };

                // include_context_files selects by the stable `id` (round
                // 13), never by (truncatable) display_path.
                let selected_context_files: Vec<serde_json::Value> = parsed
                    .context_files
                    .iter()
                    .filter(|cf| req.include_context_files.contains(&cf.id))
                    .map(|cf| json!({ "path": cf.path, "content": cf.content }))
                    .collect();

                // include_mcp_servers selects by source_path (§3.0), never
                // by a JSON "name" field. The write path projects to
                // .config, matching the §3.0 amendment.
                let include_mcp: std::collections::HashSet<&str> =
                    req.include_mcp_servers.iter().map(|s| s.as_str()).collect();
                let selected_mcp_servers: Vec<&serde_json::Value> = parsed
                    .mcp_servers
                    .iter()
                    .filter(|m| include_mcp.contains(m.source_path.as_str()))
                    .map(|m| &m.config)
                    .collect();

                let resolved_reqs = resolve_account_requirements(id_store, &parsed.requirements)
                    .map_err(|e| format!("bundle.import.commit: {e}"))?;
                // codex P2, PR #2381 round 12: commit's response reuses the
                // exact same bounded-display projection preview's
                // equivalent fields use, via the shared bounded_display fn.
                let resolved_requirement_ids: Vec<String> =
                    resolved_reqs.iter().filter(|r| r.resolved()).map(|r| bounded_display(&r.id)).collect();
                let unresolved_requirements: Vec<BundleImportUnresolvedRequirement> = resolved_reqs
                    .iter()
                    .filter(|r| !r.resolved())
                    .map(|r| BundleImportUnresolvedRequirement {
                        id: bounded_display(&r.id),
                        provider: bounded_display(&r.provider),
                        env: bounded_display(&r.env),
                        match_count: r.match_count,
                    })
                    .collect();

                // Two-pass skill collision, recomputed server-side --
                // §4.1 point 4's "empty rename on a colliding skill = skip"
                // rule is enforced authoritatively here, not left to a
                // possibly-buggy/malicious client to have honored: without
                // this, a "duplicate_in_bundle" collision (not yet in the
                // global catalog) would let the FIRST of two same-slug
                // skills write successfully under its own slug even with
                // an empty rename, since skill_upsert_unique_global alone
                // wouldn't reject it until the second attempt.
                let global_slugs: std::collections::HashSet<String> = mstore
                    .skill_list_global(identity_store)
                    .map_err(|e| format!("bundle.import.commit: {e}"))?
                    .into_iter()
                    .map(|item| item.skill.name)
                    .collect();
                let in_bundle_dupes = bi::duplicate_in_bundle_slugs(&parsed.skills);

                let rollback_skills = |ids: &[String]| -> Vec<String> {
                    ids.iter()
                        .filter_map(|id| mstore.skill_delete(identity_store, id).err().map(|e| format!("{id}: {e}")))
                        .collect()
                };

                let now = now_ms();
                let mut imported_skill_ids: Vec<String> = Vec::new();
                let mut skipped_skills: Vec<String> = Vec::new();

                // reagentx P1, PR #2382 round 3: req.include_skills is
                // client-supplied with no length cap and no dedup by
                // source_dir -- an unbounded/repeated array could otherwise
                // drive an unbounded number of skill_upsert_unique_global
                // Store writes regardless of how many skills the archive
                // itself actually parsed to (parsed.skills is already
                // bounded by MAX_IMPORTED_SKILLS). First occurrence wins per
                // source_dir (matches this module's existing duplicate-
                // reference convention elsewhere), then capped at the same
                // MAX_IMPORTED_SKILLS the parser itself enforces.
                let mut seen_source_dirs: std::collections::HashSet<&str> = std::collections::HashSet::new();
                let deduped_skill_selections: Vec<&SkillSelection> = req
                    .include_skills
                    .iter()
                    .filter(|s| seen_source_dirs.insert(s.source_dir.as_str()))
                    .take(bi::MAX_IMPORTED_SKILLS)
                    .collect();

                for selection in deduped_skill_selections {
                    let Some(skill) = parsed.skills.iter().find(|s| s.source_dir == selection.source_dir) else {
                        continue; // source_dir not present in this parse -- nothing to import
                    };
                    let collision = bi::classify_skill_collision(&skill.slug, &global_slugs, &in_bundle_dupes);
                    let non_empty_rename = selection.import_as.as_deref().filter(|s| !s.is_empty());

                    if collision != "none" && non_empty_rename.is_none() {
                        // §4.1 point 4: never silently sent through with
                        // its original, known-conflicting slug.
                        skipped_skills.push(bounded_display(&skill.slug));
                        continue;
                    }
                    let effective_slug = non_empty_rename.map(|s| s.to_string()).unwrap_or_else(|| skill.slug.clone());

                    let row = crate::backend::storage::Skill {
                        id: uuid::Uuid::new_v4().to_string(),
                        name: effective_slug.clone(),
                        trigger: effective_slug.clone(),
                        skill_type: crate::backend::agent_config::SKILL_TYPE_AGENT_SKILL.to_string(),
                        description: skill.description.clone(),
                        content: skill.content.clone(),
                        is_global: true,
                        created_at: now,
                        updated_at: now,
                    };
                    match identity_store.skill_upsert_unique_global(&row) {
                        Ok(()) => imported_skill_ids.push(row.id),
                        Err(crate::backend::storage::error::StoreError::Other(msg))
                            if msg.contains("already exists") =>
                        {
                            // codex P2, PR #2382 round 2: effective_slug can be
                            // the caller-supplied import_as, which has no
                            // length bound anywhere before this point (unlike
                            // a parsed skill.slug, implicitly bounded by the
                            // per-entry decompression cap) -- and msg's own
                            // text embeds the identical unbounded value a
                            // second time (StoreError::Other's own format!
                            // interpolates skill.name). Use a fixed message
                            // instead of msg's text (its only informative
                            // content, "already exists", is already implied by
                            // the branch guard) and the shared bounded_display
                            // projection every other warning in this handler
                            // already uses, closing both unbounded paths at
                            // once rather than truncating msg's text in place.
                            warnings.push(format!("skill \"{}\": already exists", bounded_display(&effective_slug)));
                            skipped_skills.push(bounded_display(&effective_slug));
                        }
                        Err(e) => {
                            let rollback_errors = rollback_skills(&imported_skill_ids);
                            let mut msg = format!(
                                "bundle.import.commit: failed to create skill \"{effective_slug}\": {e}"
                            );
                            if !rollback_errors.is_empty() {
                                msg.push_str(&format!(
                                    "; additionally, rollback of {} previously-created skill(s) failed and may have left orphaned global skill row(s): {}",
                                    rollback_errors.len(),
                                    rollback_errors.join("; ")
                                ));
                            }
                            return Err(msg);
                        }
                    }
                }

                let memory = Bundle {
                    id: uuid::Uuid::new_v4().to_string(),
                    name: bundle_name,
                    description: parsed.description,
                    is_blank: false,
                    is_global: false,
                    // Like `description` above (and unlike instructions/
                    // context/mcp), provider/model are structural identity
                    // fields, not opt-in components — there's no
                    // include_provider toggle in the preview/commit UI, so
                    // these always carry straight through when present.
                    provider: parsed.provider,
                    model: parsed.model,
                    instructions,
                    instructions_by_provider: serde_json::to_string(&instructions_by_provider)
                        .unwrap_or_else(|_| "{}".to_string()),
                    context_files: serde_json::to_string(&selected_context_files)
                        .unwrap_or_else(|_| "[]".to_string()),
                    mcp_servers: serde_json::to_string(&selected_mcp_servers)
                        .unwrap_or_else(|_| "[]".to_string()),
                    skills: serde_json::to_string(&imported_skill_ids)
                        .unwrap_or_else(|_| "[]".to_string()),
                    sort_order: 0,
                    created_at: now,
                    updated_at: now,
                    is_system: false,
                };
                if let Err(e) = id_store.bundle_upsert(&memory) {
                    let rollback_errors = rollback_skills(&imported_skill_ids);
                    let mut msg = format!("bundle.import.commit: {e}");
                    if !rollback_errors.is_empty() {
                        msg.push_str(&format!(
                            "; additionally, rollback of {} previously-created skill(s) failed and may have left orphaned global skill row(s): {}",
                            rollback_errors.len(),
                            rollback_errors.join("; ")
                        ));
                    }
                    return Err(msg);
                }

                // Ref tables are authoritative — bind after the bundle row
                // exists. Only the servers the user actually selected.
                warnings.extend(bind_imported_components(
                    &mstore,
                    &id_store,
                    &identity_store,
                    &memory.id,
                    &imported_skill_ids,
                    &selected_mcp_servers.iter().map(|c| (*c).clone()).collect::<Vec<_>>(),
                ));

                broker.publish(crate::backend::mps::MuxEvent {
                    event: "memories:changed".to_string(),
                    scopes: vec![], sender: String::new(), persist: 0, data: None,
                });
                if !imported_skill_ids.is_empty() {
                    broker.publish(crate::backend::mps::MuxEvent {
                        event: "skills:changed".to_string(),
                        scopes: vec![], sender: String::new(), persist: 0, data: None,
                    });
                }

                // codex P1, PR #2381 round 9: bounded the same as preview's
                // response, not left unbounded like today's bundle.import.
                let (bounded_warnings, warnings_truncated) = bound_warnings_for_response(warnings);

    Ok(BundleImportCommitResponse {
        bundle_id: memory.id,
        imported_skill_ids,
        skipped_skills,
        resolved_requirement_ids,
        unresolved_requirements,
        warnings: bounded_warnings,
        warnings_truncated,
    })
}

pub(super) fn register_bundle_import_commit(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let id_store = state.id_store.clone();
    let identity_store = state.identity_store.clone();
    let mstore = state.mstore.clone();
    let broker = state.broker.clone();
    engine.register_typed(
        COMMAND_BUNDLE_IMPORT_COMMIT,
        move |req: CommitReq, _ctx| {
            let id_store = id_store.clone();
            let identity_store = identity_store.clone();
            let mstore = mstore.clone();
            let broker = broker.clone();
            async move {
                bundle_import_commit_impl(&id_store, &identity_store, &mstore, &broker, req).await
            }
        },
    );
}
