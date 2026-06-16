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
use std::path::Path;

/// Encode an absolute workspace path to a Claude project-dir slug. Claude Code
/// replaces each of `/ \ : .` with `-` (lossy — a literal `-` inside a segment is
/// indistinguishable from a separator; this matches the CLI's own scheme and the
/// `decode_project_path` direction in `history::claude_adapter`).
pub fn encode_project_slug(path: &str) -> String {
    path.chars()
        .map(|c| if matches!(c, '/' | '\\' | ':' | '.') { '-' } else { c })
        .collect()
}

/// The provider session id (jsonl stem) of the agent's **largest** session under
/// `projects_dir/<slug>`. We pick the largest, not the newest, on purpose: an
/// accidental fresh session (one started when a blank cross-channel open failed
/// to resume) must never win over the real conversation. `None` when there are
/// no session files (or the project dir doesn't exist).
pub fn largest_session_id(projects_dir: &Path, slug: &str) -> Option<String> {
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
    best.map(|(_, stem)| stem)
}

/// Populate `session_id` for registry records that lack one, from the agent's
/// largest provider session. Idempotent (skips records that already carry a
/// non-empty id). Returns the number populated. Best-effort per record — a
/// record with no `source_agents_base` or no transcript is skipped, never fatal.
pub fn backfill_session_ids(reg: &Registry, projects_dir: &Path) -> usize {
    let records = match reg.list_active() {
        Ok(r) => r,
        Err(_) => return 0,
    };
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
        let Some(sid) = largest_session_id(projects_dir, &slug) else {
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
        let projects = tmp.path();
        let slug = "C--Users-x--agentmux-agents-naki";
        let dir = projects.join(slug);
        fs::create_dir_all(&dir).unwrap();
        // The original long conversation...
        fs::write(dir.join("91f26930-long.jsonl"), vec![b'x'; 6_000_000]).unwrap();
        // ...and the accidental fresh short one started after the bug.
        fs::write(dir.join("e96ed91b-short.jsonl"), vec![b'x'; 15_000]).unwrap();
        // The recovery guarantee: pick the LARGE original, never the tiny recent.
        assert_eq!(
            largest_session_id(projects, slug).as_deref(),
            Some("91f26930-long")
        );
        // Missing project dir is graceful.
        assert_eq!(largest_session_id(projects, "nope"), None);
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
        let projects = tmp.path().join("projects");
        let base = r"C:\agents";
        // Provider sessions for agent "naki" (working_dir naki-0612a).
        let slug = encode_project_slug(&format!("{base}/naki-0612a"));
        let dir = projects.join(&slug);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("LONG.jsonl"), vec![b'x'; 1_000_000]).unwrap();
        fs::write(dir.join("short.jsonl"), vec![b'x'; 1_000]).unwrap();

        let reg = Registry::open(tmp.path().join("registry")).unwrap();
        reg.upsert(&rec("naki", Some(base), "naki-0612a", None)).unwrap();
        // Already-wired record must be left untouched.
        reg.upsert(&rec("keep", Some(base), "keep-x", Some("EXISTING"))).unwrap();
        // No transcript on disk → skipped, not an error.
        reg.upsert(&rec("notx", Some(base), "ghost-x", None)).unwrap();

        let n = backfill_session_ids(&reg, &projects);
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
        assert_eq!(backfill_session_ids(&reg, &projects), 0);
    }
}
