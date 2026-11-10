// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The scratch file service (`createscratchfile`, `movescratchfile`), moved out of `register_editor_handlers`
//! unchanged as named functions. Spec: docs/specs/SPEC_EDITOR_WIDGET_DEFAULT_UX_2026_06_14.md. Each takes the
//! user's home folder as a parameter, so a test can point it at a temp dir.

use std::path::{Path, PathBuf};

use super::{editor_may_change, in_a_distro_home, scratch_session_token};
use crate::backend::base::expand_home_dir_safe;
use crate::backend::rpc_types::{CreateScratchFileReq, CreateScratchFileResult, MoveScratchFileReq, MoveScratchFileResult};

const THIRTY_DAYS_MS: u64 = 30 * 24 * 60 * 60 * 1000;

/// A saved-nowhere scratch buffer that could be handed out again.
pub(super) struct Candidate {
    created_at: u64,
    pub(super) scratch_id: String,
    display_name: String,
}

fn scratch_dir_of(home: &Path) -> PathBuf {
    home.join(".agentmux").join("cache").join("scratch")
}

/// Creates a scratch buffer file in `~/.agentmux/cache/scratch/`, or hands out the most recent unclaimed one.
pub(super) fn create_scratch_file(home: &Path, cmd: CreateScratchFileReq) -> Result<CreateScratchFileResult, String> {
    let scratch_dir = scratch_dir_of(home);
    std::fs::create_dir_all(&scratch_dir).map_err(|e| format!("createscratchfile: create dir: {e}"))?;
    let now_ms = agentmux_common::time::now_ms_u64();
    let session_token = scratch_session_token();

    let candidates = scan_candidates(&scratch_dir, cmd.exclude_scratch_ids.as_deref(), now_ms);
    if let Some((scratch_id, display_name)) = claim_most_recent(&scratch_dir, candidates, session_token) {
        let file_path = scratch_dir.join(format!("{}.md", scratch_id));
        return Ok(CreateScratchFileResult { scratch_id, file_path: file_path.to_string_lossy().into_owned(), display_name });
    }
    mint_scratch(&scratch_dir, cmd.display_name, now_ms, session_token)
}

/// Phase 1: scan the `.md.meta` files, collecting candidates (not saved, not expired, not in
/// `exclude_scratch_ids`) and pruning expired pairs.
pub(super) fn scan_candidates(scratch_dir: &Path, exclude: Option<&[String]>, now_ms: u64) -> Vec<Candidate> {
    let mut candidates = Vec::new();
    let Ok(entries) = std::fs::read_dir(scratch_dir) else { return candidates };
    for entry in entries.flatten() {
        let meta_path = entry.path();
        let fname = meta_path.file_name().and_then(|f| f.to_str()).unwrap_or("").to_string();
        if !fname.ends_with(".md.meta") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&meta_path) else { continue };
        let Ok(meta_json) = serde_json::from_str::<serde_json::Value>(&content) else { continue };
        if meta_json.get("saved_to").map_or(false, |v| !v.is_null()) {
            continue;
        }
        let created_at = meta_json.get("created_at").and_then(|v| v.as_u64()).unwrap_or(0);
        let sid = meta_json.get("scratch_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
        if created_at == 0 || sid.is_empty() {
            continue;
        }
        if exclude.map_or(false, |ex| ex.iter().any(|e| e == &sid)) {
            continue;
        }
        if now_ms.saturating_sub(created_at) > THIRTY_DAYS_MS {
            // Prune the expired pair (all three files: .md, .md.meta, .md.claim).
            let _ = std::fs::remove_file(scratch_dir.join(format!("{}.md", sid)));
            let _ = std::fs::remove_file(&meta_path);
            let _ = std::fs::remove_file(scratch_dir.join(format!("{}.md.claim", sid)));
            continue;
        }
        let display_name = meta_json.get("display_name").and_then(|v| v.as_str()).unwrap_or("Untitled").to_string();
        candidates.push(Candidate { created_at, scratch_id: sid, display_name });
    }
    candidates
}

/// Phase 2: try to atomically claim the most recent candidate. Claim files stop two concurrent calls returning
/// the same backing file to different panes. Each claim file stores the per-process session token; a claim from
/// a previous process (crash/restart) has a different token and is cleared so recovery works.
pub(super) fn claim_most_recent(scratch_dir: &Path, mut candidates: Vec<Candidate>, session_token: &str) -> Option<(String, String)> {
    candidates.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    for Candidate { scratch_id: sid, display_name: dname, .. } in &candidates {
        let claim_path = scratch_dir.join(format!("{}.md.claim", sid));
        // Evict stale claims from a previous process session.
        if claim_path.exists() {
            if let Ok(tok) = std::fs::read_to_string(&claim_path) {
                if tok.trim() == session_token {
                    // A valid live claim: another concurrent call already owns this.
                    continue;
                }
            }
            // A stale claim (different session): remove it so this can be reclaimed.
            let _ = std::fs::remove_file(&claim_path);
        }
        // Try to exclusively create the claim file. If another concurrent call wins the race it will have
        // already created it, so this one is skipped.
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&claim_path) {
            Ok(mut f) => {
                use std::io::Write as _;
                let _ = f.write_all(session_token.as_bytes());
                if scratch_dir.join(format!("{}.md", sid)).exists() {
                    return Some((sid.clone(), dname.clone()));
                }
                // The backing file is gone (deleted externally): release the claim.
                let _ = std::fs::remove_file(&claim_path);
            }
            Err(_) => continue, // A concurrent call claimed this one first.
        }
    }
    None
}

/// No reusable candidate: a fresh UUID pair, claimed so subsequent calls don't immediately reuse it.
fn mint_scratch(
    scratch_dir: &Path,
    display_name: Option<String>,
    now_ms: u64,
    session_token: &str,
) -> Result<CreateScratchFileResult, String> {
    let scratch_id = uuid::Uuid::new_v4().to_string();
    let display_name = display_name.unwrap_or_else(|| "Untitled".to_string());
    let file_path = scratch_dir.join(format!("{}.md", scratch_id));
    std::fs::write(&file_path, "").map_err(|e| format!("createscratchfile: {e}"))?;
    let meta = serde_json::json!({
        "display_name": display_name,
        "scratch_id": scratch_id,
        "created_at": now_ms,
    });
    let meta_path = scratch_dir.join(format!("{}.md.meta", scratch_id));
    std::fs::write(&meta_path, meta.to_string()).map_err(|e| format!("createscratchfile: meta: {e}"))?;
    let claim_path = scratch_dir.join(format!("{}.md.claim", scratch_id));
    let _ = std::fs::write(&claim_path, session_token);
    Ok(CreateScratchFileResult { scratch_id, file_path: file_path.to_string_lossy().into_owned(), display_name })
}

/// Promotes a scratch buffer to a real user-chosen path (Save As).
pub(super) fn move_scratch_file(home: &Path, cmd: &MoveScratchFileReq) -> Result<MoveScratchFileResult, String> {
    let canonical_home = home.canonicalize().map_err(|e| format!("movescratchfile: home: {e}"))?;
    // The scratch_id has no path components.
    if cmd.scratch_id.contains('/') || cmd.scratch_id.contains('\\') || cmd.scratch_id.contains("..") {
        return Err("movescratchfile: invalid scratch_id".to_string());
    }
    let scratch_dir = scratch_dir_of(home);
    let scratch_path = scratch_dir.join(format!("{}.md", cmd.scratch_id));
    if !scratch_path.exists() {
        return Err("movescratchfile: scratch file not found".to_string());
    }
    let expanded_dest = expand_home_dir_safe(&cmd.destination_path);
    let dest_path = expanded_dest.as_path();

    // A coarse home-boundary check before any filesystem mutation, on the unexpanded path because the
    // destination may not exist yet (canonicalize fails on non-existent paths).
    if !dest_path.starts_with(home) && !in_a_distro_home(dest_path) {
        return Err("movescratchfile: destination outside home directory".to_string());
    }
    // Reject ".." components before create_dir_all: the coarse starts_with check is purely lexical and passes
    // paths like ~/foo/../../../tmp/evil because the prefix matches, but create_dir_all would then materialize
    // directories outside the home boundary before the canonical check fires.
    if dest_path.components().any(|c| c == std::path::Component::ParentDir) {
        return Err("movescratchfile: destination path must not contain '..' components".to_string());
    }
    // An existing destination is rejected to avoid silent data loss.
    if dest_path.exists() {
        return Err("movescratchfile: destination already exists".to_string());
    }
    if let Some(parent) = dest_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("movescratchfile: create dirs: {e}"))?;
    }
    // Canonicalize the parent (now guaranteed to exist) and append the filename.
    let canonical_dest = dest_path
        .parent()
        .and_then(|p| p.canonicalize().ok())
        .map(|p| p.join(dest_path.file_name().unwrap_or_default()))
        .ok_or_else(|| "movescratchfile: invalid destination path".to_string())?;
    // The fine-grained home-boundary check, with canonical paths.
    if !editor_may_change(&canonical_dest, &canonical_home) {
        return Err("movescratchfile: destination outside home directory".to_string());
    }
    // Copy then remove the scratch source (a cross-device-safe move).
    std::fs::copy(&scratch_path, &canonical_dest).map_err(|e| format!("movescratchfile: copy: {e}"))?;
    std::fs::remove_file(&scratch_path).map_err(|e| format!("movescratchfile: remove scratch: {e}"))?;
    // Clean up the .meta sidecar and the claim file.
    let _ = std::fs::remove_file(scratch_dir.join(format!("{}.md.meta", cmd.scratch_id)));
    let _ = std::fs::remove_file(scratch_dir.join(format!("{}.md.claim", cmd.scratch_id)));
    Ok(MoveScratchFileResult { file_path: canonical_dest.to_string_lossy().into_owned() })
}
