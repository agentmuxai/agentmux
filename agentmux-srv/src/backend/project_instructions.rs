// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What an agent will actually read as project instructions.
//!
//! Phase 3 of `SPEC_INSTRUCTION_AND_MEMORY_PORTABILITY_2026_09_09.md`. Until
//! now AgentMux could say what it *wrote* to an agent's working directory and
//! nothing about what the agent would *read* there. A repository's own
//! `CLAUDE.md` or `AGENTS.md` can be the majority of what an agent is told,
//! and §2.4 established that the only thing ever recorded about one was a
//! single boolean — no path, no hash, no content.
//!
//! **Read-only, permanently.** This module opens files and hashes them. It
//! never writes one, and nothing downstream of it may either: the ownership
//! protection in `SPEC_CLAUDE_MD_OWNERSHIP_PROTECTION_2026_08_22.md` exists
//! because a foreign instruction file is not ours, and a portability feature
//! must not become a back door into overwriting one (spec §5.2).
//!
//! The file set comes from the provider registry's `native_instruction_sources`
//! — every path a provider picks up on its own — rather than from
//! `startup_instructions_filename`, which names only the single file AgentMux
//! writes. For Copilot those differ by three files.

use std::path::{Path, PathBuf};

use serde::Serialize;

use super::agent_config::{is_agentmux_managed_instructions, AGENTMUX_MEMORY_FILENAME};
use super::providers;

/// Per-file read ceiling.
///
/// Matches native memory's own cap (`native_memory_handlers.rs:494`) rather
/// than the far larger ABF one: this content is surfaced in a UI and hashed on
/// every observation, and an instruction file that large is pathological
/// regardless. Oversized files are still *reported*, with `content` empty and
/// `truncated` set — the operator needs to know the file is there even when
/// showing it is impractical, which is the whole point of the feature.
pub const MAX_INSTRUCTION_FILE_BYTES: u64 = 10 * 1024 * 1024;

/// Who wrote an instruction file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum InstructionOwner {
    /// Carries one of AgentMux's managed markers — regenerated on launch.
    Agentmux,
    /// The repository's own. Read and tracked; never written.
    Foreign,
}

/// One resolved project-instruction file.
#[derive(Debug, Clone, Serialize)]
pub struct ProjectInstructionFile {
    /// Working-directory-relative, exactly as the registry declares it.
    pub path: String,
    /// False when the provider would look here and find nothing. Reported
    /// rather than filtered out: "the file a provider reads is absent" is
    /// itself the answer to what the agent will read.
    pub exists: bool,
    pub size_bytes: u64,
    /// SHA-256 of the file's bytes, or empty when absent/unreadable. The
    /// tracking half of Phase 3 compares this across observations to detect a
    /// foreign file changing under a running agent.
    pub content_hash: String,
    pub owner: InstructionOwner,
    /// Empty when absent, unreadable, or over [`MAX_INSTRUCTION_FILE_BYTES`].
    pub content: String,
    /// True when the file exists but was too large to read.
    pub truncated: bool,
    /// Set when the file exists but could not be read — a permissions problem,
    /// or bytes that are not UTF-8. Distinguished from absence on purpose:
    /// `write_claude_md_respecting_ownership` treats unreadable as foreign
    /// rather than as absent for the same reason (`agent_config.rs:1122`), and
    /// silently reporting "no instructions" for a file that exists would be
    /// the same class of lie this feature exists to remove.
    pub error: Option<String>,
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

/// Every path this provider reads, plus AgentMux's own side file.
///
/// `.claude/AGENTMUX_MEMORY.md` is included for providers whose write target
/// is `CLAUDE.md`, because that is the file AgentMux falls back to when the
/// repository owns `CLAUDE.md`
/// (`SPEC_CLAUDE_MD_OWNERSHIP_PROTECTION_2026_08_22.md`) — it is genuinely
/// read, via an `@`-import line, and an instruction set that omitted it would
/// be missing AgentMux's own contribution. It is not in the registry's
/// `native_instruction_sources` because the provider does not discover it
/// natively; the import line is ours.
fn paths_for_provider(provider_id: &str) -> Vec<String> {
    let Some(provider) = providers::get_provider(provider_id) else {
        return Vec::new();
    };
    let mut paths: Vec<String> = provider
        .native_instruction_sources
        .iter()
        .map(|s| s.to_string())
        .collect();
    if provider.startup_instructions_filename == Some("CLAUDE.md") {
        let side_file = AGENTMUX_MEMORY_FILENAME.to_string();
        if !paths.contains(&side_file) {
            paths.push(side_file);
        }
    }
    paths
}

/// Who owns this file — by marker, or by construction.
///
/// Most files are classified by their managed marker, which is the honest test
/// for a path AgentMux may or may not have written.
///
/// `.claude/AGENTMUX_MEMORY.md` is the exception, and it has to be: AgentMux
/// writes that file's content **raw**, with no marker
/// (`agent_config.rs:1207`), because it is not a file anyone else was ever
/// going to own — `agent_config.rs:990` calls it "100% AgentMux's own
/// content". Testing it by marker returns `foreign` every time, which is
/// backwards for the one field that decides whether a file is ours to touch
/// (ReAgent, PR #3156).
///
/// Classifying it by path is not a guess: AgentMux creates that exact path,
/// regenerates it on every launch, and the ownership protection exists
/// precisely because the *other* file at that location is not ours.
fn classify_owner(rel_path: &str, content: &str) -> InstructionOwner {
    if rel_path == AGENTMUX_MEMORY_FILENAME {
        return InstructionOwner::Agentmux;
    }
    if is_agentmux_managed_instructions(content) {
        InstructionOwner::Agentmux
    } else {
        InstructionOwner::Foreign
    }
}

/// Read one instruction file, whatever state it is in.
fn read_one(working_dir: &Path, rel_path: &str) -> ProjectInstructionFile {
    let full = working_dir.join(rel_path);
    let mut out = ProjectInstructionFile {
        path: rel_path.to_string(),
        exists: false,
        size_bytes: 0,
        content_hash: String::new(),
        owner: InstructionOwner::Foreign,
        content: String::new(),
        truncated: false,
        error: None,
    };

    let meta = match std::fs::metadata(&full) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return out,
        Err(e) => {
            out.exists = true;
            out.error = Some(format!("stat failed: {e}"));
            return out;
        }
    };
    if !meta.is_file() {
        // A directory where the provider expects a file: it exists and it is
        // not readable as instructions. Saying so beats reporting absence.
        out.exists = true;
        out.error = Some("not a regular file".to_string());
        return out;
    }

    out.exists = true;
    out.size_bytes = meta.len();
    if meta.len() > MAX_INSTRUCTION_FILE_BYTES {
        out.truncated = true;
        out.error = Some(format!(
            "file is {} bytes, over the {MAX_INSTRUCTION_FILE_BYTES}-byte read limit",
            meta.len()
        ));
        return out;
    }

    match std::fs::read(&full) {
        Ok(bytes) => {
            // Hash the bytes, not the string: the hash has to be stable for a
            // file that is not valid UTF-8, which is exactly the case where
            // `content` cannot be populated.
            out.content_hash = sha256_hex(&bytes);
            match String::from_utf8(bytes) {
                Ok(text) => {
                    out.owner = classify_owner(rel_path, &text);
                    out.content = text;
                }
                Err(_) => {
                    out.error = Some("not valid UTF-8".to_string());
                }
            }
        }
        Err(e) => out.error = Some(format!("read failed: {e}")),
    }
    out
}

/// Ceiling on files discovered by directory scan, per scan entry.
///
/// A scanned directory is repository-controlled and unbounded — nothing stops
/// a repo holding thousands of `*.instructions.md` files. The declared paths
/// above are a fixed handful, so only this side needs a limit. On overflow the
/// scan reports what it found plus an explicit marker entry, rather than
/// silently returning a prefix: a truncated list that looks complete is the
/// failure mode this module exists to remove.
pub const MAX_SCANNED_INSTRUCTION_FILES: usize = 200;

/// Collect qualifying files under one declared scan directory.
///
/// Sorted by path so the answer is stable between calls — directory iteration
/// order is not. Symlinked directories are not followed: a link out of the
/// working directory would silently widen what this reports beyond the
/// project, and `read_one` still reads a symlinked *file* normally.
fn scan_dir(
    working_dir: &Path,
    scan: &providers::InstructionDirScan,
    out: &mut Vec<String>,
) {
    fn walk(
        root: &Path,
        dir: &Path,
        suffix: &str,
        recursive: bool,
        found: &mut Vec<String>,
    ) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            // An unreadable or absent scan directory is the ordinary case —
            // most repositories have none. Nothing to report.
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                if recursive && !meta.is_symlink() {
                    walk(root, &path, suffix, recursive, found);
                }
                continue;
            }
            if !path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.ends_with(suffix)) {
                continue;
            }
            if let Ok(rel) = path.strip_prefix(root) {
                // Forward slashes so the reported path matches the
                // registry's own spelling on every platform.
                found.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }

    let mut found = Vec::new();
    walk(
        working_dir,
        &working_dir.join(scan.dir),
        scan.suffix,
        scan.recursive,
        &mut found,
    );
    found.sort();
    out.extend(found);
}

/// Everything `provider_id` will read as instructions from `working_directory`.
///
/// Declared paths first, in registry order, then anything found by directory
/// scan — Copilot's `.github/instructions/**/*.instructions.md` being the case
/// that needs the second half (Codex, PR #3154). Ordering is stable between
/// calls.
///
/// Returns an empty list for an unknown provider or a blank working directory
/// — there is no directory to look in, which is not an error.
pub fn resolve_project_instructions(
    provider_id: &str,
    working_directory: &str,
) -> Vec<ProjectInstructionFile> {
    if working_directory.trim().is_empty() {
        return Vec::new();
    }
    let root = PathBuf::from(working_directory);
    let mut files: Vec<ProjectInstructionFile> = paths_for_provider(provider_id)
        .into_iter()
        .map(|rel| read_one(&root, &rel))
        .collect();

    let Some(provider) = providers::get_provider(provider_id) else {
        return files;
    };
    for scan in provider.native_instruction_dirs {
        let mut scanned: Vec<String> = Vec::new();
        scan_dir(&root, scan, &mut scanned);

        let overflowed = scanned.len() > MAX_SCANNED_INSTRUCTION_FILES;
        let total = scanned.len();
        scanned.truncate(MAX_SCANNED_INSTRUCTION_FILES);

        for rel in scanned {
            // A declared path could also match a scan; report it once.
            if files.iter().any(|f| f.path == rel) {
                continue;
            }
            files.push(read_one(&root, &rel));
        }
        if overflowed {
            files.push(ProjectInstructionFile {
                path: format!("{}/…", scan.dir),
                exists: true,
                size_bytes: 0,
                content_hash: String::new(),
                owner: InstructionOwner::Foreign,
                content: String::new(),
                truncated: true,
                error: Some(format!(
                    "{total} files match {}/**/*{} — only the first {MAX_SCANNED_INSTRUCTION_FILES} are listed",
                    scan.dir, scan.suffix
                )),
            });
        }
    }
    files
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::agent_config::CLAUDE_MD_MANAGED_MARKER;

    fn write(dir: &Path, rel: &str, content: &str) {
        let full = dir.join(rel);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(full, content).unwrap();
    }

    #[test]
    fn copilot_resolves_all_four_of_its_sources_not_just_the_write_target() {
        // The case that motivated the whole field: resolving only
        // `startup_instructions_filename` would report one file out of four.
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "AGENTS.md", "agents");
        write(dir.path(), ".github/copilot-instructions.md", "copilot");
        write(dir.path(), "CLAUDE.md", "claude");
        // GEMINI.md deliberately absent.

        let files = resolve_project_instructions("copilot", dir.path().to_str().unwrap());
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"AGENTS.md"), "{paths:?}");
        assert!(paths.contains(&".github/copilot-instructions.md"), "{paths:?}");
        assert!(paths.contains(&"CLAUDE.md"), "{paths:?}");
        assert!(paths.contains(&"GEMINI.md"), "{paths:?}");

        let gemini = files.iter().find(|f| f.path == "GEMINI.md").unwrap();
        assert!(!gemini.exists, "an absent source must be reported, not dropped");
        assert!(gemini.content_hash.is_empty());
    }

    #[test]
    fn a_repo_owned_file_reads_as_foreign_and_an_agentmux_one_does_not() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "CLAUDE.md", "# My project\n\nHouse rules.\n");
        let files = resolve_project_instructions("claude", dir.path().to_str().unwrap());
        let claude_md = files.iter().find(|f| f.path == "CLAUDE.md").unwrap();
        assert_eq!(claude_md.owner, InstructionOwner::Foreign);
        assert_eq!(claude_md.content, "# My project\n\nHouse rules.\n");
        assert!(!claude_md.content_hash.is_empty());

        write(
            dir.path(),
            "CLAUDE.md",
            &format!("{CLAUDE_MD_MANAGED_MARKER}\n\ngenerated\n"),
        );
        let files = resolve_project_instructions("claude", dir.path().to_str().unwrap());
        let claude_md = files.iter().find(|f| f.path == "CLAUDE.md").unwrap();
        assert_eq!(claude_md.owner, InstructionOwner::Agentmux);
    }

    #[test]
    fn claude_includes_the_agentmux_side_file_the_import_line_points_at() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), AGENTMUX_MEMORY_FILENAME, "side file");
        let files = resolve_project_instructions("claude", dir.path().to_str().unwrap());
        let side = files
            .iter()
            .find(|f| f.path == AGENTMUX_MEMORY_FILENAME)
            .expect("the @-imported side file is genuinely read");
        assert!(side.exists);
        assert_eq!(side.content, "side file");
        // AgentMux writes this file's content raw, with no marker
        // (`agent_config.rs:1207`), so a marker test would call the one file
        // that is unambiguously ours `foreign` (ReAgent, PR #3156).
        assert_eq!(
            side.owner,
            InstructionOwner::Agentmux,
            "the side file is AgentMux's by construction, marker or not"
        );
    }

    #[test]
    fn only_the_side_file_gets_ownership_by_path() {
        // The path rule must not leak into the marker rule: an unmarked
        // CLAUDE.md is the repository's, which is the entire premise of the
        // ownership protection.
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "CLAUDE.md", "no marker here");
        let files = resolve_project_instructions("claude", dir.path().to_str().unwrap());
        assert_eq!(
            files.iter().find(|f| f.path == "CLAUDE.md").unwrap().owner,
            InstructionOwner::Foreign
        );
    }

    #[test]
    fn a_provider_without_a_native_convention_resolves_to_nothing() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "CLAUDE.md", "not read by kimi");
        // kimi has no confirmed file convention at all.
        assert!(resolve_project_instructions("kimi", dir.path().to_str().unwrap()).is_empty());
    }

    #[test]
    fn a_blank_working_directory_or_unknown_provider_is_empty_not_an_error() {
        assert!(resolve_project_instructions("claude", "").is_empty());
        assert!(resolve_project_instructions("claude", "   ").is_empty());
        let dir = tempfile::tempdir().unwrap();
        assert!(resolve_project_instructions("nope", dir.path().to_str().unwrap()).is_empty());
    }

    #[test]
    fn the_hash_changes_when_the_file_does() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "CLAUDE.md", "before");
        let before = resolve_project_instructions("claude", dir.path().to_str().unwrap());
        let h1 = before.iter().find(|f| f.path == "CLAUDE.md").unwrap().content_hash.clone();

        write(dir.path(), "CLAUDE.md", "after");
        let after = resolve_project_instructions("claude", dir.path().to_str().unwrap());
        let h2 = &after.iter().find(|f| f.path == "CLAUDE.md").unwrap().content_hash;

        assert_ne!(&h1, h2, "a changed file must produce a different hash");
    }

    #[test]
    fn copilots_path_scoped_instructions_are_found_by_scan() {
        // Codex on #3154: these reach the agent, and resolving only the
        // declared literals would miss every one of them.
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), ".github/instructions/rust.instructions.md", "rust rules");
        write(dir.path(), ".github/instructions/deep/ts.instructions.md", "ts rules");
        // Not an instructions file — must not be picked up.
        write(dir.path(), ".github/instructions/README.md", "readme");

        let files = resolve_project_instructions("copilot", dir.path().to_str().unwrap());
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert!(
            paths.contains(&".github/instructions/rust.instructions.md"),
            "{paths:?}"
        );
        assert!(
            paths.contains(&".github/instructions/deep/ts.instructions.md"),
            "recursive, per the `**`: {paths:?}"
        );
        assert!(
            !paths.contains(&".github/instructions/README.md"),
            "only the declared suffix qualifies: {paths:?}"
        );

        let rust = files
            .iter()
            .find(|f| f.path.ends_with("rust.instructions.md"))
            .unwrap();
        assert_eq!(rust.content, "rust rules");
        assert!(!rust.content_hash.is_empty());
    }

    #[test]
    fn a_provider_with_no_scan_directory_ignores_one_that_exists() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), ".github/instructions/x.instructions.md", "not for claude");
        let files = resolve_project_instructions("claude", dir.path().to_str().unwrap());
        assert!(
            !files.iter().any(|f| f.path.contains(".github/instructions")),
            "claude declares no scan directory"
        );
    }

    #[test]
    fn an_oversized_scan_reports_the_overflow_instead_of_a_silent_prefix() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..(MAX_SCANNED_INSTRUCTION_FILES + 5) {
            write(
                dir.path(),
                &format!(".github/instructions/f{i}.instructions.md"),
                "x",
            );
        }
        let files = resolve_project_instructions("copilot", dir.path().to_str().unwrap());
        let scanned = files
            .iter()
            .filter(|f| f.path.starts_with(".github/instructions/") && !f.path.ends_with('…'))
            .count();
        assert_eq!(scanned, MAX_SCANNED_INSTRUCTION_FILES);
        let marker = files
            .iter()
            .find(|f| f.path.ends_with('…'))
            .expect("overflow must be reported, not silently truncated");
        assert!(marker.truncated);
        assert!(marker.error.as_deref().unwrap_or("").contains("only the first"));
    }

    #[test]
    fn scan_results_are_ordered_the_same_way_every_time() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["c", "a", "b"] {
            write(
                dir.path(),
                &format!(".github/instructions/{name}.instructions.md"),
                "x",
            );
        }
        let first = resolve_project_instructions("copilot", dir.path().to_str().unwrap());
        let second = resolve_project_instructions("copilot", dir.path().to_str().unwrap());
        let p1: Vec<&str> = first.iter().map(|f| f.path.as_str()).collect();
        let p2: Vec<&str> = second.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(p1, p2, "directory iteration order must not leak through");
    }

    #[test]
    fn a_directory_in_the_files_place_is_reported_not_silently_absent() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("CLAUDE.md")).unwrap();
        let files = resolve_project_instructions("claude", dir.path().to_str().unwrap());
        let claude_md = files.iter().find(|f| f.path == "CLAUDE.md").unwrap();
        assert!(claude_md.exists);
        assert!(claude_md.error.is_some(), "must not read as absent");
        assert!(claude_md.content.is_empty());
    }
}
