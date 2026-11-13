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
    // A working directory is compared with its links followed, so a root that is itself a link
    // is compared by what it points at too.
    let resolved: Vec<PathBuf> = roots.iter().filter_map(|root| canonical(root)).collect();
    roots.extend(resolved);
    roots
}

/// `std::fs::canonicalize` of `path`, without the `\\?\` prefix Windows puts on it (the roots
/// and the paths compared with them are written without it). `None` when it does not resolve.
fn canonical(path: &Path) -> Option<PathBuf> {
    let resolved = std::fs::canonicalize(path).ok()?;
    #[cfg(windows)]
    {
        let text = resolved.to_string_lossy();
        if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
            return Some(PathBuf::from(format!(r"\\{rest}")));
        }
        if let Some(rest) = text.strip_prefix(r"\\?\") {
            if rest.as_bytes().get(1) == Some(&b':') {
                return Some(PathBuf::from(rest));
            }
        }
    }
    Some(resolved)
}

/// Whether `meta` is a symlink, or on Windows any reparse point (a junction is one).
fn is_link(meta: &std::fs::Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        return meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0;
    }
    #[cfg(not(windows))]
    false
}

/// `path` with every `.` and `..` resolved, and every symlink on the way followed as it is met,
/// so a `..` after a link goes up from the link's target, as the filesystem does (`link/..` is
/// the target's parent, not the link's). A folder that does not exist yet (or a link that does
/// not resolve) is kept as written. A `..` above the root stays at the root.
fn normalize_path(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => out.push(prefix.as_os_str()),
            Component::RootDir => out.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => match out.components().next_back() {
                Some(Component::Normal(_)) => {
                    out.pop();
                }
                // At a root or prefix there is nowhere above it to go.
                Some(_) => {}
                None => out.push(".."),
            },
            Component::Normal(part) => {
                out.push(part);
                if std::fs::symlink_metadata(&out).is_ok_and(|meta| is_link(&meta)) {
                    if let Some(target) = canonical(&out) {
                        out = target;
                    }
                }
            }
        }
    }
    out
}

/// `path` made absolute against the process's current directory, exactly as srv resolves a
/// relative `cmd:cwd` (and ACP/MSP's `session_cwd` does), with parent traversal resolved. The
/// traversal must be gone before the ownership check, or `.agentmux/../project` reads as owned by
/// its text and the launch writes the config (and its keys) into the project (#47392).
fn absolutize(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    };
    normalize_path(&absolute)
}

/// Whether `work_dir` is AgentMux's own folder, where the config belongs in it.
pub fn is_agent_owned_workdir(work_dir: &Path) -> bool {
    let work_dir = absolutize(work_dir);
    owned_roots().iter().any(|root| is_within(&work_dir, root))
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
/// own (the file belongs in the working directory itself). A relative path is resolved first, so
/// both the writer and ACP/MSP's `session_cwd` key the same folder.
pub fn relocated_dir_for(expanded: &Path) -> Option<PathBuf> {
    let expanded = absolutize(expanded);
    if is_agent_owned_workdir(&expanded) {
        return None;
    }
    let root = crate::data_paths::agentmux_root().ok()?;
    Some(root.join("mcp").join(workdir_key(&expanded)))
}

/// The relocated config for an expanded working directory, or `None` when the file belongs in
/// the working directory itself.
pub fn relocated_path_for(expanded: &Path) -> Option<PathBuf> {
    relocated_dir_for(expanded).map(|dir| dir.join(RELOCATED_MCP_JSON))
}

/// Where this agent's MCP config is: in its working directory when AgentMux owns the folder,
/// else under the data root.
pub fn config_path_for(expanded: &Path) -> PathBuf {
    let expanded = absolutize(expanded);
    relocated_path_for(&expanded).unwrap_or_else(|| expanded.join(MCP_JSON))
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
        assert_eq!(config_path_for(&agent), absolutize(&agent).join(MCP_JSON));
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

    /// A `..` must be resolved before the ownership check: `.agentmux/agents/x/../../../..` reads
    /// as AgentMux's by its text, but is the user's folder.
    #[test]
    fn parent_traversal_cannot_pass_as_an_agentmux_folder() {
        let home = dirs::home_dir().unwrap();
        let inside_agent = home.join(".agentmux").join("agents").join("aria");
        assert!(is_agent_owned_workdir(&inside_agent), "the fixture starts owned");
        let escapes = inside_agent.join("..").join("..").join("..").join("..").join("agentmux-escape-project");
        assert!(!is_agent_owned_workdir(&escapes), "resolved out of .agentmux: {}", escapes.display());
        assert_eq!(relocated_dir_for(&escapes), relocated_dir_for(&normalize_path(&escapes)), "one resolved key");
        assert!(!config_path_for(&escapes).starts_with(normalize_path(&escapes)), "the config is not in the project");
    }

    /// A directory link: a symlink, or a junction on Windows (it needs no privilege there).
    fn link_dir(link: &Path, target: &Path) {
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, link).unwrap();
        #[cfg(windows)]
        {
            let made = std::process::Command::new("cmd").args(["/C", "mklink", "/J"]).arg(link).arg(target).output().unwrap();
            assert!(made.status.success(), "mklink /J: {}", String::from_utf8_lossy(&made.stdout));
        }
    }

    /// #47418: `<dir>/link/..` is the link's target's parent, not `<dir>`. Resolved by its text,
    /// a project reached through a link to a folder elsewhere would read as the folder the link
    /// is in, and be classified (and written into) as if it were there.
    #[test]
    fn a_parent_after_a_link_goes_up_from_the_links_target() {
        let tmp = tempfile::tempdir().unwrap();
        let base = canonical(tmp.path()).expect("the temp folder resolves");
        let child = base.join("projects").join("child");
        std::fs::create_dir_all(&child).unwrap();
        std::fs::create_dir_all(base.join("home")).unwrap();
        let link = base.join("home").join("link");
        link_dir(&link, &child);

        let through_link = link.join("..").join("sibling");
        assert_eq!(normalize_path(&through_link), base.join("projects").join("sibling"));
        assert_ne!(normalize_path(&through_link), base.join("home").join("sibling"), "not the lexical answer");
        // The same folder spelled with and without the link keys one config folder.
        assert_eq!(relocated_dir_for(&through_link), relocated_dir_for(&base.join("projects").join("sibling")));
        // A link as the last part is followed too, and a path with no link in it is left as written.
        assert_eq!(normalize_path(&link), child);
        assert_eq!(normalize_path(&base.join("home").join("not-there")), base.join("home").join("not-there"));
    }

    /// #47392: the writer gets `cmd:cwd` as written; ACP/MSP resolve a relative one against the
    /// server's cwd before asking. Both must key the same folder.
    #[test]
    fn a_relative_workdir_resolves_like_the_absolute_one() {
        let cwd = std::env::current_dir().unwrap();
        let relative = Path::new("agentmux-relative-test-project");
        let absolute = cwd.join(relative);
        assert_eq!(relocated_dir_for(relative), relocated_dir_for(&absolute));
        assert_eq!(relocated_path_for(relative), relocated_path_for(&absolute));
        assert_eq!(config_path_for(relative), config_path_for(&absolute));
        assert_eq!(is_agent_owned_workdir(relative), is_agent_owned_workdir(&absolute));
    }
}
