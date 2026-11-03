// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The helper programs an agent's hooks and MCP config run (`agentmux-mcp`, `agentmux-bashwrap`), written down once
//! in `config/tools.lock.json` and used by both ways an agent gets them (task #13666, step 4). A native agent finds
//! them in AgentMux's tool folders ([`find_in_tool_dirs`]); a container agent finds them where the image copies them,
//! and `docker/Dockerfile.agent-agentmux` is checked against the lock by `the_dockerfile_installs_what_the_lock_lists`,
//! so a tool added to one side without the other fails a test, not an agent's first turn.

use std::path::PathBuf;

use serde::Deserialize;

const LOCK_JSON: &str = include_str!("../config/tools.lock.json");

#[derive(Debug, Deserialize)]
pub struct ToolLock {
    pub tools: Vec<LockedTool>,
}

#[derive(Debug, Deserialize)]
pub struct LockedTool {
    pub id: String,
    /// The cargo package that builds it.
    pub package: String,
    /// Where the container image installs it.
    pub container_path: String,
    pub used_by: Vec<String>,
}

pub fn lock() -> ToolLock {
    serde_json::from_str(LOCK_JSON).expect("tools.lock.json parses")
}

/// A folder AgentMux keeps its own tools in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolDir {
    /// `<srv-dir>/tools/bin`, where a packaged build ships them.
    Bundled,
    /// `~/.agentmux/tools/bin`, where `task dev` puts them.
    User,
    /// The folder srv itself runs from.
    BesideSrv,
}

impl ToolDir {
    fn path(self) -> Option<PathBuf> {
        match self {
            ToolDir::Bundled => crate::backend::tool_store::bundled_tools_dir(),
            ToolDir::User => crate::backend::tool_store::user_tools_dir(),
            ToolDir::BesideSrv => std::env::current_exe().ok().and_then(|exe| exe.parent().map(|d| d.to_path_buf())),
        }
    }
}

/// `name` as a file in the first of `dirs` that has it, with the platform's executable suffix tried first.
pub fn find_in_tool_dirs(name: &str, dirs: &[ToolDir]) -> Option<PathBuf> {
    let names = [format!("{name}{}", std::env::consts::EXE_SUFFIX), name.to_string()];
    dirs.iter()
        .filter_map(|dir| dir.path())
        .find_map(|dir| names.iter().map(|n| dir.join(n)).find(|p| p.is_file()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    const DOCKERFILE: &str = include_str!("../../../../docker/Dockerfile.agent-agentmux");

    #[test]
    fn the_lock_names_each_tool_once_and_says_who_uses_it() {
        let lock = lock();
        let ids: BTreeSet<_> = lock.tools.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids.len(), lock.tools.len(), "a tool is listed twice");
        for t in &lock.tools {
            assert!(!t.used_by.is_empty(), "{} says nothing about who uses it", t.id);
            assert_eq!(t.container_path, format!("/usr/local/bin/{}", t.id), "{}: the image installs tools on PATH under their id", t.id);
        }
    }

    #[test]
    fn the_dockerfile_installs_what_the_lock_lists() {
        let lock = lock();
        let wanted: BTreeSet<(String, String)> = lock
            .tools
            .iter()
            .map(|t| (format!("/build/target/release/{}", t.package), t.container_path.clone()))
            .collect();
        let copied: BTreeSet<(String, String)> = DOCKERFILE
            .lines()
            .filter(|l| l.starts_with("COPY --from=mcp-builder"))
            .map(|l| {
                let mut paths = l.split_whitespace().filter(|w| w.starts_with('/'));
                (paths.next().unwrap_or_default().to_string(), paths.next().unwrap_or_default().to_string())
            })
            .collect();
        assert_eq!(copied, wanted, "docker/Dockerfile.agent-agentmux COPY lines must equal tools.lock.json");
        let built: BTreeSet<String> = DOCKERFILE
            .lines()
            .find(|l| l.starts_with("RUN cargo build --release"))
            .map(|l| l.split_whitespace().skip_while(|w| *w != "-p").filter(|w| *w != "-p").map(str::to_string).collect())
            .unwrap_or_default();
        let packages: BTreeSet<String> = lock.tools.iter().map(|t| t.package.clone()).collect();
        assert_eq!(built, packages, "the mcp-builder stage must build exactly the locked packages");
    }

    /// The first word of every command a launch's hooks run, and the command of its `agentmux` MCP server.
    fn commands_a_launch_runs() -> BTreeSet<String> {
        let files = crate::backend::agent_config::build_config_files(
            &Default::default(),
            &[],
            "Lock",
            "lock-id",
            "lock",
            "work",
            "claude",
        );
        let json = |name: &str| -> serde_json::Value {
            let f = files.iter().find(|f| f.filename == name).unwrap_or_else(|| panic!("no {name}"));
            serde_json::from_str(&f.content).unwrap()
        };
        let mut out = BTreeSet::new();
        let settings = json(".claude/settings.json");
        let mut stack = vec![&settings["hooks"]];
        while let Some(v) = stack.pop() {
            match v {
                serde_json::Value::Object(map) => {
                    if let Some(cmd) = map.get("command").and_then(|c| c.as_str()) {
                        out.insert(cmd.split_whitespace().next().unwrap_or_default().to_string());
                    }
                    stack.extend(map.values());
                }
                serde_json::Value::Array(items) => stack.extend(items),
                _ => {}
            }
        }
        let mcp = json(".mcp.json");
        out.insert(mcp["mcpServers"]["agentmux"]["command"].as_str().unwrap().to_string());
        out
    }

    #[test]
    fn every_tool_a_launch_runs_is_in_the_lock() {
        let ids: BTreeSet<String> = lock().tools.into_iter().map(|t| t.id).collect();
        let used = commands_a_launch_runs();
        assert!(!used.is_empty(), "the launch ran no tools: the check would pass on nothing");
        let unlocked: Vec<_> = used.difference(&ids).collect();
        assert!(unlocked.is_empty(), "add these to tools.lock.json, and to the image: {unlocked:?}");
    }

    #[test]
    fn a_tool_is_found_in_the_first_folder_that_has_it() {
        assert_eq!(find_in_tool_dirs("agentmux-no-such-tool", &[ToolDir::Bundled, ToolDir::User, ToolDir::BesideSrv]), None);
    }
}
