// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Backfill the registry record `session_id` from each agent's provider
//! transcript, so a cross-channel open `--resume`s the original conversation
//! instead of starting a fresh session.
//!
//! Background: the registry `session_id` is **read** on launch (surfaced to the
//! picker, which passes it as `--resume <sid>` on the first turn of a reattached
//! block — `agent_handlers.rs`) but was **never written** by production code, so
//! it was always `null`. A fresh-build / cross-channel open therefore had no sid
//! to resume and spawned a brand-new session, which then shadowed the original.
//! See docs/retro/retro-cross-channel-conversation-continuity-regression-2026-06-16.md.
//!
//! Once populated, `--resume` keeps the same session id across turns, so a single
//! idempotent startup pass keeps continuity solid without per-turn wiring.

use crate::registry::Registry;
use std::path::{Path, PathBuf};

/// Encode an absolute workspace path to a Claude project-dir slug. Claude Code
/// replaces each of `/ \ : .` with `-` (lossy — a literal `-` inside a segment is
/// indistinguishable from a separator; this matches the CLI's own scheme and the
/// `decode_project_path` direction in `history::claude_adapter`).
pub fn encode_project_slug(path: &str) -> String {
    path.chars()
        .map(|c| if matches!(c, '/' | '\\' | ':' | '.') { '-' } else { c })
        .collect()
}

/// The largest `(size, session-id)` directly under `projects_dir/<slug>`, or
/// `None` if the dir doesn't exist / has no session files.
fn largest_with_size(projects_dir: &Path, slug: &str) -> Option<(u64, String)> {
    let dir = projects_dir.join(slug);
    let mut best: Option<(u64, String)> = None;
    for entry in std::fs::read_dir(&dir).ok()?.flatten() {
        let p = entry.path();
        if p.extension().map_or(false, |e| e == "jsonl") {
            if let (Ok(meta), Some(stem)) = (entry.metadata(), p.file_stem()) {
                let stem = stem.to_string_lossy().to_string();
                if best.as_ref().map_or(true, |(sz, _)| meta.len() > *sz) {
                    best = Some((meta.len(), stem));
                }
            }
        }
    }
    best
}

/// The provider session id (jsonl stem) of the agent's **largest** session across
/// the candidate project roots (the account-wide default and, for identity-bound
/// agents, the identity bundle). We pick the largest, not the newest, on purpose:
/// an accidental fresh session (one started when a blank cross-channel open failed
/// to resume) must never win over the real conversation. `None` when there are no
/// session files under any root.
pub fn largest_session_id(projects_dirs: &[PathBuf], slug: &str) -> Option<String> {
    let mut best: Option<(u64, String)> = None;
    for d in projects_dirs {
        if let Some((sz, stem)) = largest_with_size(d, slug) {
            if best.as_ref().map_or(true, |(b, _)| sz > *b) {
                best = Some((sz, stem));
            }
        }
    }
    best.map(|(_, stem)| stem)
}

/// Recovery lookup for a confirmed-stale `--resume` failure
/// (`docs/status/STATUS_CROSS_CHANNEL_RESUME_STALE_SESSION_ID_2026_08_20.md`):
/// rather than giving up and starting a blank conversation the moment a
/// registry/inherited `session_id` turns out to be unreachable, look for
/// the largest session actually on disk under this exact `config_dir` (the
/// `CLAUDE_CONFIG_DIR` the CLI process is about to run with) for
/// `working_dir`. `None` when `config_dir` is empty/unset (nothing to
/// search) or no session exists — callers fall back to starting blank in
/// either case, same as before this existed.
///
/// `config_dir` is expanded the same way the actual spawn path expands
/// every env var (`core::apply_working_dir` -> `expand_home_dir_safe`)
/// before applying it to the child process — a `CLAUDE_CONFIG_DIR` of
/// `~/.claude` must resolve to the real home directory here too, or this
/// searches a literal, nonexistent `~/.claude/projects` while the CLI
/// itself reads the real expanded path, silently defeating recovery for
/// every `~`-shorthand config dir (Codex P2 on PR #2693).
///
/// `working_dir` gets the same treatment, via `core.rs`'s OWN
/// `expand_home_dir` — not `base::expand_home_dir_safe` above, a
/// different function; `apply_working_dir` uses each for a different
/// input and this must mirror both exactly. This one matters even more
/// than the config-dir case: `agent_open.rs`'s default cwd for any agent
/// without an explicit `working_directory` is the literal string
/// `~/.agentmux/agents/<slug>` — the single most common case, not an
/// edge case — so without this, `encode_project_slug` would hash the
/// wrong (literal-tilde) path and recovery would silently fail for
/// nearly every default-configured agent (Codex P1 on PR #2693, found
/// after #2693 had already merged — the config-dir half of this same
/// class of bug was fixed pre-merge, this half wasn't caught in time).
pub fn find_largest_session_for_working_dir(config_dir: &str, working_dir: &str) -> Option<String> {
    if config_dir.is_empty() {
        return None;
    }
    let expanded = crate::backend::base::expand_home_dir_safe(config_dir);
    let projects_dir = expanded.join("projects");
    let expanded_working_dir = crate::backend::blockcontroller::core::expand_home_dir(working_dir);
    let slug = encode_project_slug(&expanded_working_dir);
    largest_session_id(&[projects_dir], &slug)
}

/// Populate `session_id` for registry records that lack one, from the agent's
/// largest provider session. Idempotent (skips records that already carry a
/// non-empty id). Returns the number populated. Best-effort per record — a
/// record with no `source_agents_base` or no transcript is skipped, never fatal.
pub fn backfill_session_ids(reg: &Registry, shared_dir: &Path) -> usize {
    let records = match reg.list_active() {
        Ok(r) => r,
        Err(_) => return 0,
    };
    let default_projects = shared_dir
        .join("providers")
        .join("claude")
        .join("projects");
    let mut count = 0;
    for mut rec in records {
        if rec.data.session_id.as_deref().map_or(false, |s| !s.is_empty()) {
            continue; // already wired
        }
        let Some(base) = rec.data.source_agents_base.as_deref() else {
            continue;
        };
        let base = base.trim_end_matches(['/', '\\']);
        let workspace = format!("{base}/{}", rec.data.working_dir);
        let slug = encode_project_slug(&workspace);
        // Candidate project roots: the account-wide default, plus the agent's
        // identity bundle when bound to a non-default identity — identity-bound
        // agents write sessions under `identities/<id>/claude/projects` (per
        // `history::claude_adapter` discovery). [reagent #1479 P2]
        let mut dirs = vec![default_projects.clone()];
        if let Some(id) = rec.data.identity_id.as_deref() {
            if !id.is_empty() && id != "default" {
                dirs.push(
                    shared_dir
                        .join("identities")
                        .join(id)
                        .join("claude")
                        .join("projects"),
                );
            }
        }
        let Some(sid) = largest_session_id(&dirs, &slug) else {
            continue;
        };
        rec.data.session_id = Some(sid.clone());
        if reg.upsert(&rec).is_ok() {
            count += 1;
            tracing::info!(
                instance = %rec.data.instance_name,
                session_id = %sid,
                "registry: backfilled session_id for cross-channel resume"
            );
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{NamedAgentRecord, NamedAgentRecordV1};
    use std::fs;

    #[test]
    fn encode_slug_matches_claude_convention() {
        // Verified against a real on-disk slug (Naki).
        assert_eq!(
            encode_project_slug(r"C:\Users\asafe\.agentmux\agents\naki-0612a"),
            "C--Users-asafe--agentmux-agents-naki-0612a"
        );
        // POSIX path; a literal '-' inside a segment is preserved.
        assert_eq!(
            encode_project_slug("/home/u/.agentmux/agents/foo-bar"),
            "-home-u--agentmux-agents-foo-bar"
        );
    }

    #[test]
    fn largest_session_beats_a_fresh_short_one() {
        let tmp = tempfile::tempdir().unwrap();
        let projects = tmp.path().to_path_buf();
        let slug = "C--Users-x--agentmux-agents-naki";
        let dir = projects.join(slug);
        fs::create_dir_all(&dir).unwrap();
        // The original long conversation...
        fs::write(dir.join("91f26930-long.jsonl"), vec![b'x'; 6_000_000]).unwrap();
        // ...and the accidental fresh short one started after the bug.
        fs::write(dir.join("e96ed91b-short.jsonl"), vec![b'x'; 15_000]).unwrap();
        // The recovery guarantee: pick the LARGE original, never the tiny recent.
        assert_eq!(
            largest_session_id(&[projects.clone()], slug).as_deref(),
            Some("91f26930-long")
        );
        // Missing project dir is graceful.
        assert_eq!(largest_session_id(&[projects], "nope"), None);
    }

    #[test]
    fn find_largest_session_for_working_dir_locates_the_real_transcript() {
        let tmp = tempfile::tempdir().unwrap();
        let config_dir = tmp.path();
        let working_dir = r"C:\Users\asafe\.agentmux\agents\agentx-0623n";
        let slug = encode_project_slug(working_dir);
        let dir = config_dir.join("projects").join(&slug);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("972a6a4f-live.jsonl"), vec![b'x'; 2_800_000]).unwrap();

        assert_eq!(
            find_largest_session_for_working_dir(&config_dir.to_string_lossy(), working_dir).as_deref(),
            Some("972a6a4f-live")
        );
    }

    #[test]
    fn find_largest_session_for_working_dir_is_none_for_an_empty_config_dir() {
        // An unset CLAUDE_CONFIG_DIR (empty string) must not be treated as
        // "search the current directory" — nothing to recover from.
        assert_eq!(
            find_largest_session_for_working_dir("", r"C:\Users\asafe\.agentmux\agents\agentx-0623n"),
            None
        );
    }

    #[test]
    fn find_largest_session_for_working_dir_is_none_when_nothing_exists_there() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(
            find_largest_session_for_working_dir(
                &tmp.path().to_string_lossy(),
                r"C:\Users\asafe\.agentmux\agents\nobody-here",
            ),
            None
        );
    }

    /// Codex P2 on PR #2693: `config_dir` must be expanded the same way the
    /// actual spawn path expands every env var (`core::apply_working_dir`'s
    /// `expand_home_dir_safe`) — a literal, un-expanded `~/...` must not be
    /// searched as-is, or recovery silently fails for every config dir that
    /// uses `~`-shorthand. Writes into a uniquely-named folder under the
    /// REAL home directory (there's no way to test `~` resolution without
    /// one) and removes it afterward regardless of outcome.
    #[test]
    fn find_largest_session_for_working_dir_expands_a_tilde_config_dir() {
        let home = dirs::home_dir().expect("test requires a resolvable home dir");
        let rel = format!(".agentmux-test-tilde-expansion-{}", std::process::id());
        let config_dir_abs = home.join(&rel);
        let working_dir = r"C:\Users\test\.agentmux\agents\tilde-test";
        let slug = encode_project_slug(working_dir);
        let dir = config_dir_abs.join("projects").join(&slug);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("abc123-session.jsonl"), b"{}").unwrap();

        let result = find_largest_session_for_working_dir(&format!("~/{rel}"), working_dir);

        fs::remove_dir_all(&config_dir_abs).ok();

        assert_eq!(
            result.as_deref(),
            Some("abc123-session"),
            "a ~-prefixed config_dir must resolve to the real home directory, not be searched literally"
        );
    }

    /// Codex P1 on PR #2693 (found post-merge, in a follow-up review pass
    /// against the fix commit): `working_dir` needs the exact same
    /// expansion `config_dir` got above — via `core.rs`'s OWN
    /// `expand_home_dir`, a different function from `base::
    /// expand_home_dir_safe` used for `config_dir`, since
    /// `apply_working_dir` uses one for each. This is the MORE important
    /// half: `agent_open.rs`'s default cwd for any agent with no explicit
    /// `working_directory` is literally `~/.agentmux/agents/<slug>` — not
    /// an edge case, the single most common configuration — so without
    /// this, recovery silently fails for nearly every default agent.
    #[test]
    fn find_largest_session_for_working_dir_expands_a_tilde_working_dir() {
        let home = dirs::home_dir().expect("test requires a resolvable home dir");
        let config_rel = format!(".agentmux-test-tilde-workdir-config-{}", std::process::id());
        let config_dir_abs = home.join(&config_rel);
        // Mirrors agent_open.rs's actual default cwd format exactly.
        let working_dir_rel = format!(".agentmux-test-tilde-workdir-agent-{}", std::process::id());
        let working_dir_tilde = format!("~/{working_dir_rel}");
        let expanded_working_dir = home.join(&working_dir_rel);

        let slug = encode_project_slug(&expanded_working_dir.to_string_lossy());
        let dir = config_dir_abs.join("projects").join(&slug);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("def456-session.jsonl"), b"{}").unwrap();

        let result = find_largest_session_for_working_dir(&config_dir_abs.to_string_lossy(), &working_dir_tilde);

        fs::remove_dir_all(&config_dir_abs).ok();

        assert_eq!(
            result.as_deref(),
            Some("def456-session"),
            "a ~-prefixed working_dir (agent_open.rs's actual default cwd shape) must resolve to \
             the real home directory, not be encoded literally with the tilde"
        );
    }

    fn rec(id: &str, base: Option<&str>, wd: &str, sid: Option<&str>) -> NamedAgentRecord {
        NamedAgentRecord {
            schema_version: 3,
            data: NamedAgentRecordV1 {
                instance_id: id.to_string(),
                instance_name: id.to_string(),
                definition_id: "claude-code".to_string(),
                identity_id: Some("default".to_string()),
                memory_id: None,
                session_id: sid.map(String::from),
                working_dir: wd.to_string(),
                source_agents_base: base.map(String::from),
                created_at_ms: 1,
                last_launched_at_ms: 1,
                created_by_version: "(legacy)".to_string(),
                last_launched_by_version: "(legacy)".to_string(),
            },
        }
    }

    #[test]
    fn backfill_fills_empties_picks_largest_and_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let shared = tmp.path();
        let projects = shared.join("providers").join("claude").join("projects");
        let base = r"C:\agents";
        // Provider sessions for agent "naki" (working_dir naki-0612a).
        let slug = encode_project_slug(&format!("{base}/naki-0612a"));
        let dir = projects.join(&slug);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("LONG.jsonl"), vec![b'x'; 1_000_000]).unwrap();
        fs::write(dir.join("short.jsonl"), vec![b'x'; 1_000]).unwrap();

        let reg = Registry::open(shared.join("registry")).unwrap();
        reg.upsert(&rec("naki", Some(base), "naki-0612a", None)).unwrap();
        // Already-wired record must be left untouched.
        reg.upsert(&rec("keep", Some(base), "keep-x", Some("EXISTING"))).unwrap();
        // No transcript on disk → skipped, not an error.
        reg.upsert(&rec("notx", Some(base), "ghost-x", None)).unwrap();

        let n = backfill_session_ids(&reg, shared);
        assert_eq!(n, 1, "only the one empty record with a transcript is filled");

        let by = |id: &str| {
            reg.list_active()
                .unwrap()
                .into_iter()
                .find(|r| r.data.instance_id == id)
                .unwrap()
        };
        assert_eq!(by("naki").data.session_id.as_deref(), Some("LONG"), "largest session");
        assert_eq!(by("keep").data.session_id.as_deref(), Some("EXISTING"), "untouched");
        assert_eq!(by("notx").data.session_id, None, "no transcript → left null");

        // Idempotent: a second pass fills nothing.
        assert_eq!(backfill_session_ids(&reg, shared), 0);
    }

    #[test]
    fn backfill_resolves_identity_bundle_sessions() {
        // An identity-bound agent writes sessions under
        // identities/<id>/claude/projects, NOT the default root. [reagent #1479 P2]
        let tmp = tempfile::tempdir().unwrap();
        let shared = tmp.path();
        let base = r"C:\agents";
        let slug = encode_project_slug(&format!("{base}/bound-0612a"));
        let idir = shared
            .join("identities")
            .join("bundle1")
            .join("claude")
            .join("projects")
            .join(&slug);
        fs::create_dir_all(&idir).unwrap();
        fs::write(idir.join("BOUND.jsonl"), vec![b'x'; 500_000]).unwrap();

        let reg = Registry::open(shared.join("registry")).unwrap();
        let mut r = rec("bound", Some(base), "bound-0612a", None);
        r.data.identity_id = Some("bundle1".to_string());
        reg.upsert(&r).unwrap();

        assert_eq!(backfill_session_ids(&reg, shared), 1);
        let got = reg
            .list_active()
            .unwrap()
            .into_iter()
            .find(|x| x.data.instance_id == "bound")
            .unwrap();
        assert_eq!(
            got.data.session_id.as_deref(),
            Some("BOUND"),
            "identity-bound session resolved from the bundle dir"
        );
    }
}
