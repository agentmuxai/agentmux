// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Where an agent's MCP config lives (#3680).
//!
//! An agent's `working_directory` may be the user's own project. A launch used to write
//! `.mcp.json` there, which overwrote the project's own file and put the agent's signing
//! keys where a routine `git add -A` could publish them. This module is the one answer to
//! "where is this agent's MCP config":
//!
//! - In a folder AgentMux owns (under its data directory) the file stays where it has
//!   always been, `<working directory>/.mcp.json`, which Claude Code reads on its own.
//! - In a user's project the file moves to AgentMux's data directory, outside the project,
//!   in a folder keyed by the project path: `<data dir>/mcp/<hash>/mcp.json`. Claude is
//!   pointed at it with `--mcp-config` (it still reads the project's own `.mcp.json`
//!   itself); every other harness is given the servers from it by srv. A launch never
//!   writes into the project.
//!
//! The config has its own folder rather than a bare file because a container gets it as a
//! bind mount. A single-file bind keeps the inode it was made with, so an atomic rewrite
//! (a new inode) would leave a running container reading the old config; a directory bind
//! follows the replacement.
//!
//! The relocated file is written `0600` and lives under the data root AgentMux keeps
//! owner-only. The path itself is derived by [`agentmux_common::mcp_config_path`], so srv
//! (which writes it) and `agentmux-mcp doctor` (which diagnoses it) cannot disagree.

use std::io::Write;
use std::path::{Path, PathBuf};

pub use agentmux_common::mcp_config_path::MCP_JSON;
use agentmux_common::mcp_config_path as shared;

/// Whether `work_dir` is AgentMux's own folder, where a launch may keep the agent's
/// `.mcp.json` as it always has.
pub fn is_agent_owned_workdir(work_dir: &Path) -> bool {
    shared::is_agent_owned_workdir(work_dir)
}

/// The folder holding `work_dir`'s relocated config, or `None` when the folder is AgentMux's
/// own and the file belongs in it (`<work_dir>/.mcp.json`). This is what a container binds
/// (a directory, not the file: see the module doc).
pub fn relocated_dir(work_dir: &str) -> Option<PathBuf> {
    let expanded = crate::backend::blockcontroller::core::expand_home_dir(work_dir);
    shared::relocated_dir_for(Path::new(&expanded))
}

/// The relocated config for `work_dir`, or `None` when the folder is AgentMux's own and the
/// file belongs in it (`<work_dir>/.mcp.json`).
pub fn relocated_path(work_dir: &str) -> Option<PathBuf> {
    let expanded = crate::backend::blockcontroller::core::expand_home_dir(work_dir);
    shared::relocated_path_for(Path::new(&expanded))
}

/// Where this agent's MCP config is: in its working directory when AgentMux owns the folder,
/// else under AgentMux's data root.
pub fn config_path(work_dir: &str) -> PathBuf {
    let expanded = crate::backend::blockcontroller::core::expand_home_dir(work_dir);
    shared::config_path_for(Path::new(&expanded))
}

/// Write the relocated config `content` at `path`: parent folders created, the file `0600` on
/// Unix, through a temporary file and a rename so it is never briefly readable.
pub fn write_relocated(path: &Path, content: &str) -> std::io::Result<()> {
    let Some(parent) = path.parent() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "the MCP config path has no parent",
        ));
    };
    std::fs::create_dir_all(parent)?;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("mcp.json");
    let tmp = parent.join(format!(".{name}.{}.tmp", uuid::Uuid::new_v4()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let written = options.open(&tmp).and_then(|mut file| {
        file.write_all(content.as_bytes())?;
        file.sync_all()
    });
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

/// The MCP config a host turn reads: for a user's project, AgentMux's relocated config merged
/// with the project's own `.mcp.json` servers (read-only — the project file is never written),
/// so a user's servers keep reaching every harness, as they did when the launch merged them.
/// `None` when neither file exists or is usable.
pub fn read_for_turn(work_dir: &str) -> Option<String> {
    let expanded = crate::backend::blockcontroller::core::expand_home_dir(work_dir);
    let Some(relocated) = relocated_path(&expanded) else {
        return std::fs::read_to_string(Path::new(&expanded).join(MCP_JSON)).ok();
    };
    let ours = std::fs::read_to_string(&relocated).ok();
    let theirs = std::fs::read_to_string(Path::new(&expanded).join(MCP_JSON)).ok();
    match (ours, theirs) {
        (Some(ours), Some(theirs)) => Some(merge_project_servers(&ours, &theirs)),
        (Some(ours), None) => Some(ours),
        (None, Some(theirs)) => Some(theirs),
        (None, None) => None,
    }
}

/// `ours` with every `mcpServers` entry from `theirs` added where it does not already exist.
/// AgentMux's own entries win a name clash; other top-level keys are ours'.
fn merge_project_servers(ours: &str, theirs: &str) -> String {
    let Ok(mut ours) = serde_json::from_str::<serde_json::Value>(ours) else {
        return theirs.to_string();
    };
    let Ok(theirs) = serde_json::from_str::<serde_json::Value>(theirs) else {
        return ours.to_string();
    };
    let Some(ours_obj) = ours.as_object_mut() else {
        return theirs.to_string();
    };
    let servers = ours_obj
        .entry("mcpServers")
        .or_insert_with(|| serde_json::json!({}));
    let Some(servers) = servers.as_object_mut() else {
        return ours.to_string();
    };
    if let Some(theirs) = theirs
        .get("mcpServers")
        .and_then(serde_json::Value::as_object)
    {
        for (name, value) in theirs {
            servers.entry(name.clone()).or_insert_with(|| value.clone());
        }
    }
    serde_json::to_string_pretty(&ours).unwrap_or_else(|_| ours.to_string())
}

/// The one lock every test holds while it points `AGENTMUX_DATA_HOME` at a temp folder. The
/// variable is process-wide, so tests in different modules must not overlap; a second private
/// mutex would let one test restore the value under another.
#[cfg(test)]
pub(crate) fn data_home_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// A test points `AGENTMUX_DATA_HOME` at a temp folder so the relocated configs it writes never
/// reach the real data root. Tests that set it hold [`data_home_lock`].
#[cfg(test)]
pub(crate) fn with_data_home<T>(root: &Path, f: impl FnOnce() -> T) -> T {
    let _held = data_home_lock();
    let saved = std::env::var_os("AGENTMUX_DATA_HOME");
    std::env::set_var("AGENTMUX_DATA_HOME", root);
    let out = f();
    match saved {
        Some(v) => std::env::set_var("AGENTMUX_DATA_HOME", v),
        None => std::env::remove_var("AGENTMUX_DATA_HOME"),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_users_project_gets_a_config_outside_it_and_an_agent_folder_keeps_its_own() {
        let data = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        with_data_home(data.path(), || {
            let project_path = project.path().to_string_lossy().into_owned();
            let relocated =
                relocated_path(&project_path).expect("a project folder is not AgentMux's");
            assert!(
                relocated.starts_with(data.path().join("mcp")),
                "{relocated:?}"
            );
            assert!(
                !relocated.starts_with(project.path()),
                "never under the project: {relocated:?}"
            );

            let owned = data.path().join("agents").join("aria");
            std::fs::create_dir_all(&owned).unwrap();
            let owned_path = owned.to_string_lossy().into_owned();
            assert_eq!(relocated_path(&owned_path), None);
            assert_eq!(config_path(&owned_path), owned.join(".mcp.json"));
        });
    }

    #[test]
    fn the_same_project_always_gets_the_same_file_and_another_gets_another() {
        let data = tempfile::tempdir().unwrap();
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        with_data_home(data.path(), || {
            let a = a.path().to_string_lossy().into_owned();
            let b = b.path().to_string_lossy().into_owned();
            assert_eq!(config_path(&a), config_path(&a));
            assert_ne!(config_path(&a), config_path(&b));
        });
    }

    #[test]
    fn the_relocated_file_is_written_owner_only_and_read_back() {
        let data = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        with_data_home(data.path(), || {
            let path = relocated_path(&project.path().to_string_lossy()).unwrap();
            write_relocated(&path, r#"{"mcpServers":{"agentmux":{}}}"#).unwrap();
            assert_eq!(
                std::fs::read_to_string(&path).unwrap(),
                r#"{"mcpServers":{"agentmux":{}}}"#
            );
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                assert_eq!(
                    std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                    0o600
                );
            }
        });
    }

    #[test]
    fn a_project_file_is_merged_read_only_with_agentmuxs_servers_winning() {
        let data = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        with_data_home(data.path(), || {
            std::fs::write(
                project.path().join(MCP_JSON),
                r#"{"mcpServers":{"agentmux":{"command":"stale"},"mine":{"command":"mine-mcp"}}}"#,
            )
            .unwrap();
            let path = relocated_path(&project.path().to_string_lossy()).unwrap();
            write_relocated(
                &path,
                r#"{"mcpServers":{"agentmux":{"command":"agentmux-mcp"}}}"#,
            )
            .unwrap();
            let merged = read_for_turn(&project.path().to_string_lossy()).unwrap();
            let value: serde_json::Value = serde_json::from_str(&merged).unwrap();
            assert_eq!(
                value["mcpServers"]["agentmux"]["command"], "agentmux-mcp",
                "ours wins"
            );
            assert_eq!(
                value["mcpServers"]["mine"]["command"], "mine-mcp",
                "the user's server is kept"
            );
            assert_eq!(
                std::fs::read_to_string(project.path().join(MCP_JSON)).unwrap(),
                r#"{"mcpServers":{"agentmux":{"command":"stale"},"mine":{"command":"mine-mcp"}}}"#,
                "the project file is never rewritten"
            );
        });
    }
}
