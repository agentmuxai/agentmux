// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Where an agent's MCP config lives (#3680), shared by srv and `agentmux-mcp doctor`.
//!
//! An agent's working directory may be the user's own project. A launch used to write
//! `.mcp.json` there; now, for a folder AgentMux does not own, the config goes under the data
//! root, outside the project:
//!
//! ```text
//! <data root>/mcp/<hash of the expanded working directory>/mcp.json
//! ```
//!
//! In a folder AgentMux owns the file stays `<working directory>/.mcp.json`, which Claude Code
//! reads on its own. srv derives the path to write and read it; `agentmux-mcp doctor` derives
//! the same one to diagnose it, so the two cannot disagree.
//!
//! A container gets the folder as a read-only bind at [`CONTAINER_MCP_CONFIG_DIR`]; the file
//! keeps its name inside it. The folder rather than the file matters: a single-file bind keeps
//! the inode it was made with, so an atomic rewrite would leave the container on a stale copy.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// The file name every provider's workspace MCP config uses.
pub const MCP_JSON: &str = ".mcp.json";
/// The file name of the relocated config inside its own folder.
pub const RELOCATED_MCP_JSON: &str = "mcp.json";
/// Where a container agent's relocated config folder is mounted.
pub const CONTAINER_MCP_CONFIG_DIR: &str = "/home/agent/.agentmux/mcp";

/// `path` as a comparable string: separators normalized, no trailing slash, and folded on
/// Windows, where two spellings of one folder must compare equal.
fn normalized(path: &Path) -> String {
    let s = path.to_string_lossy().replace('\\', "/");
    let s = s.trim_end_matches('/').to_string();
    if cfg!(windows) {
        s.to_lowercase()
    } else {
        s
    }
}

fn is_within(path: &Path, root: &Path) -> bool {
    let path = normalized(path);
    let root = normalized(root);
    !root.is_empty() && (path == root || path.starts_with(&format!("{root}/")))
}

/// The roots a launch may write into: AgentMux's own data tree.
fn owned_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Ok(root) = crate::data_paths::agentmux_root() {
        roots.push(root);
    }
    // The default agent home is `~/.agentmux/agents/<slug>`, resolved from the real home even
    // when a data-home override is set for an isolated instance.
    if let Some(home) = dirs::home_dir() {
        roots.push(home.join(".agentmux"));
    }
    roots
}

/// Whether `work_dir` is AgentMux's own folder, where the config belongs in it.
pub fn is_agent_owned_workdir(work_dir: &Path) -> bool {
    owned_roots().iter().any(|root| is_within(work_dir, root))
}

/// Expand a leading `~` against the real home. srv has its own expander (it also honours the
/// dev-instance home override for paths under `~/.agentmux`); a user's project expands the same
/// way in both.
pub fn expand_home(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if text == "~" {
        return dirs::home_dir().unwrap_or_else(|| path.to_path_buf());
    }
    if let Some(rest) = text.strip_prefix("~/").or_else(|| text.strip_prefix("~\\")) {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    path.to_path_buf()
}

/// A stable, filesystem-safe key for an already-expanded working directory.
pub fn workdir_key(expanded: &Path) -> String {
    let mut hash = Sha256::new();
    hash.update(normalized(expanded).as_bytes());
    let digest = hash.finalize();
    digest[..16].iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The folder holding `expanded`'s relocated config, or `None` when the folder is AgentMux's
/// own (the file belongs in the working directory itself).
pub fn relocated_dir_for(expanded: &Path) -> Option<PathBuf> {
    if is_agent_owned_workdir(expanded) {
        return None;
    }
    let root = crate::data_paths::agentmux_root().ok()?;
    Some(root.join("mcp").join(workdir_key(expanded)))
}

/// The relocated config for an expanded working directory, or `None` when the file belongs in
/// the working directory itself.
pub fn relocated_path_for(expanded: &Path) -> Option<PathBuf> {
    relocated_dir_for(expanded).map(|dir| dir.join(RELOCATED_MCP_JSON))
}

/// Where this agent's MCP config is: in its working directory when AgentMux owns the folder,
/// else under the data root.
pub fn config_path_for(expanded: &Path) -> PathBuf {
    relocated_path_for(expanded).unwrap_or_else(|| expanded.join(MCP_JSON))
}

/// The config's path inside a container, where its folder is mounted.
pub fn container_config_path() -> PathBuf {
    Path::new(CONTAINER_MCP_CONFIG_DIR).join(RELOCATED_MCP_JSON)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_agents_own_folder_keeps_the_workspace_file() {
        let home = dirs::home_dir().unwrap();
        let agent = home.join(".agentmux").join("agents").join("aria");
        assert!(relocated_path_for(&agent).is_none(), "an AgentMux folder keeps .mcp.json in it");
        assert_eq!(config_path_for(&agent), agent.join(MCP_JSON));
    }

    #[test]
    fn a_users_project_gets_a_folder_outside_it_with_mcp_json_in_it() {
        // A directory that is not under any AgentMux root, without having to exist.
        let project = if cfg!(windows) { Path::new(r"C:\agentmux-test-project") } else { Path::new("/agentmux-test-project") };
        assert!(!is_agent_owned_workdir(project));
        let path = relocated_path_for(project).expect("a project is not AgentMux's");
        assert_eq!(path.file_name().and_then(|n| n.to_str()), Some(RELOCATED_MCP_JSON));
        assert!(path.parent().is_some_and(|dir| dir.file_name().is_some_and(|n| n.to_str().unwrap_or("").len() == 32)), "a 16-byte hex key: {path:?}");
        assert!(!path.starts_with(project), "never under the project: {path:?}");
        assert_eq!(config_path_for(project), path);
    }

    #[test]
    fn the_same_project_gets_the_same_key_and_another_gets_another() {
        let a = Path::new("/agentmux-test-a");
        let b = Path::new("/agentmux-test-b");
        assert_eq!(workdir_key(a), workdir_key(a));
        assert_ne!(workdir_key(a), workdir_key(b));
    }

    #[test]
    fn a_containers_config_is_the_file_inside_the_mounted_folder() {
        assert_eq!(
            container_config_path(),
            Path::new(CONTAINER_MCP_CONFIG_DIR).join(RELOCATED_MCP_JSON)
        );
    }

    #[test]
    fn a_tilde_is_expanded_for_a_user_path() {
        let home = dirs::home_dir().unwrap();
        assert_eq!(expand_home(Path::new("~/work")), home.join("work"));
        assert_eq!(expand_home(Path::new("/already/absolute")), Path::new("/already/absolute"));
    }
}
