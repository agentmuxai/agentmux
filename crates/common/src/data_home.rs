// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! An isolated instance's data folder (task #3569).
//!
//! An instance started with `AGENTMUX_HOME_OVERRIDE` keeps the normal
//! versioned layout: the launcher puts each version's data under
//! `<override>/channels/<c>/versions/<v>/data`, and srv, once started, names
//! that folder in `AGENTMUX_DATA_HOME`. Without this, `agentmux_root()` kept
//! answering with the override root, srv kept `objects.db` in `<override>/db`,
//! and two versions shared one db, so the carry-forward step never ran.

use std::path::PathBuf;

/// `AGENTMUX_DATA_HOME` when it names a folder inside the override root (the
/// version folder srv was started on); `None` for no override, no data home,
/// the root itself, or a data home outside the root (an ambient leftover,
/// which the override still wins over).
pub(crate) fn inside_override() -> Option<PathBuf> {
    let root = PathBuf::from(std::env::var_os("AGENTMUX_HOME_OVERRIDE")?);
    let data = PathBuf::from(std::env::var_os("AGENTMUX_DATA_HOME")?);
    let usable = !root.as_os_str().is_empty() && !data.as_os_str().is_empty();
    (usable && data != root && data.starts_with(&root)).then_some(data)
}

#[cfg(test)]
mod tests {
    use crate::data_paths::agentmux_root;
    use tempfile::TempDir;

    #[test]
    fn agentmux_root_follows_a_data_home_inside_the_override() {
        let _lock = crate::TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let root = TempDir::new().expect("tempdir");
        let outside = TempDir::new().expect("tempdir");
        let version_dir = root.path().join("channels").join("x").join("versions").join("1.0.0").join("data");
        std::env::set_var("AGENTMUX_HOME_OVERRIDE", root.path());
        std::env::set_var("AGENTMUX_DATA_HOME", &version_dir);
        let inside = agentmux_root().expect("root resolves");
        std::env::set_var("AGENTMUX_DATA_HOME", outside.path());
        let elsewhere = agentmux_root().expect("root resolves");
        std::env::set_var("AGENTMUX_DATA_HOME", root.path());
        let same = agentmux_root().expect("root resolves");
        std::env::set_var("AGENTMUX_DATA_HOME", "");
        let empty = agentmux_root().expect("root resolves");
        std::env::remove_var("AGENTMUX_HOME_OVERRIDE");
        std::env::remove_var("AGENTMUX_DATA_HOME");
        assert_eq!(inside, version_dir);
        assert_eq!(elsewhere, root.path(), "an ambient data home outside the root loses");
        assert_eq!(same, root.path());
        assert_eq!(empty, root.path());
    }
}
