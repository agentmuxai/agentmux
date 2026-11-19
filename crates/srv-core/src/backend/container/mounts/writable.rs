// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The host folders a container agent can write: what its pane binds writable into its container, each with
//! the place it has there. Read from the pane's own settings, the values its container is made from, so it
//! holds before the container exists and while the runtime is down.

use std::path::{Path, PathBuf};

use crate::backend::blockcontroller::META_KEY_CMD_CWD;
use crate::backend::obj::{meta_get_string, MetaMapType};

/// One host folder bound writable into a container, and where it is there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WritableBind {
    pub host: PathBuf,
    /// The absolute path of the folder inside the container.
    pub target: String,
}

/// The folders the pane `meta` binds writable into its agent's container: its workspace, `memory` (the host
/// folder of the agent's Personal Memory, when it has one) and every volume of its list that is a host
/// folder not bound read-only. Nothing for a pane whose agent does not run in a container.
pub fn writable_binds(meta: &MetaMapType, memory: Option<&Path>) -> Vec<WritableBind> {
    if !crate::backend::runtime_adapter::RuntimeKind::of_block_meta(meta).is_container() {
        return Vec::new();
    }
    let mut binds = Vec::new();
    let workspace = meta_get_string(meta, META_KEY_CMD_CWD, "");
    if !workspace.trim().is_empty() {
        binds.push(WritableBind {
            host: crate::backend::base::expand_home_dir_safe(workspace.trim()),
            target: super::CONTAINER_WORKSPACE_DIR.to_string(),
        });
    }
    if let Some(memory) = memory {
        binds.push(WritableBind { host: memory.to_path_buf(), target: super::container_memory_dir() });
    }
    let volumes: Vec<String> = serde_json::from_str(&meta_get_string(meta, "agent:container_volumes", "[]")).unwrap_or_default();
    for spec in &volumes {
        let Some((source, target, read_only)) = super::super::parse_volume_spec(spec) else { continue };
        if read_only || target.is_empty() || !is_host_path(source) {
            continue;
        }
        binds.push(WritableBind { host: crate::backend::base::expand_home_dir_safe(source), target: target.to_string() });
    }
    binds
}

/// Whether a volume's source names a host folder (a bind) and not a named volume: the rule a new
/// container's mounts are made by.
fn is_host_path(source: &str) -> bool {
    source.starts_with('/') || source.starts_with('~') || (source.len() >= 2 && source.as_bytes()[1] == b':')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane(mode: &str, cwd: &str, volumes: &str) -> MetaMapType {
        let mut meta = MetaMapType::new();
        meta.insert("agentMode".to_string(), serde_json::json!(mode));
        meta.insert(META_KEY_CMD_CWD.to_string(), serde_json::json!(cwd));
        meta.insert("agent:container_volumes".to_string(), serde_json::json!(volumes));
        meta
    }

    #[test]
    fn a_container_agents_pane_binds_its_workspace_its_memory_and_its_writable_host_volumes() {
        let volumes = r#"["/data/shared:/data","/data/docs:/docs:ro","cache:/cache","C:\\repos\\x:/x","broken"]"#;
        let binds = writable_binds(&pane("container", "/w/aria", volumes), Some(Path::new("/claude/projects/-w-aria/memory")));
        let got: Vec<(String, &str)> = binds.iter().map(|b| (b.host.to_string_lossy().replace('\\', "/"), b.target.as_str())).collect();
        assert_eq!(
            got,
            vec![
                ("/w/aria".to_string(), "/workspace"),
                ("/claude/projects/-w-aria/memory".to_string(), "/home/agent/.claude/projects/-workspace/memory"),
                ("/data/shared".to_string(), "/data"),
                ("C:/repos/x".to_string(), "/x"),
            ],
            "a read-only volume, a named volume and a spec with no target are not folders the agent can write"
        );
    }

    #[test]
    fn a_pane_whose_agent_runs_on_the_host_binds_nothing() {
        assert!(writable_binds(&pane("host", "/w/host", r#"["/data:/data"]"#), Some(Path::new("/m"))).is_empty());
        assert!(writable_binds(&pane("container", " ", "[]"), None).is_empty(), "no folder is bound");
    }
}
