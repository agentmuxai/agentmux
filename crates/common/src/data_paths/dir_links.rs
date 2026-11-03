// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Directory helpers `DataPaths` leans on: the history link (a junction or symlink that
//! keeps one history folder across per-channel stores) and the owner-only root.

use std::path::Path;

/// Best-effort: ensure `link_path` (a `projects/` subdirectory inside an
/// [`super::isolated_auth_enabled`]-isolated provider credential dir) is a
/// directory junction (Windows) / symlink (Unix) pointing at
/// `target_dir` (the always-global [`super::DataPaths::identity_history_dir`]),
/// so a provider's session transcripts stay reachable across a
/// channel/build change even though the credential directory around them
/// is per-channel isolated. See
/// `docs/specs/SPEC_AGENT_IDENTITY_HISTORY_PERSISTENCE_PROTOCOL_2026_08_16.md`
/// §4.1.
///
/// A Windows **junction** (not a symlink) is used deliberately —
/// `std::os::windows::fs::symlink_dir` requires `SeCreateSymbolicLinkPrivilege`
/// (admin, or Developer Mode enabled), which this app cannot assume every
/// user has; junctions need no special privilege. Uses the `junction`
/// crate (`FSCTL_SET_REPARSE_POINT` under the hood) rather than
/// hand-rolled reparse-point FFI, for the same reason this codebase's own
/// `bundle.rs` comment notes about Windows symlink privilege requirements
/// — this is credential-adjacent storage, not a place to improvise
/// low-level filesystem code.
///
/// Idempotent: a no-op if `link_path` is already a link pointing at
/// `target_dir`. If a REAL directory already exists at `link_path` (data
/// written before this function existed, or before `isolated_auth_enabled`
/// applied to this bundle), its entries are moved into `target_dir` one
/// by one via `rename` (atomic, same-volume — always true here, both
/// live under the single `~/.agentmux` tree) before the now-empty
/// directory is replaced with the link. **Never deletes or overwrites
/// data**: an entry whose name already exists at the target is left
/// exactly where it was, under the old per-channel path, rather than
/// risking clobbering real history — it becomes reachable again once a
/// human resolves the name collision, rather than silently lost. Any
/// I/O error at any step is returned to the caller, who is expected to
/// treat this as best-effort (log and continue spawning) per the same
/// philosophy as the jekt-key injection in `agent_config.rs`.
pub fn ensure_history_link(link_path: &std::path::Path, target_dir: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(target_dir)?;
    // The `junction` crate creates the link via a plain `fs::create_dir`
    // (not `create_dir_all`) on `link_path` itself, so its parent must
    // already exist. Unix's `symlink` has the same requirement. Real
    // production callers already have this (the isolated identity dir is
    // created before this function runs), but this function must not
    // depend on caller ordering to be correct/testable on its own.
    if let Some(parent) = link_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    #[cfg(windows)]
    let already_correct = junction::exists(link_path).unwrap_or(false)
        && junction::get_target(link_path).ok().as_deref() == Some(target_dir);
    #[cfg(unix)]
    let already_correct = std::fs::symlink_metadata(link_path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
        && std::fs::read_link(link_path).ok().as_deref() == Some(target_dir);
    #[cfg(not(any(windows, unix)))]
    let already_correct = false;

    if already_correct {
        return Ok(());
    }

    match std::fs::symlink_metadata(link_path) {
        Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {
            // Pre-existing REAL directory: migrate its contents (never
            // clobbering an existing target entry), then remove the
            // now-empty directory so a link can take its place.
            for entry in std::fs::read_dir(link_path)? {
                let entry = entry?;
                let dest = target_dir.join(entry.file_name());
                if !dest.exists() {
                    std::fs::rename(entry.path(), &dest)?;
                }
            }
            // Only remove if migration left it empty (i.e. every entry
            // moved, or there were none) — a leftover name collision
            // means real, un-migrated data is still here, and this must
            // not silently paper over that by leaving a directory where
            // a link was expected. Surfacing the removal error is the
            // signal to the caller that this bundle needs manual review.
            std::fs::remove_dir(link_path)?;
        }
        Ok(meta) if meta.file_type().is_symlink() => {
            // A link already exists but pointed at something else (stale/
            // wrong target from a prior version of this function). Safe
            // to drop: no data lives directly at a link path, only at
            // whatever it points to, which this doesn't touch.
            #[cfg(windows)]
            {
                let _ = junction::delete(link_path);
            }
            #[cfg(unix)]
            {
                let _ = std::fs::remove_file(link_path);
            }
        }
        Ok(_) => {
            // A plain file (or anything else that's neither a real
            // directory nor a link) sits where a link needs to go.
            // Nothing in this codebase is expected to ever put a file
            // named "projects" here, but reagentx P2 on PR #2605 caught
            // that the previous version of this match arm silently
            // deleted it via remove_file/junction::delete with no
            // migration — directly contradicting this function's own
            // never-delete-data guarantee. Surface an error instead of
            // guessing what to do with unexpected real data.
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!(
                    "ensure_history_link: {} exists and is neither a directory nor a link; \
                     refusing to delete it",
                    link_path.display()
                ),
            ));
        }
        Err(_) => {
            // Nothing at link_path yet — normal first-time case.
        }
    }

    #[cfg(windows)]
    {
        junction::create(target_dir, link_path)?;
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target_dir, link_path)?;
    }
    #[cfg(not(any(windows, unix)))]
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "ensure_history_link: no directory-link mechanism on this platform",
        ));
    }

    Ok(())
}

/// What [`ensure_owner_only_dir`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnerOnlyOutcome {
    /// The directory did not exist; it was created with mode `0700`.
    Created,
    /// It existed with group/other permission bits set; those were removed.
    /// `previous_mode` is the old permission bits (e.g. `0o755`).
    Tightened { previous_mode: u32 },
    /// It already had no group/other access. Nothing changed.
    AlreadyPrivate,
    /// Nothing to do here: not a Unix platform (Windows scopes the user
    /// profile, and so `~/.agentmux`, to its owner through ACLs), or `dir`
    /// is the user's home directory itself — a misconfigured
    /// `AGENTMUX_HOME_OVERRIDE`/`AGENTMUX_DATA_HOME` pointing at `$HOME`
    /// must not chmod the whole home to `0700`.
    NotApplicable,
}

/// Make `dir` owner-only on Unix: create it `0700` if missing (its parents
/// with the normal umask — they are the user's, not AgentMux's), or strip
/// group/other bits from an existing directory, keeping the owner bits as
/// they were. See [`super::DataPaths::ensure_dirs`] for why (#3682).
///
/// Follows a symlinked root to its target, as every other access to the root
/// already does. Errors if `dir` exists but is not a directory.
pub fn ensure_owner_only_dir(dir: &Path) -> std::io::Result<OwnerOnlyOutcome> {
    ensure_owner_only_dir_inner(dir, dirs::home_dir().as_deref())
}

pub(super) fn ensure_owner_only_dir_inner(dir: &Path, user_home: Option<&Path>) -> std::io::Result<OwnerOnlyOutcome> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

        if let Some(home) = user_home {
            let same = match (std::fs::canonicalize(dir), std::fs::canonicalize(home)) {
                (Ok(a), Ok(b)) => a == b,
                _ => dir == home,
            };
            if same {
                return Ok(OwnerOnlyOutcome::NotApplicable);
            }
        }

        match std::fs::metadata(dir) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if let Some(parent) = dir.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                // Non-recursive on purpose: only the root itself is ours to
                // make private. The requested mode is still masked by the
                // umask, which can only remove bits — never add group/other.
                match std::fs::DirBuilder::new().mode(0o700).create(dir) {
                    Ok(()) => return Ok(OwnerOnlyOutcome::Created),
                    // Lost a race with a concurrent launch: fall through and
                    // check what that one created.
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(e) => return Err(e),
                }
            }
            Err(e) => return Err(e),
            Ok(_) => {}
        }

        let meta = std::fs::metadata(dir)?;
        if !meta.is_dir() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!("{} exists and is not a directory", dir.display()),
            ));
        }
        let mode = meta.permissions().mode() & 0o777;
        if mode & 0o077 == 0 {
            return Ok(OwnerOnlyOutcome::AlreadyPrivate);
        }
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(mode & 0o700))?;
        Ok(OwnerOnlyOutcome::Tightened { previous_mode: mode })
    }
    #[cfg(not(unix))]
    {
        let _ = (dir, user_home);
        Ok(OwnerOnlyOutcome::NotApplicable)
    }
}
