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
fn paths_for_provider(provider_id: &str, working_dir: &Path) -> Vec<String> {
    let Some(provider) = providers::get_provider(provider_id) else {
        return Vec::new();
    };
    let mut paths: Vec<String> = provider
        .native_instruction_sources
        .iter()
        .map(|s| s.to_string())
        .collect();

    // The side file is only read when a `CLAUDE.md` currently imports it.
    //
    // A `CLAUDE.md` write target is not enough on its own: when AgentMux owns
    // that file it writes the content directly and adds no import, and an
    // import appended on some earlier launch can be removed by hand while the
    // side file stays on disk. Including it unconditionally reports stale
    // content as instructions the agent is consuming, which is the same class
    // of confident-but-wrong answer this module exists to prevent (Codex, PR
    // #3156). Testing the actual `@` line is the only honest condition.
    if provider.startup_instructions_filename == Some("CLAUDE.md") {
        let import_needle = format!("@{AGENTMUX_MEMORY_FILENAME}");
        let imported = std::fs::read_to_string(working_dir.join("CLAUDE.md"))
            .map(|c| c.contains(&import_needle))
            .unwrap_or(false);
        let side_file = AGENTMUX_MEMORY_FILENAME.to_string();
        if imported && !paths.contains(&side_file) {
            paths.push(side_file);
        }
    }
    paths
}

/// The directory this agent will actually run in.
///
/// Must match `agent.open` exactly, or the answer describes a directory the
/// agent never uses (Codex, PR #3156): a blank `working_directory` means the
/// per-agent default, and a `~` path is expanded at launch. Reproducing both
/// here rather than reading the stored value raw is the difference between
/// reporting an agent's instructions and reporting nothing at all, since the
/// default-workdir case is the common one
/// (`SPEC_FIX_PERSONAL_MEMORY_EMPTY_WORKDIR_2026_09_01.md`).
pub fn effective_working_dir(working_directory: &str, agent_name: &str) -> String {
    let raw = if working_directory.trim().is_empty() {
        crate::backend::storage::agents::default_agent_working_dir(agent_name)
    } else {
        working_directory.to_string()
    };
    if raw.starts_with("~/") || raw == "~" {
        if let Ok(home) = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")) {
            return format!("{}/{}", home, raw.trim_start_matches("~/").trim_start_matches('~'));
        }
    }
    raw
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

/// Ceiling on the total content this resolver will return in one call.
///
/// The per-file cap and the file-count cap do not bound the payload between
/// them: 200 files just under the per-file limit is roughly 2 GiB, assembled
/// in memory before serialization (Codex, PR #3156). Past this budget a file
/// is still listed with its hash and metadata; only its content is dropped.
pub const MAX_TOTAL_INSTRUCTION_BYTES: u64 = 8 * 1024 * 1024;

/// Collect qualifying files under one declared scan directory.
///
/// Sorted by path so the answer is stable between calls — directory iteration
/// order is not. Symlinked directories are not followed: a link out of the
/// working directory would silently widen what this reports beyond the
/// project, and `read_one` still reads a symlinked *file* normally.
/// Ceiling on directories entered during one scan.
///
/// The file cap alone does not bound a deep or wide tree that contains few
/// matches — the walk still has to enter every directory to find that out.
const MAX_SCANNED_DIRECTORIES: usize = 500;

/// Collect qualifying files under one declared scan directory.
///
/// **Bounded during traversal, not after it.** Collecting everything and then
/// truncating leaves the advertised ceiling doing nothing about the cost it
/// exists to bound: a repository-controlled directory with a very large number
/// of matches still gets fully walked, allocated and sorted first (Codex, PR
/// #3156). The walk now stops as soon as it has one file more than the cap,
/// which is exactly enough to know the limit was hit.
///
/// Entries are sorted at each level before descending, so stopping early still
/// yields the same set every time. Without that, an early stop would return
/// whatever the filesystem happened to hand back first.
///
/// Symlinks are skipped outright: `file_type()` does not follow the link,
/// unlike `metadata()`, which resolves the target and reports
/// `is_symlink() == false` for every symlink — so an earlier guard written
/// against `metadata()` never fired at all. A scanned directory is
/// repository-controlled, and nothing about "follow this link" can be verified
/// from here; the declared literal paths are still read through links, which
/// is an ordinary layout.
///
/// Returns true when the cap was reached.
fn scan_dir(
    working_dir: &Path,
    scan: &providers::InstructionDirScan,
    out: &mut Vec<String>,
) -> ScanOutcome {
    fn walk(
        root: &Path,
        dir: &Path,
        suffix: &str,
        recursive: bool,
        found: &mut Vec<String>,
        dirs_visited: &mut usize,
    ) {
        if found.len() > MAX_SCANNED_INSTRUCTION_FILES || *dirs_visited >= MAX_SCANNED_DIRECTORIES {
            return;
        }
        *dirs_visited += 1;

        let Ok(entries) = std::fs::read_dir(dir) else {
            // An unreadable or absent scan directory is the ordinary case —
            // most repositories have none. Nothing to report.
            return;
        };
        // Deterministic order, so an early stop is reproducible.
        let mut entries: Vec<std::fs::DirEntry> = entries.flatten().collect();
        entries.sort_by_key(|e| e.file_name());

        for entry in entries {
            if found.len() > MAX_SCANNED_INSTRUCTION_FILES {
                return;
            }
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else { continue };
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                if recursive {
                    walk(root, &path, suffix, recursive, found, dirs_visited);
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

    let scan_root = working_dir.join(scan.dir);

    // **Check the scan ROOT, not just what is inside it.** Skipping symlinked
    // child entries does nothing if `.github/instructions` — or any ancestor
    // of it — is itself a link: `read_dir` follows it transparently, the
    // resulting paths still `strip_prefix` cleanly, and the outside file's
    // content goes out over an authenticated RPC (ReAgent P0, PR #3156).
    //
    // Canonicalizing both sides closes the whole class at once — a symlinked
    // final component, a symlinked ancestor, and `..` traversal — rather than
    // testing the one shape that was reported.
    let Ok(root_canon) = working_dir.canonicalize() else {
        return ScanOutcome { hit_cap: false, escaped: false };
    };
    let Ok(scan_canon) = scan_root.canonicalize() else {
        // Almost always "no such directory", which is the ordinary case.
        return ScanOutcome { hit_cap: false, escaped: false };
    };
    if !scan_canon.starts_with(&root_canon) {
        return ScanOutcome { hit_cap: false, escaped: true };
    }

    let mut found = Vec::new();
    let mut dirs_visited = 0usize;
    walk(
        working_dir,
        &scan_root,
        scan.suffix,
        scan.recursive,
        &mut found,
        &mut dirs_visited,
    );
    let hit_cap = found.len() > MAX_SCANNED_INSTRUCTION_FILES;
    found.truncate(MAX_SCANNED_INSTRUCTION_FILES);
    found.sort();
    out.extend(found);
    ScanOutcome { hit_cap, escaped: false }
}

/// What a scan ran into, beyond the files it found.
struct ScanOutcome {
    /// The file ceiling was reached and the walk stopped early.
    hit_cap: bool,
    /// The scan directory resolves outside the working directory, so it was
    /// not read at all. Reported rather than silently skipped: a repository
    /// that has done this is exactly the case an operator should see.
    escaped: bool,
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
    let mut files: Vec<ProjectInstructionFile> = paths_for_provider(provider_id, &root)
        .into_iter()
        .map(|rel| read_one(&root, &rel))
        .collect();

    let Some(provider) = providers::get_provider(provider_id) else {
        return files;
    };
    for scan in provider.native_instruction_dirs {
        let mut scanned: Vec<String> = Vec::new();
        // The walk stops at the cap rather than collecting everything first,
        // so it reports *that* the limit was hit, not how far past it the
        // repository goes — counting the rest would mean doing the traversal
        // the cap exists to avoid.
        let outcome = scan_dir(&root, scan, &mut scanned);

        for rel in scanned {
            // A declared path could also match a scan; report it once.
            if files.iter().any(|f| f.path == rel) {
                continue;
            }
            let mut file = read_one(&root, &rel);
            // Per-file and per-count ceilings do not bound the whole payload:
            // 200 files just under the per-file cap is roughly 2 GiB of
            // response, built in memory before serialization (Codex, PR
            // #3156). Past the aggregate budget a file is still LISTED, with
            // its hash and metadata — only its content is dropped. Reporting
            // that the file is there costs nothing; shipping its bytes is
            // what does.
            let used: u64 = files.iter().map(|f| f.content.len() as u64).sum();
            if used.saturating_add(file.content.len() as u64) > MAX_TOTAL_INSTRUCTION_BYTES {
                file.content = String::new();
                file.truncated = true;
                file.error = Some(format!(
                    "content omitted — the {MAX_TOTAL_INSTRUCTION_BYTES}-byte total budget for this response is exhausted"
                ));
            }
            files.push(file);
        }
        if outcome.escaped {
            files.push(ProjectInstructionFile {
                path: scan.dir.to_string(),
                exists: true,
                size_bytes: 0,
                content_hash: String::new(),
                owner: InstructionOwner::Foreign,
                content: String::new(),
                truncated: false,
                error: Some(format!(
                    "{} resolves outside the working directory — not read",
                    scan.dir
                )),
            });
        }
        if outcome.hit_cap {
            files.push(ProjectInstructionFile {
                path: format!("{}/…", scan.dir),
                exists: true,
                size_bytes: 0,
                content_hash: String::new(),
                owner: InstructionOwner::Foreign,
                content: String::new(),
                truncated: true,
                error: Some(format!(
                    "more than {MAX_SCANNED_INSTRUCTION_FILES} files match {}/**/*{} — only the first {MAX_SCANNED_INSTRUCTION_FILES} are listed",
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
        // The side file is read only when CLAUDE.md actually imports it —
        // see `the_side_file_is_only_reported_when_claude_md_actually_imports_it`.
        write(
            dir.path(),
            "CLAUDE.md",
            &format!("# Repo rules

@{AGENTMUX_MEMORY_FILENAME}
"),
        );
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

#[cfg(test)]
mod review_fix_tests {
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
    fn the_side_file_is_only_reported_when_claude_md_actually_imports_it() {
        // A CLAUDE.md write target does not by itself mean the side file is
        // read: when AgentMux owns CLAUDE.md it writes the content directly
        // and adds no import, and a previously appended import can be removed
        // by hand while the side file stays behind (Codex, PR #3156).
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), AGENTMUX_MEMORY_FILENAME, "stale side file");
        write(
            dir.path(),
            "CLAUDE.md",
            &format!("{CLAUDE_MD_MANAGED_MARKER}\n\nowned by agentmux, no import\n"),
        );

        let files = resolve_project_instructions("claude", dir.path().to_str().unwrap());
        assert!(
            !files.iter().any(|f| f.path == AGENTMUX_MEMORY_FILENAME),
            "no @-import line means the agent does not read it"
        );

        // Now the import exists, so it genuinely is read.
        write(
            dir.path(),
            "CLAUDE.md",
            &format!("# Repo rules\n\n@{AGENTMUX_MEMORY_FILENAME}\n"),
        );
        let files = resolve_project_instructions("claude", dir.path().to_str().unwrap());
        let side = files
            .iter()
            .find(|f| f.path == AGENTMUX_MEMORY_FILENAME)
            .expect("an active import means it is read");
        assert_eq!(side.owner, InstructionOwner::Agentmux);
    }

    #[test]
    fn a_symlinked_directory_inside_a_scan_is_not_followed() {
        // `metadata()` follows the link and reports is_symlink() == false, so
        // the original guard never fired at all — a repository could point a
        // link out of the working directory, or build a cycle (Codex, #3156).
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        write(outside.path(), "secret.instructions.md", "not part of this project");
        std::fs::create_dir_all(dir.path().join(".github/instructions")).unwrap();
        write(dir.path(), ".github/instructions/own.instructions.md", "ours");

        let link = dir.path().join(".github/instructions/elsewhere");
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(outside.path(), &link).is_ok();
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_dir(outside.path(), &link).is_ok();
        if !made {
            // Windows needs privilege for symlinks; the guard is still
            // exercised on any platform that allows creating one.
            return;
        }

        let files = resolve_project_instructions("copilot", dir.path().to_str().unwrap());
        assert!(
            files.iter().any(|f| f.path.ends_with("own.instructions.md")),
            "the real file must still be found"
        );
        assert!(
            !files.iter().any(|f| f.path.contains("secret")),
            "a symlink must not carry the scan outside the working directory"
        );
    }

    #[test]
    fn a_symlinked_scan_root_is_refused_and_reported() {
        // ReAgent P0 on #3156: guarding child entries does nothing if the scan
        // directory ITSELF is a link — `read_dir` follows it transparently,
        // the paths still strip_prefix cleanly, and the outside file's content
        // goes out over an authenticated RPC. The nested-symlink test could
        // never have caught this, because it only ever links a child.
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        write(outside.path(), "secret.instructions.md", "not part of this project");

        std::fs::create_dir_all(dir.path().join(".github")).unwrap();
        let link = dir.path().join(".github/instructions");
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(outside.path(), &link).is_ok();
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_dir(outside.path(), &link).is_ok();
        if !made {
            return; // Windows needs privilege to create symlinks.
        }

        let files = resolve_project_instructions("copilot", dir.path().to_str().unwrap());
        assert!(
            !files.iter().any(|f| f.path.contains("secret")),
            "a symlinked scan root must not be read: {:?}",
            files.iter().map(|f| &f.path).collect::<Vec<_>>()
        );
        let refused = files
            .iter()
            .find(|f| f.path == ".github/instructions")
            .expect("the refusal must be reported, not silent");
        assert!(refused
            .error
            .as_deref()
            .unwrap_or("")
            .contains("outside the working directory"));
    }

    #[test]
    fn an_overflowing_scan_stops_early_and_still_returns_the_same_set() {
        // The cap is now applied during the walk, not after it (Codex, PR
        // #3156) — collecting everything first left the ceiling doing nothing
        // about the cost it exists to bound. Stopping early only stays honest
        // because entries are ordered before descending; without that, two
        // calls would return different prefixes.
        let dir = tempfile::tempdir().unwrap();
        for i in 0..(MAX_SCANNED_INSTRUCTION_FILES + 25) {
            write(
                dir.path(),
                &format!(".github/instructions/f{i:04}.instructions.md"),
                "x",
            );
        }

        let first = resolve_project_instructions("copilot", dir.path().to_str().unwrap());
        let second = resolve_project_instructions("copilot", dir.path().to_str().unwrap());
        let p1: Vec<&str> = first.iter().map(|f| f.path.as_str()).collect();
        let p2: Vec<&str> = second.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(p1, p2, "an early stop must still be reproducible");

        assert!(
            first.iter().any(|f| f.path.ends_with('…')),
            "hitting the cap must be reported"
        );
        let listed = first
            .iter()
            .filter(|f| f.path.starts_with(".github/instructions/") && !f.path.ends_with('…'))
            .count();
        assert_eq!(listed, MAX_SCANNED_INSTRUCTION_FILES);
    }

    #[test]
    fn a_blank_working_directory_resolves_to_the_launch_default() {
        // The common case: launch uses default_agent_working_dir when the
        // stored value is blank, so reading the raw value reported nothing at
        // all for those agents (Codex, PR #3156).
        let resolved = effective_working_dir("", "My Agent");
        assert!(!resolved.trim().is_empty(), "must not resolve to nothing");
        // The default is itself a `~` path, so the blank case exercises the
        // expansion too — reading the stored value raw gave neither.
        let default_raw = crate::backend::storage::agents::default_agent_working_dir("My Agent");
        assert!(default_raw.starts_with('~'), "precondition: the default is a tilde path");
        assert!(!resolved.starts_with('~'), "must be expanded: {resolved}");
        assert!(
            resolved.ends_with(default_raw.trim_start_matches("~/")),
            "must be the same directory, expanded: {resolved} vs {default_raw}"
        );
    }

    #[test]
    fn a_tilde_path_is_expanded_the_way_launch_expands_it() {
        let expanded = effective_working_dir("~/projects/x", "ignored");
        assert!(!expanded.starts_with('~'), "got {expanded}");
        assert!(expanded.ends_with("projects/x"), "got {expanded}");
    }

    #[test]
    fn an_explicit_working_directory_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().to_str().unwrap();
        assert_eq!(effective_working_dir(p, "ignored"), p);
    }
}
