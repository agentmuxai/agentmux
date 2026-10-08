// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Which files an agent may upload into a browser pane
//! (docs/specs/SPEC_AGENT_DRIVEN_BROWSER_PANES_2026_10_07.md §5.3): only
//! files inside its own workspace, so a page (or a tricked agent) can't use
//! an upload field to carry off anything else on the machine.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};


/// At most this many files per upload.
pub(crate) const MAX_UPLOAD_FILES: usize = 10;

fn launch_workspaces() -> &'static Mutex<HashMap<String, PathBuf>> {
    static W: OnceLock<Mutex<HashMap<String, PathBuf>>> = OnceLock::new();
    W.get_or_init(Default::default)
}

/// Remember the workspace the agent on `block_id` was started in: the
/// `AGENTMUX_AGENT_WORKDIR` srv just put in its spawn env (host agents only;
/// a container agent has none). Called by every spawn path right after
/// `agent_handlers::input::carry_agent_workdir_env`.
///
/// Uploads are checked against this record, not the block's `cmd:cwd`: any
/// client with the instance auth key can rewrite that meta, so an agent
/// could otherwise point its "workspace" at `/` and upload anything.
pub(crate) fn record_launch_workspace(block_id: &str, env_vars: &HashMap<String, String>) {
    let mut map = launch_workspaces().lock().unwrap_or_else(|p| p.into_inner());
    match env_vars.get("AGENTMUX_AGENT_WORKDIR").filter(|w| !w.trim().is_empty()) {
        Some(w) => {
            map.insert(block_id.to_string(), PathBuf::from(w));
        }
        None => {
            map.remove(block_id);
        }
    }
}

/// The workspace the agent on `block_id` was launched in, if srv started it
/// as a host agent.
pub(crate) fn agent_workspace(block_id: &str) -> Option<PathBuf> {
    launch_workspaces().lock().unwrap_or_else(|p| p.into_inner()).get(block_id).cloned()
}

/// `canonicalize` without Windows' `\\?\` verbatim prefix, which the browser
/// doesn't expect in a file path.
fn canonical(p: &Path) -> std::io::Result<PathBuf> {
    let c = std::fs::canonicalize(p)?;
    let s = c.to_string_lossy();
    Ok(match s.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC\\") => PathBuf::from(rest),
        _ => c,
    })
}

/// Check `paths` (absolute, or relative to the workspace) and return their
/// canonical forms: each must be an existing file inside `workspace` after
/// resolving `..` and symlinks.
pub(crate) fn check_upload_paths(workspace: &Path, paths: &[String]) -> Result<Vec<String>, String> {
    if paths.is_empty() {
        return Err("no files given".to_string());
    }
    if paths.len() > MAX_UPLOAD_FILES {
        return Err(format!("at most {MAX_UPLOAD_FILES} files per upload"));
    }
    let ws = canonical(workspace)
        .map_err(|e| format!("your workspace {} can't be read: {e}", workspace.display()))?;
    let mut out = Vec::with_capacity(paths.len());
    for p in paths {
        let raw = Path::new(p.trim());
        let joined = if raw.is_absolute() { raw.to_path_buf() } else { workspace.join(raw) };
        let c = canonical(&joined).map_err(|_| format!("no such file: {p}"))?;
        if !c.starts_with(&ws) {
            return Err(format!(
                "{p} is outside your workspace ({}); an agent can upload only files in its own workspace: copy the file there first",
                ws.display()
            ));
        }
        if !c.is_file() {
            return Err(format!("{p} is not a file"));
        }
        out.push(c.to_string_lossy().into_owned());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::obj::MetaMapType;
    use serde_json::json;

    fn ws() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("agent-ws");
        std::fs::create_dir_all(ws.join("sub")).unwrap();
        std::fs::write(ws.join("report.txt"), "x").unwrap();
        std::fs::write(ws.join("sub").join("a.bin"), "y").unwrap();
        std::fs::write(dir.path().join("secret.txt"), "z").unwrap();
        (dir, ws)
    }

    #[test]
    fn files_inside_the_workspace_are_allowed_absolute_or_relative() {
        let (_d, ws) = ws();
        let got = check_upload_paths(&ws, &["report.txt".into(), ws.join("sub").join("a.bin").to_string_lossy().into_owned()]).unwrap();
        assert_eq!(got.len(), 2);
        assert!(got[0].ends_with("report.txt") && !got[0].starts_with(r"\\?\"), "{got:?}");
    }

    #[test]
    fn a_file_outside_the_workspace_is_refused_including_through_dotdot() {
        let (d, ws) = ws();
        let e = check_upload_paths(&ws, &[d.path().join("secret.txt").to_string_lossy().into_owned()]).unwrap_err();
        assert!(e.contains("outside your workspace"), "{e}");
        let e = check_upload_paths(&ws, &["../secret.txt".into()]).unwrap_err();
        assert!(e.contains("outside your workspace"), "{e}");
    }

    #[test]
    fn missing_files_directories_and_too_many_are_refused() {
        let (_d, ws) = ws();
        assert!(check_upload_paths(&ws, &["nope.txt".into()]).unwrap_err().contains("no such file"));
        assert!(check_upload_paths(&ws, &["sub".into()]).unwrap_err().contains("not a file"));
        assert!(check_upload_paths(&ws, &[]).is_err());
        let many: Vec<String> = (0..11).map(|_| "report.txt".to_string()).collect();
        assert!(check_upload_paths(&ws, &many).unwrap_err().contains("at most"));
    }

    #[test]
    fn the_workspace_is_what_srv_launched_the_agent_in() {
        // Through the real spawn-env helper: host agents get one, a
        // container agent (or no launch directory) gets none.
        let spawn = |pairs: &[(&str, &str)]| {
            let mut m = MetaMapType::new();
            for (k, v) in pairs {
                m.insert(k.to_string(), json!(v));
            }
            let mut env = HashMap::new();
            crate::server::agent_handlers::input::carry_agent_workdir_env(&mut env, &m);
            env
        };
        record_launch_workspace("ws-test-a", &spawn(&[("cmd:cwd", "C:/work/agent")]));
        assert!(agent_workspace("ws-test-a").is_some());
        record_launch_workspace("ws-test-a", &spawn(&[("cmd:cwd", "C:/work/agent"), ("agentMode", "container")]));
        assert_eq!(agent_workspace("ws-test-a"), None);
        assert_eq!(agent_workspace("ws-test-never-launched"), None);
    }
}
