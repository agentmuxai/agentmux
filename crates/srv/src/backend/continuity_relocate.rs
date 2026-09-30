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
//! Each marker's name carries a token of its own, so a relocation settles
//! only its own copy, never a later one placed at the same path.

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

/// A copy [`relocate`] placed, with the marker that makes it this
/// relocation's. Each relocation's marker has its own name, so holding one
/// is ownership: once a sweep has taken it (and a later relocation placed a
/// new copy at the same path, under a new marker), settling through the old
/// handle finds its marker gone and touches nothing (codex P1 on #3907).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Relocated {
    pub copy: PathBuf,
    marker: PathBuf,
}

/// Copy `src` in as session `sid` for `cwd` under `dest_config_dir`, marked
/// as `owner_uid`'s. Refuses when the destination already exists: an
/// existing file with the head's id is never overwritten (spec §4.3 (5)).
pub(crate) fn relocate(src: &Path, dest_config_dir: &str, cwd: &str, sid: &str, owner_uid: &str) -> Result<Relocated, String> {
    let dest = session_file_path(dest_config_dir, cwd, sid).ok_or("no destination path")?;
    if dest.exists() {
        return Err(format!("{} already exists", dest.display()));
    }
    let dir = dest.parent().ok_or("destination has no parent")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    // Both names carry this relocation's token: once placed, the temp name is
    // a hard link to the copy, so a concurrent relocation reusing it would
    // write straight into the winner's copy (codex P1 on #3924).
    let token = uuid::Uuid::new_v4().simple().to_string();
    let tmp = dir.join(format!("{sid}.jsonl{TMP_INFIX}{owner_uid}.{token}"));
    let bytes = match std::fs::copy(src, &tmp) {
        Ok(b) => b,
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            return Err(format!("copy {}: {e}", src.display()));
        }
    };
    let marker = marker_for(&dest, &token);
    let record = serde_json::json!({ "owner_uid": owner_uid, "source": src.to_string_lossy(), "bytes": bytes });
    let placed = std::fs::write(&marker, record.to_string())
        .map_err(|e| format!("marker {}: {e}", marker.display()))
        .and_then(|()| place(&tmp, &dest).map_err(|e| format!("place {}: {e}", dest.display())));
    let _ = std::fs::remove_file(&tmp);
    if let Err(e) = placed {
        let _ = std::fs::remove_file(&marker);
        return Err(e);
    }
    Ok(Relocated { copy: dest, marker })
}

/// Give the finished copy its real name, only if nothing has it: two spawns
/// relocating the same head at once both pass the `exists` check, and a
/// rename would let the second replace the first's copy under both markers
/// (codex P1 on #3924). A hard link fails on an existing name, atomically.
fn place(tmp: &Path, dest: &Path) -> std::io::Result<()> {
    std::fs::hard_link(tmp, dest)
}

/// Remove a copy [`relocate`] placed, and its marker, if it is still this
/// relocation's. The marker goes first: only a relocation that still held
/// it removes the copy. Returns whether it did.
pub(crate) fn remove(r: &Relocated) -> bool {
    if !unmark(r) {
        return false;
    }
    remove_file(&r.copy);
    true
}

/// Drop a copy's marker only, keeping the copy: the CLI resumed it in place,
/// so it is a live session now and no sweep may remove it. Returns whether
/// the copy was still this relocation's.
pub(crate) fn unmark(r: &Relocated) -> bool {
    match std::fs::remove_file(&r.marker) {
        Ok(()) => true,
        Err(e) => {
            if e.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(target: "continuity", path = %r.marker.display(), error = %e, "relocated copy: unmark failed");
            }
            false
        }
    }
}

fn remove_file(p: &Path) {
    if let Err(e) = std::fs::remove_file(p) {
        if e.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!(target: "continuity", path = %p.display(), error = %e, "relocated copy: remove failed");
        }
    }
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
    // `<sid>.jsonl.agentmux-tmp-<uid>.<token>`, or without the token from
    // before tokens.
    let tmp_tag = format!("{TMP_INFIX}{owner_uid}");
    let mut removed = 0;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(&tmp_tag) || name.contains(&format!("{tmp_tag}.")) {
            let _ = std::fs::remove_file(entry.path());
        } else if let Some(copy_name) = name.strip_suffix(MARKER_SUFFIX).map(copy_name_of) {
            let owner = std::fs::read_to_string(entry.path())
                .ok()
                .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
                .and_then(|v| v.get("owner_uid").and_then(|o| o.as_str()).map(str::to_string));
            if owner.as_deref() == Some(owner_uid) {
                remove(&Relocated { copy: dir.join(copy_name), marker: entry.path() });
                removed += 1;
            }
        }
    }
    removed
}

/// `<copy>.<token>.agentmux-relocated`: the token makes each relocation's
/// marker its own.
fn marker_for(copy: &Path, token: &str) -> PathBuf {
    let mut name = copy.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(format!(".{token}{MARKER_SUFFIX}"));
    copy.with_file_name(name)
}

/// The copy a marker (its name without the suffix) is for. A marker from
/// before tokens is `<sid>.jsonl.agentmux-relocated`.
fn copy_name_of(stem: &str) -> &str {
    if stem.ends_with(".jsonl") {
        stem
    } else {
        stem.rsplit_once('.').map_or(stem, |(copy, _token)| copy)
    }
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
        let r = relocate(&src, &s(to.path()), CWD, "head", "uid-1").unwrap();
        assert_eq!(std::fs::read_to_string(&r.copy).unwrap(), "{\"turn\":1}\n");
        assert_eq!(r.copy, session_file_path(&s(to.path()), CWD, "head").unwrap());
        assert!(r.marker.is_file(), "a copy is never unmarked");
        assert_eq!(std::fs::read_to_string(&src).unwrap(), "{\"turn\":1}\n");
        let leftovers: Vec<_> = std::fs::read_dir(r.copy.parent().unwrap())
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
        let markers = std::fs::read_dir(existing.parent().unwrap())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(MARKER_SUFFIX))
            .count();
        assert_eq!(markers, 0);
    }

    #[test]
    fn remove_takes_the_copy_and_its_marker() {
        let (from, to) = (cfg(), cfg());
        let src = write_session(from.path(), "head", "{}\n");
        let r = relocate(&src, &s(to.path()), CWD, "head", "uid-1").unwrap();
        assert!(remove(&r));
        assert!(!r.copy.exists() && !r.marker.exists());
        assert!(!remove(&r), "gone already: quiet, and not this relocation's any more");
    }

    /// A replacement spawn swept this relocation's copy and placed its own at
    /// the same path: the old handle settles nothing (codex P1 on #3907).
    #[test]
    fn a_stale_handle_never_touches_a_later_relocation_at_the_same_path() {
        let (from, to) = (cfg(), cfg());
        let src = write_session(from.path(), "head", "{}\n");
        let old = relocate(&src, &s(to.path()), CWD, "head", "uid-1").unwrap();
        assert_eq!(sweep(&s(to.path()), CWD, "uid-1"), 1);
        let new = relocate(&src, &s(to.path()), CWD, "head", "uid-1").unwrap();
        assert_eq!(old.copy, new.copy);

        assert!(!unmark(&old));
        assert!(!remove(&old));
        assert!(new.copy.exists() && new.marker.exists(), "the replacement's copy is still its own");
        assert!(unmark(&new));
    }

    /// The loser of two concurrent relocations of one head fails to place its
    /// copy instead of replacing the winner's.
    #[test]
    fn placing_never_replaces_a_copy_already_there() {
        let d = cfg();
        let dest = write_session(d.path(), "head", "{\"winner\":true}\n");
        let tmp = dest.with_file_name("head.jsonl.agentmux-tmp-uid-2");
        std::fs::write(&tmp, "{\"loser\":true}\n").unwrap();
        assert_eq!(place(&tmp, &dest).unwrap_err().kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "{\"winner\":true}\n");
    }

    /// Each relocation copies through its own temp name, so none can write
    /// into another's placed copy through a shared one (codex P1 on #3924).
    #[test]
    fn a_relocation_leaves_no_writable_alias_of_its_copy() {
        let (from, to) = (cfg(), cfg());
        let src = write_session(from.path(), "head", "{\"turn\":1}\n");
        let r = relocate(&src, &s(to.path()), CWD, "head", "uid-1").unwrap();
        let names: Vec<String> = std::fs::read_dir(r.copy.parent().unwrap())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert!(!names.iter().any(|n| n.contains(TMP_INFIX)), "no temp name left: {names:?}");
    }

    #[test]
    fn half_made_copies_are_swept_with_or_without_a_token_but_only_this_agents() {
        let to = cfg();
        let dir = write_session(to.path(), "ordinary", "{}\n").parent().unwrap().to_path_buf();
        let tokened = dir.join(format!("head.jsonl{TMP_INFIX}uid-1.abc123"));
        let legacy = dir.join(format!("other.jsonl{TMP_INFIX}uid-1"));
        let longer_uid = dir.join(format!("head.jsonl{TMP_INFIX}uid-10.abc123"));
        for p in [&tokened, &legacy, &longer_uid] {
            std::fs::write(p, "{}").unwrap();
        }
        sweep(&s(to.path()), CWD, "uid-1");
        assert!(!tokened.exists() && !legacy.exists());
        assert!(longer_uid.exists(), "uid-10's temp file is not uid-1's");
    }

    #[test]
    fn a_marker_from_before_tokens_is_still_swept() {
        let to = cfg();
        let copy = write_session(to.path(), "head", "{}\n");
        let legacy = copy.with_file_name(format!("head.jsonl{MARKER_SUFFIX}"));
        std::fs::write(&legacy, r#"{"owner_uid":"uid-1"}"#).unwrap();
        assert_eq!(sweep(&s(to.path()), CWD, "uid-1"), 1);
        assert!(!copy.exists() && !legacy.exists());
    }

    #[test]
    fn an_unmarked_copy_is_a_live_session_no_sweep_touches() {
        let (from, to) = (cfg(), cfg());
        let src = write_session(from.path(), "head", "{}\n");
        let r = relocate(&src, &s(to.path()), CWD, "head", "uid-1").unwrap();
        assert!(unmark(&r));
        assert_eq!(sweep(&s(to.path()), CWD, "uid-1"), 0);
        assert!(r.copy.exists());
    }

    #[test]
    fn a_sweep_removes_only_this_agents_copies() {
        let (from, to) = (cfg(), cfg());
        let src = write_session(from.path(), "head", "{}\n");
        let mine = relocate(&src, &s(to.path()), CWD, "head", "uid-1").unwrap();
        let src2 = write_session(from.path(), "other", "{}\n");
        let theirs = relocate(&src2, &s(to.path()), CWD, "other", "uid-2").unwrap();
        let ordinary = write_session(to.path(), "ordinary", "{}\n");
        let half_made = mine.copy.parent().unwrap().join(format!("x.jsonl{TMP_INFIX}uid-1"));
        std::fs::write(&half_made, "{}").unwrap();

        assert_eq!(sweep(&s(to.path()), CWD, "uid-1"), 1);
        assert!(!mine.copy.exists() && !mine.marker.exists());
        assert!(!half_made.exists());
        assert!(theirs.copy.exists() && theirs.marker.exists(), "another agent's copy stays");
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
