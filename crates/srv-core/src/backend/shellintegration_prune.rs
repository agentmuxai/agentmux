// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Removes the per-instance shell-integration folders (`~/.agentmux/shell/<hash>`)
//! of instances that no longer exist.
//!
//! `integration_base_for` keys a folder by a hash of the instance's data dir, so
//! every dev build, test home and old version leaves one behind (372 on one
//! machine after two months). A hash can't be reversed, so each folder records
//! the data dir it belongs to in `.instance`; a folder is stale when that dir is
//! gone. Age is never the test.
//!
//! A folder from before the marker existed is matched against the instances
//! still on disk and stamped, so it is cleaned up later once its dir goes away.
//! One that matches nothing is left alone: its dir may live anywhere.

use std::path::{Path, PathBuf};

use super::shellintegration::instance_key;

const INSTANCE_MARKER: &str = ".instance";

#[derive(Debug, Default, PartialEq, Eq)]
pub struct PruneReport {
    pub removed: usize,
    pub stamped: usize,
    /// Folders with no marker that match no known instance: left in place.
    pub unattributed: usize,
}

/// Records which instance `base` belongs to. Called at deploy.
pub fn stamp_instance(base: &Path, data_dir: &Path) {
    let canonical = data_dir.canonicalize().unwrap_or_else(|_| data_dir.to_path_buf());
    if let Err(e) = std::fs::write(base.join(INSTANCE_MARKER), canonical.to_string_lossy().as_bytes()) {
        tracing::debug!("shell integration: could not stamp {}: {}", base.display(), e);
    }
}

fn is_instance_folder_name(name: &str) -> bool {
    name.len() == 16 && name.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// True only when the path is definitely absent. An unreadable path (a drive
/// that is offline, a permissions error) is not proof the instance is gone.
fn definitely_gone(path: &Path) -> bool {
    matches!(std::fs::metadata(path), Err(e) if e.kind() == std::io::ErrorKind::NotFound)
}

/// Sweeps `shell_root`, keeping `current_base` and every folder whose instance
/// still exists. `live_data_dirs` are the instance data dirs found on disk.
pub fn prune_stale_integration_dirs(
    shell_root: &Path,
    current_base: &Path,
    live_data_dirs: &[PathBuf],
) -> PruneReport {
    let mut report = PruneReport::default();
    let Ok(entries) = std::fs::read_dir(shell_root) else { return report };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if !is_instance_folder_name(&name) || !path.is_dir() || path == current_base {
            continue;
        }
        match std::fs::read_to_string(path.join(INSTANCE_MARKER)) {
            Ok(recorded) => {
                // Verbatim: a data dir's name may end in whitespace, and `stamp_instance` writes no newline.
                if !recorded.is_empty() && definitely_gone(Path::new(&recorded)) {
                    match std::fs::remove_dir_all(&path) {
                        Ok(()) => report.removed += 1,
                        Err(e) => tracing::debug!("shell integration: could not remove {}: {}", path.display(), e),
                    }
                }
            }
            Err(_) => match live_data_dirs.iter().find(|d| instance_key(d) == name) {
                Some(dir) => {
                    stamp_instance(&path, dir);
                    report.stamped += 1;
                }
                None => report.unattributed += 1,
            },
        }
    }
    report
}

/// The data dir of every instance `~/.agentmux` knows of: installed versions
/// and dev builds.
pub fn live_instance_data_dirs(agentmux_home: &Path) -> Vec<PathBuf> {
    crate::registry::enumerate_objects_dbs(agentmux_home)
        .into_iter()
        .filter_map(|db| db.parent()?.parent().map(Path::to_path_buf))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(root: &Path, name: &str) -> PathBuf {
        let p = root.join(name);
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(p.join("muxlog.mjs"), "x").unwrap();
        p
    }

    #[test]
    fn a_folder_whose_instance_dir_is_gone_is_removed_and_a_live_one_stays() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("shell");
        let live_dir = tmp.path().join("live-data");
        std::fs::create_dir_all(&live_dir).unwrap();
        let live = folder(&root, "aaaaaaaaaaaaaaaa");
        let dead = folder(&root, "bbbbbbbbbbbbbbbb");
        let current = folder(&root, "cccccccccccccccc");
        stamp_instance(&live, &live_dir);
        stamp_instance(&dead, &tmp.path().join("deleted-data"));
        let report = prune_stale_integration_dirs(&root, &current, &[]);
        assert_eq!(report, PruneReport { removed: 1, stamped: 0, unattributed: 0 });
        assert!(live.exists() && current.exists() && !dead.exists());
    }

    #[test]
    fn an_unmarked_folder_is_stamped_when_its_instance_exists_and_left_when_unknown() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("shell");
        let data = tmp.path().join("inst").join("data");
        std::fs::create_dir_all(&data).unwrap();
        let matched = folder(&root, &instance_key(&data));
        let unknown = folder(&root, "dddddddddddddddd");
        let current = root.join("eeeeeeeeeeeeeeee");
        let report = prune_stale_integration_dirs(&root, &current, &[data.clone()]);
        assert_eq!(report, PruneReport { removed: 0, stamped: 1, unattributed: 1 });
        assert!(unknown.exists());
        assert!(std::fs::read_to_string(matched.join(INSTANCE_MARKER)).unwrap().contains("data"));
        // Once the instance is gone, the stamped folder is stale.
        std::fs::remove_dir_all(&data).unwrap();
        let report = prune_stale_integration_dirs(&root, &current, &[]);
        assert_eq!(report.removed, 1);
        assert!(!matched.exists() && unknown.exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_data_dir_whose_name_ends_in_whitespace_is_still_found_alive() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("shell");
        let data = tmp.path().join("data ");
        std::fs::create_dir_all(&data).unwrap();
        let live = folder(&root, "aaaaaaaaaaaaaaaa");
        stamp_instance(&live, &data);
        let report = prune_stale_integration_dirs(&root, &root.join("cccccccccccccccc"), &[]);
        assert_eq!(report, PruneReport::default());
        assert!(live.exists());
    }

    #[test]
    fn other_entries_under_the_shell_root_are_never_touched() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("shell");
        let bash = folder(&root, "bash");
        let lib = folder(&root, "lib");
        std::fs::write(root.join(".version"), "v").unwrap();
        let current = root.join("ffffffffffffffff");
        let report = prune_stale_integration_dirs(&root, &current, &[]);
        assert_eq!(report, PruneReport::default());
        assert!(bash.exists() && lib.exists() && root.join(".version").exists());
    }
}
