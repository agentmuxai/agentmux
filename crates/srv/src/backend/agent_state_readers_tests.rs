// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The guard of `docs/specs/SPEC_AGENT_AVAILABILITY_EVERY_PATH_ONE_DECISION_2026_10_31.md` §4.3: a
//! path that decides what to do with an agent (deliver, hold, replay, start, renew a lease) asks the
//! availability module (`agent_status::decide`, fed by `status_of`), and does not read the agent's
//! state itself. This scans the srv sources for the direct readers; a file not listed below that
//! calls one fails, so a new path has to either use the module or be added here with its reason.

use std::path::{Path, PathBuf};

/// The direct readers of an agent's state (`backend/agent_state.rs`).
const READERS: [&str; 3] = ["current_state", "current_state_at", "unavailable_of_block"];

/// The files that call a reader, and why each may. A reason that says "feeds" is the reader the
/// module is built on; "display" shows a state and decides nothing; "busy" asks whether a turn is in
/// flight, which is not an availability.
const ALLOWED: [(&str, &str); 10] = [
    ("backend/fleet_source.rs", "display: the fleet feed's state for the Swarm, LAN and mobile"),
    ("backend/kanban.rs", "busy: the reminder loop reminds an idle agent; a limited or signed-out one reads as not idle"),
    ("backend/supervisor/watcher.rs", "feeds: the Supervisor pauses on an agent that is out of quota or signed out"),
    ("backend/worker_notes.rs", "busy: a note is not sent to a worker waiting on its user"),
    ("server/account_episodes/moves.rs", "busy: an account move waits for the turn in flight to end"),
    ("server/app_api/agent_spawn.rs", "display: the unavailable detail beside an agent in `Agents list`"),
    ("server/app_api/kanban.rs", "display: the Kanban card's holder and its unavailable detail"),
    ("server/availability_view.rs", "feeds: `availability_of_block`, the one reader of the fact"),
    ("server/jekt_hold.rs", "feeds: `holding_unavailable`, the hold's reader, which asks `verdict`"),
    ("server/work_tasks.rs", "feeds: the lease reads the fact and asks `decide` (`renews_lease`)"),
];

fn is_word_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Whether `line` calls `name(` as its own word (not `reflects_current_state()` or `x.current_state`).
fn calls(line: &str, name: &str) -> bool {
    let needle = format!("{name}(");
    line.match_indices(&needle).any(|(at, _)| {
        let before = at.checked_sub(1).map(|i| line.as_bytes()[i]);
        !before.is_some_and(|b| is_word_char(b) || b == b'.')
    })
}

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "tests") {
                continue;
            }
            sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            if !name.ends_with("tests.rs") && !name.ends_with("_tests.rs") {
                out.push(path);
            }
        }
    }
}

/// The lines of `text` that are not comments and come before its test module (`#[cfg(test)]` followed,
/// past any other attributes, by `mod`). A `#[cfg(test)]` on anything else does not end the code.
fn non_test_lines(text: &str) -> Vec<&str> {
    let lines: Vec<&str> = text.lines().collect();
    let mut code = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if line.trim() == "#[cfg(test)]" {
            let next = lines[i + 1..].iter().map(|l| l.trim()).find(|l| !l.starts_with("#["));
            if next.is_some_and(|l| l.starts_with("mod ") || l.starts_with("pub mod ")) {
                break;
            }
        }
        if !line.trim_start().starts_with("//") {
            code.push(*line);
        }
    }
    code
}

/// The files (under `src`, `/`-separated) whose non-test code calls a reader.
fn callers() -> Vec<String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    sources(&root, &mut files);
    let mut found = Vec::new();
    for file in files {
        let rel = file.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/");
        // The definitions, and this module.
        if rel == "backend/agent_state.rs" || rel.starts_with("backend/agent_status") || rel.starts_with("backend/agent_state_readers") {
            continue;
        }
        let text = std::fs::read_to_string(&file).unwrap();
        let uses = non_test_lines(&text).iter().any(|l| READERS.iter().any(|r| calls(l, r)));
        if uses {
            found.push(rel);
        }
    }
    found.sort();
    found
}

#[test]
fn only_the_listed_files_read_an_agents_state_directly() {
    let allowed: Vec<&str> = ALLOWED.iter().map(|(file, _)| *file).collect();
    let found = callers();
    let new: Vec<&String> = found.iter().filter(|f| !allowed.contains(&f.as_str())).collect();
    assert!(
        new.is_empty(),
        "{new:?} read an agent's state directly. To decide a delivery, a hold, a replay, a start or a lease, \
         use `server::availability_view::status_of` and `backend::agent_status::decide`; to show a state or \
         ask whether a turn is in flight, add the file to ALLOWED in {} with the reason.",
        file!()
    );
    let stale: Vec<&&str> = allowed.iter().filter(|f| !found.iter().any(|x| x == **f)).collect();
    assert!(stale.is_empty(), "{stale:?} no longer read an agent's state directly: remove them from ALLOWED");
}

#[test]
fn the_word_finder_matches_a_call_and_nothing_else() {
    assert!(calls("    agent_state::current_state(&m, b)", "current_state"));
    assert!(calls("let s = current_state_at(m, b, 1);", "current_state_at"));
    assert!(!calls("fn has_node_reflects_current_state() {", "current_state"));
    assert!(!calls("x.current_state(1)", "current_state"));
    assert!(!calls("current_state_at(m)", "current_state"));
}
