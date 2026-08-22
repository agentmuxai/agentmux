// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Browser-pane bookmarks — a flat list of saved URLs, deliberately stored
//! under `shared_dir` (`~/.agentmux/shared/browser-bookmarks.json`) rather
//! than `settings.json`. `settings.json` isolation is whole-file only (see
//! `docs/specs/SPEC_SETTINGS_ISOLATED_BY_CHANNEL_2026_08_19.md` §6 — per-key
//! tiering was considered and rejected there), so a key inside it can't stay
//! global while the rest of the file is channel-isolated. `shared_dir` is
//! this codebase's existing mechanism for data that must always be global
//! regardless of channel/isolation flags — the same directory already backs
//! the agent registry, transcripts, and `provider_auth_dir()`. See
//! `docs/specs/SPEC_BROWSER_PANE_BOOKMARKS_AND_GO_ICON_2026_08_22.md`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// One saved bookmark. `favicon_url` and `created_at` are best-effort —
/// absent on any record written before a field was added, so both default
/// on deserialize rather than failing the whole list over one old entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BrowserBookmark {
    pub id: String,
    pub title: String,
    pub url: String,
    #[serde(default)]
    pub favicon_url: String,
    #[serde(default)]
    pub created_at: i64,
}

const BOOKMARKS_FILE_NAME: &str = "browser-bookmarks.json";

/// Resolves the shared, cross-channel bookmarks file path. Returns `None`
/// when `DataPaths` can't resolve (CI / unusual env) — mirrors
/// `providers_handlers.rs`'s own `DataPaths::from_env()` best-effort
/// fallback, so callers stay best-effort too rather than erroring the whole
/// nav bar over an environment that never has this info anyway.
pub fn bookmarks_file_path() -> Option<PathBuf> {
    agentmux_common::DataPaths::from_env().map(|p| p.shared_dir.join(BOOKMARKS_FILE_NAME))
}

/// Read the bookmarks list. A missing file is "no bookmarks yet", not an
/// error — every fresh install/environment starts here. A present-but-
/// unparseable file IS an error: silently resetting to an empty list would
/// look exactly like the user's saved bookmarks vanished, so this fails
/// loud instead (matches this codebase's general preference for warning
/// over silent data loss, e.g. the ABF import path's warning-budget
/// conventions).
pub fn read_bookmarks(path: &Path) -> Result<Vec<BrowserBookmark>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = std::fs::read_to_string(path).map_err(|e| format!("read bookmarks: {e}"))?;
    if content.trim().is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str(&content).map_err(|e| format!("parse bookmarks: {e}"))
}

/// Write the whole bookmarks list, replacing whatever was there before —
/// the same wholesale-replace shape as `widget:pinned`'s `SetConfigCommand`
/// convention, just pointed at this dedicated file instead of
/// `settings.json`. Writes to a per-call unique temp file then renames over
/// the real path, so a crash or concurrent write never leaves a
/// half-written, corrupt `browser-bookmarks.json` behind (mirrors the
/// tmp-then-rename pattern used for native-memory file writes in
/// `bundle_import_for_agent_impl`). Two windows writing at the same instant
/// still race at the whole-file level — last write wins, no merge — an
/// accepted limitation, not something this function tries to solve.
pub fn write_bookmarks(path: &Path, bookmarks: &[BrowserBookmark]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
    }
    let json = serde_json::to_string_pretty(bookmarks).map_err(|e| format!("serialize bookmarks: {e}"))?;
    let tmp = path.with_file_name(format!(
        ".{BOOKMARKS_FILE_NAME}.{}.tmp",
        uuid::Uuid::new_v4()
    ));
    std::fs::write(&tmp, &json).map_err(|e| format!("write bookmarks: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("rename bookmarks: {e}")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bookmark(id: &str, url: &str) -> BrowserBookmark {
        BrowserBookmark {
            id: id.to_string(),
            title: format!("Title for {id}"),
            url: url.to_string(),
            favicon_url: String::new(),
            created_at: 0,
        }
    }

    #[test]
    fn missing_file_reads_as_empty_list() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(BOOKMARKS_FILE_NAME);
        assert_eq!(read_bookmarks(&path).unwrap(), Vec::new());
    }

    #[test]
    fn empty_file_reads_as_empty_list() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(BOOKMARKS_FILE_NAME);
        std::fs::write(&path, "").unwrap();
        assert_eq!(read_bookmarks(&path).unwrap(), Vec::new());
    }

    #[test]
    fn write_then_read_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(BOOKMARKS_FILE_NAME);
        let bookmarks = vec![
            bookmark("b1", "https://example.com"),
            bookmark("b2", "https://agentmux.ai"),
        ];
        write_bookmarks(&path, &bookmarks).unwrap();
        assert_eq!(read_bookmarks(&path).unwrap(), bookmarks);
    }

    #[test]
    fn write_creates_missing_parent_directories() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("dir").join(BOOKMARKS_FILE_NAME);
        write_bookmarks(&path, &[bookmark("b1", "https://example.com")]).unwrap();
        assert!(path.exists());
    }

    #[test]
    fn write_overwrites_a_previous_list_wholesale() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(BOOKMARKS_FILE_NAME);
        write_bookmarks(&path, &[bookmark("b1", "https://example.com")]).unwrap();
        write_bookmarks(&path, &[bookmark("b2", "https://agentmux.ai")]).unwrap();
        assert_eq!(read_bookmarks(&path).unwrap(), vec![bookmark("b2", "https://agentmux.ai")]);
    }

    #[test]
    fn corrupt_json_fails_loud_instead_of_silently_resetting_to_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(BOOKMARKS_FILE_NAME);
        std::fs::write(&path, "{not valid json").unwrap();
        let err = read_bookmarks(&path).unwrap_err();
        assert!(err.contains("parse bookmarks"));
    }

    #[test]
    fn old_record_missing_favicon_and_created_at_still_parses() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(BOOKMARKS_FILE_NAME);
        std::fs::write(&path, r#"[{"id":"b1","title":"T","url":"https://example.com"}]"#).unwrap();
        let bookmarks = read_bookmarks(&path).unwrap();
        assert_eq!(
            bookmarks,
            vec![BrowserBookmark {
                id: "b1".to_string(),
                title: "T".to_string(),
                url: "https://example.com".to_string(),
                favicon_url: String::new(),
                created_at: 0,
            }]
        );
    }
}
