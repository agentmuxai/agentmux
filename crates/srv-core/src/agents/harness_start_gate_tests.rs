// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A start of a program in an agent's folder that is not a harness's start must say so here.
//!
//! The folder rule (`mcp_folder_rule`) keeps a harness started on this computer from reading a folder a
//! container agent can write. This test reads the sources of srv and srv-core and finds every file that starts a
//! process and names an agent's working folder; each one either asks the rule or is listed below with the reason
//! it starts no harness. A new such file fails here, so whoever adds it reads the rule first.
//!
//! What it reaches: a file that has both a process start (a `Command` or a pty command builder) and a name for
//! an agent's folder. The pane controllers that start a harness through a shared builder are asked the rule by
//! their own tests (`mcp_folder_rule`, `agent_runtime`).

use std::path::{Path, PathBuf};

/// Files that start a process and name a folder, and are not a harness's start.
const NOT_A_HARNESS: &[(&str, &str)] = &[
    ("crates/srv/src/sagas/close_pane.rs", "the process starts are test fixtures"),
    ("crates/srv/src/server/agent_resources.rs", "the process starts are test fixtures"),
    ("crates/srv/src/server/shell_handlers.rs", "the person's own `!` command in a pane's folder: a shell, not a harness"),
];

/// What a file that starts a harness asks.
const THE_RULE: &str = "mcp_folder_rule::";

// Built in pieces: the spawn-site inventory (`crates/common/tests`) counts a line that spells a start out.
const STARTS: &[&str] = &[concat!("Command", "::new("), concat!("CommandBuilder", "::new(")];
const FOLDERS: &[&str] = &["working_directory", "cmd:cwd", "expanded_dir", "work_dir"];

fn sources(dir: &Path, into: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if path.is_dir() {
            if name != "tests" {
                sources(&path, into);
            }
        } else if name.ends_with(".rs") && !name.contains("test") {
            into.push(path);
        }
    }
}

fn relative(path: &Path, root: &Path) -> String {
    path.strip_prefix(root).unwrap_or(path).to_string_lossy().replace('\\', "/")
}

#[test]
fn every_file_that_starts_a_process_in_an_agents_folder_asks_the_folder_rule_or_says_why_not() {
    let srv_core = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = srv_core.parent().and_then(Path::parent).expect("the workspace root");
    let mut files = Vec::new();
    sources(&srv_core.join("src"), &mut files);
    sources(&root.join("crates").join("srv").join("src"), &mut files);
    assert!(files.len() > 100, "the sources were found: {}", files.len());

    let mut seen = Vec::new();
    for path in files {
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        if !(STARTS.iter().any(|s| text.contains(s)) && FOLDERS.iter().any(|f| text.contains(f))) {
            continue;
        }
        let file = relative(&path, root);
        let listed = NOT_A_HARNESS.iter().any(|(listed, _)| *listed == file);
        assert!(
            listed || text.contains(THE_RULE),
            "{file} starts a process and names an agent's folder, but does not ask `mcp_folder_rule`: ask it before the start \
             (`workflow_refusal` for a one-shot start, `turn_refusal` or `refusal` for a pane), or list the file in NOT_A_HARNESS with the reason"
        );
        seen.push(file);
    }
    assert!(seen.iter().any(|f| f == "crates/srv-core/src/agents/runner.rs"), "the workflow Agent block's start is found: {seen:?}");
    for (listed, why) in NOT_A_HARNESS {
        assert!(seen.iter().any(|f| f == listed), "{listed} ({why}) no longer starts a process in a folder: take it out of NOT_A_HARNESS");
    }
}
