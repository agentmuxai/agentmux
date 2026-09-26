// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Same-identity relocation
//! (SPEC_RESUME_GATE_AND_SAME_IDENTITY_CONTINUATION_2026_09_25.md §4.3): the
//! file half. When the agent's chain head lives under another login of the
//! same Anthropic identity, one session file is copied into the spawn's
//! config dir so `--resume <head> --fork-session` can read it. The fork
//! writes a new session and never touches the copy or the original (verified
//! against Claude Code 2.1.280, spec §2.4), so the copy exists only until the
//! fork reports its own id.
//!
//! Every copy carries a marker naming the agent that made it, so the next
//! spawn of that agent can remove one a dead process left behind, and no
//! other agent's copy is touched. The marker is written before the copy
//! takes its real name: a copy is never visible to the CLI without one, so
//! a crash can't leave a lasting second file with the head's id (spec §3 I4).

use std::path::{Path, PathBuf};

use crate::backend::session_backfill::session_file_path;

const MARKER_SUFFIX: &str = ".agentmux-relocated";
const TMP_INFIX: &str = ".agentmux-tmp-";
/// How much of a session's end is read to check its last record parses.
const TAIL_CHECK_BYTES: u64 = 256 * 1024;

/// The session file `sid` for `cwd` under `config_dir`, or under the
/// always-global history dir of the same login when the channel-local one
/// is gone (a cleaned-up build channel): `channels/<c>/identities/<id>/claude`
/// has its `projects/` linked to `shared/identities/<id>/claude/projects`.
pub(crate) fn source_file(config_dir: &str, cwd: &str, sid: &str) -> Option<PathBuf> {
    let local = session_file_path(config_dir, cwd, sid)?;
    if local.is_file() {
        return Some(local);
    }
    let global = global_twin(config_dir)?;
    let path = session_file_path(&global.to_string_lossy(), cwd, sid)?;
    path.is_file().then_some(path)
}

/// `<shared>/identities/<id>/<rest…>` for a config dir under some
/// `…/identities/<id>/<rest…>`, when that differs from `config_dir`.
fn global_twin(config_dir: &str) -> Option<PathBuf> {
    let parts: Vec<String> =
        Path::new(config_dir).components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    let at = parts.iter().rposition(|p| p == "identities")?;
    let rest = parts.get(at + 1..).filter(|r| !r.is_empty())?;
    let mut twin = crate::registry::resolve_global_shared_root()?.join("identities");
    for p in rest {
        twin.push(p);
    }
    (twin != Path::new(config_dir)).then_some(twin)
}

/// Whether `path` is a session the CLI can resume: a non-empty file whose
/// last record parses (spec H5 — a file cut mid-write is not resumed).
pub(crate) fn is_resumable_file(path: &Path) -> bool {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(path) else { return false };
    let Ok(len) = f.metadata().map(|m| m.len()) else { return false };
    if len == 0 {
        return false;
    }
    let start = len.saturating_sub(TAIL_CHECK_BYTES);
    if f.seek(SeekFrom::Start(start)).is_err() {
        return false;
    }
    let mut tail = Vec::new();
    if f.read_to_end(&mut tail).is_err() {
        return false;
    }
    let text = String::from_utf8_lossy(&tail);
    text.lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .is_some_and(|l| serde_json::from_str::<serde_json::Value>(l).is_ok())
}

/// Copy `src` in as session `sid` for `cwd` under `dest_config_dir`, marked
/// as `owner_uid`'s. Refuses when the destination already exists: an
/// existing file with the head's id is never overwritten (spec §4.3 (5)).
/// Returns the copy's path.
pub(crate) fn relocate(src: &Path, dest_config_dir: &str, cwd: &str, sid: &str, owner_uid: &str) -> Result<PathBuf, String> {
    let dest = session_file_path(dest_config_dir, cwd, sid).ok_or("no destination path")?;
    if dest.exists() {
        return Err(format!("{} already exists", dest.display()));
    }
    let dir = dest.parent().ok_or("destination has no parent")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let tmp = dir.join(format!("{sid}.jsonl{TMP_INFIX}{owner_uid}"));
    let bytes = std::fs::copy(src, &tmp).map_err(|e| format!("copy {}: {e}", src.display()))?;
    let marker = marker_for(&dest);
    let record = serde_json::json!({ "owner_uid": owner_uid, "source": src.to_string_lossy(), "bytes": bytes });
    let placed = std::fs::write(&marker, record.to_string())
        .map_err(|e| format!("marker {}: {e}", marker.display()))
        .and_then(|()| std::fs::rename(&tmp, &dest).map_err(|e| format!("rename into {}: {e}", dest.display())));
    if let Err(e) = placed {
        let _ = std::fs::remove_file(&tmp);
        let _ = std::fs::remove_file(&marker);
        return Err(e);
    }
    Ok(dest)
}

/// Remove a copy [`relocate`] placed, and its marker.
pub(crate) fn remove(copy: &Path) {
    for p in [copy.to_path_buf(), marker_for(copy)] {
        if let Err(e) = std::fs::remove_file(&p) {
            if e.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(target: "continuity", path = %p.display(), error = %e, "relocated copy: remove failed");
            }
        }
    }
}

/// Drop a copy's marker only, keeping the copy: the CLI resumed it in place,
/// so it is a live session now and no sweep may remove it.
pub(crate) fn unmark(copy: &Path) {
    let _ = std::fs::remove_file(marker_for(copy));
}

/// Remove every copy `owner_uid` left in `cwd`'s project dir under
/// `config_dir` (a process that died before its fork reported an id), and
/// any half-made one. Returns how many copies went. Other agents' copies
/// and every ordinary session are left alone.
pub(crate) fn sweep(config_dir: &str, cwd: &str, owner_uid: &str) -> usize {
    let Some(dir) = session_file_path(config_dir, cwd, "x").and_then(|p| p.parent().map(Path::to_path_buf)) else {
        return 0;
    };
    let Ok(entries) = std::fs::read_dir(&dir) else { return 0 };
    let tmp_suffix = format!("{TMP_INFIX}{owner_uid}");
    let mut removed = 0;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(&tmp_suffix) {
            let _ = std::fs::remove_file(entry.path());
        } else if let Some(copy_name) = name.strip_suffix(MARKER_SUFFIX) {
            let owner = std::fs::read_to_string(entry.path())
                .ok()
                .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
                .and_then(|v| v.get("owner_uid").and_then(|o| o.as_str()).map(str::to_string));
            if owner.as_deref() == Some(owner_uid) {
                remove(&dir.join(copy_name));
                removed += 1;
            }
        }
    }
    removed
}

fn marker_for(copy: &Path) -> PathBuf {
    let mut name = copy.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(MARKER_SUFFIX);
    copy.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CWD: &str = "/agents/agenta";

    fn cfg() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    fn write_session(config_dir: &Path, sid: &str, body: &str) -> PathBuf {
        let p = session_file_path(&config_dir.to_string_lossy(), CWD, sid).unwrap();
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, body).unwrap();
        p
    }

    fn s(p: &Path) -> String {
        p.to_string_lossy().into_owned()
    }

    #[test]
    fn a_complete_session_is_resumable_and_a_cut_or_empty_one_is_not() {
        let d = cfg();
        let good = write_session(d.path(), "good", "{\"a\":1}\n{\"b\":2}\n");
        let cut = write_session(d.path(), "cut", "{\"a\":1}\n{\"b\":");
        let empty = write_session(d.path(), "empty", "");
        assert!(is_resumable_file(&good));
        assert!(!is_resumable_file(&cut), "a file cut mid-record");
        assert!(!is_resumable_file(&empty));
        assert!(!is_resumable_file(&d.path().join("missing.jsonl")));
    }

    #[test]
    fn relocation_copies_marks_and_never_touches_the_original() {
        let (from, to) = (cfg(), cfg());
        let src = write_session(from.path(), "head", "{\"turn\":1}\n");
        let copy = relocate(&src, &s(to.path()), CWD, "head", "uid-1").unwrap();
        assert_eq!(std::fs::read_to_string(&copy).unwrap(), "{\"turn\":1}\n");
        assert_eq!(copy, session_file_path(&s(to.path()), CWD, "head").unwrap());
        assert!(marker_for(&copy).is_file(), "a copy is never unmarked");
        assert_eq!(std::fs::read_to_string(&src).unwrap(), "{\"turn\":1}\n");
        let leftovers: Vec<_> = std::fs::read_dir(copy.parent().unwrap())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains(TMP_INFIX))
            .collect();
        assert!(leftovers.is_empty(), "no half-made copy remains");
    }

    #[test]
    fn an_existing_session_is_never_overwritten() {
        let (from, to) = (cfg(), cfg());
        let src = write_session(from.path(), "head", "{\"from\":\"elsewhere\"}\n");
        let existing = write_session(to.path(), "head", "{\"mine\":true}\n");
        assert!(relocate(&src, &s(to.path()), CWD, "head", "uid-1").is_err());
        assert_eq!(std::fs::read_to_string(&existing).unwrap(), "{\"mine\":true}\n");
        assert!(!marker_for(&existing).exists());
    }

    #[test]
    fn remove_takes_the_copy_and_its_marker() {
        let (from, to) = (cfg(), cfg());
        let src = write_session(from.path(), "head", "{}\n");
        let copy = relocate(&src, &s(to.path()), CWD, "head", "uid-1").unwrap();
        remove(&copy);
        assert!(!copy.exists() && !marker_for(&copy).exists());
        remove(&copy); // gone already: quiet
    }

    #[test]
    fn an_unmarked_copy_is_a_live_session_no_sweep_touches() {
        let (from, to) = (cfg(), cfg());
        let src = write_session(from.path(), "head", "{}\n");
        let copy = relocate(&src, &s(to.path()), CWD, "head", "uid-1").unwrap();
        unmark(&copy);
        assert_eq!(sweep(&s(to.path()), CWD, "uid-1"), 0);
        assert!(copy.exists());
    }

    #[test]
    fn a_sweep_removes_only_this_agents_copies() {
        let (from, to) = (cfg(), cfg());
        let src = write_session(from.path(), "head", "{}\n");
        let mine = relocate(&src, &s(to.path()), CWD, "head", "uid-1").unwrap();
        let src2 = write_session(from.path(), "other", "{}\n");
        let theirs = relocate(&src2, &s(to.path()), CWD, "other", "uid-2").unwrap();
        let ordinary = write_session(to.path(), "ordinary", "{}\n");
        let half_made = mine.parent().unwrap().join(format!("x.jsonl{TMP_INFIX}uid-1"));
        std::fs::write(&half_made, "{}").unwrap();

        assert_eq!(sweep(&s(to.path()), CWD, "uid-1"), 1);
        assert!(!mine.exists() && !marker_for(&mine).exists());
        assert!(!half_made.exists());
        assert!(theirs.exists() && marker_for(&theirs).exists(), "another agent's copy stays");
        assert!(ordinary.exists(), "an ordinary session stays");
        assert_eq!(sweep(&s(to.path()), CWD, "uid-1"), 0);
        assert_eq!(sweep("/no/such/dir", CWD, "uid-1"), 0);
    }

    #[test]
    fn the_source_is_found_under_the_login_s_global_history_when_the_channel_dir_is_gone() {
        // The crate-wide lock every `AGENTMUX_HOME_OVERRIDE` consumer takes.
        let _g = crate::test_support::ISOLATED_AUTH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = cfg();
        std::env::set_var("AGENTMUX_HOME_OVERRIDE", home.path());
        let gone_channel_dir = home.path().join("channels").join("old-build").join("identities").join("acct-1").join("claude");
        let global = home.path().join("shared").join("identities").join("acct-1").join("claude");
        let path = write_session(&global, "head", "{}\n");
        let found = source_file(&s(&gone_channel_dir), CWD, "head");
        let missing = source_file(&s(&gone_channel_dir), CWD, "missing");
        std::env::remove_var("AGENTMUX_HOME_OVERRIDE");
        assert_eq!(found, Some(path));
        assert_eq!(missing, None);
    }
}
