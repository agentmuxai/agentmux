// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `app.disk.report`: what is filling this machine's disk that AgentMux made (spec §4.3), a read.
//! The free space of the volumes AgentMux uses, the largest build folders (`target`) in the agents'
//! workspaces with the agent that owns each, and the folders of older versions in this channel.
//! Paths are relative to the AgentMux home or the agents' folder; nothing outside them is named.
//!
//! Measuring a large `target` takes long, so the walk has a time budget and says when it stopped
//! early (`complete: false`): the sizes it gives are then lower bounds. It deletes nothing.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::{AppState, RemoteCaller, RemoteRefusal, RemoteResult};
use crate::backend::build_cache as bc;

/// How long the measuring may take, inside the transport's 25 s limit.
const BUDGET: Duration = Duration::from_secs(18);
/// How many build folders and old versions are listed (largest first).
const LISTED: usize = 20;

/// The size of everything under `p`, stopping at `deadline`: (bytes, whether the whole tree was counted).
fn dir_size_until(p: &Path, deadline: Instant) -> (u64, bool) {
    let mut total = 0u64;
    let mut stack = vec![p.to_path_buf()];
    let mut seen = 0u32;
    while let Some(d) = stack.pop() {
        seen += 1;
        if seen % 256 == 0 && Instant::now() >= deadline {
            return (total, false);
        }
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            match e.file_type() {
                Ok(t) if t.is_dir() => stack.push(e.path()),
                Ok(_) => total += e.metadata().map_or(0, |m| m.len()),
                Err(_) => {}
            }
        }
    }
    (total, true)
}

/// A relative path with forward slashes, or just the last name when `p` is not under `base`.
fn relative(p: &Path, base: &Path) -> String {
    let shown = p.strip_prefix(base).unwrap_or_else(|_| Path::new(p.file_name().unwrap_or_default()));
    shown.to_string_lossy().replace('\\', "/")
}

fn volume(path: &Path) -> Value {
    match bc::disk_of(path) {
        Some((free, total)) => json!({ "free_bytes": free, "total_bytes": total }),
        None => Value::Null,
    }
}

/// The `target` folders under `agents_root`, one entry per cargo clone, largest first.
fn build_folders(agents_root: &Path, deadline: Instant) -> (Vec<Value>, u64, bool) {
    let mut found: Vec<(u64, Value)> = Vec::new();
    let mut complete = true;
    let mut total = 0u64;
    let workspaces: Vec<PathBuf> = std::fs::read_dir(agents_root)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.path())
        .collect();
    for workspace in workspaces {
        for clone in bc::clones_in(&workspace) {
            if Instant::now() >= deadline {
                complete = false;
                break;
            }
            let target = clone.join("target");
            let (bytes, whole) = dir_size_until(&target, deadline);
            let incremental: u64 = bc::incremental_roots(&target).iter().map(|r| dir_size_until(r, deadline).0).sum();
            complete &= whole;
            total += bytes;
            found.push((
                bytes,
                json!({
                    "clone": relative(&clone, agents_root),
                    "target_bytes": bytes,
                    "incremental_bytes": incremental,
                    "complete": whole,
                }),
            ));
        }
    }
    found.sort_by(|a, b| b.0.cmp(&a.0));
    (found.into_iter().take(LISTED).map(|(_, v)| v).collect(), total, complete)
}

/// The other version folders of `channel` (`channels/<channel>/versions/<version>`), largest first.
fn old_versions(home: &Path, channel: &str, live: &str, deadline: Instant) -> Vec<Value> {
    let versions = home.join("channels").join(channel).join("versions");
    let mut found: Vec<(u64, Value)> = std::fs::read_dir(&versions)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()) && e.file_name().to_string_lossy() != live)
        .map(|e| {
            let (bytes, complete) = dir_size_until(&e.path(), deadline);
            (bytes, json!({ "version": e.file_name().to_string_lossy(), "bytes": bytes, "complete": complete }))
        })
        .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0));
    found.into_iter().take(LISTED).map(|(_, v)| v).collect()
}

fn report(live_version: &str) -> Value {
    let deadline = Instant::now() + BUDGET;
    let paths = agentmux_common::data_paths::DataPaths::from_env();
    let home = agentmux_common::data_paths::agentmux_root().ok();
    let agents_root = crate::backend::build_cache_service::agents_root();
    let (build, build_total, build_complete) = match &agents_root {
        Some(root) => build_folders(root, deadline),
        None => (Vec::new(), 0, true),
    };
    let versions = match (&home, &paths) {
        (Some(home), Some(p)) => old_versions(home, &p.channel, live_version, deadline),
        _ => Vec::new(),
    };
    json!({
        "data_volume": paths.as_ref().map_or(Value::Null, |p| volume(&p.data_dir)),
        "agents_volume": agents_root.as_deref().map_or(Value::Null, volume),
        "build_folders": build,
        "build_folders_total_bytes": build_total,
        "older_versions": versions,
        "complete": build_complete && Instant::now() < deadline,
        "note": "Build folders (cargo target) are rebuilt on demand; the hourly pass already prunes orphaned incremental folders. Older version folders are removed by a person, after checking nothing runs from them.",
    })
}

pub(super) async fn disk_report(state: &AppState, _caller: &RemoteCaller, _params: Value) -> RemoteResult {
    let version = state.version.clone();
    tokio::task::spawn_blocking(move || report(&version))
        .await
        .map_err(|e| RemoteRefusal::failed(format!("measuring the disk: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tree_is_counted_and_a_passed_deadline_stops_the_walk() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("a/b")).unwrap();
        std::fs::write(dir.path().join("a/one"), vec![0u8; 1000]).unwrap();
        std::fs::write(dir.path().join("a/b/two"), vec![0u8; 500]).unwrap();
        assert_eq!(dir_size_until(dir.path(), Instant::now() + Duration::from_secs(10)), (1500, true));
        // Past its deadline a big tree stops early (checked every 256 folders).
        for i in 0..300 {
            std::fs::create_dir_all(dir.path().join(format!("many/{i}"))).unwrap();
        }
        let (_, whole) = dir_size_until(dir.path(), Instant::now() - Duration::from_secs(1));
        assert!(!whole);
    }

    #[test]
    fn paths_are_relative_to_their_base_and_never_absolute() {
        let base = Path::new("/home/u/.agentmux/agents");
        assert_eq!(relative(&base.join("korp/agentmux"), base), "korp/agentmux");
        assert_eq!(relative(Path::new("/elsewhere/clone"), base), "clone", "outside the base only the last name is given");
    }

    #[test]
    fn older_versions_leave_out_the_live_one_and_are_largest_first() {
        let home = tempfile::tempdir().unwrap();
        let versions = home.path().join("channels/stable/versions");
        for (v, size) in [("0.60.7", 100usize), ("0.60.8", 5000), ("0.60.9", 40)] {
            std::fs::create_dir_all(versions.join(v)).unwrap();
            std::fs::write(versions.join(v).join("f"), vec![0u8; size]).unwrap();
        }
        let listed = old_versions(home.path(), "stable", "0.60.9", Instant::now() + Duration::from_secs(10));
        let names: Vec<&str> = listed.iter().filter_map(|v| v["version"].as_str()).collect();
        assert_eq!(names, ["0.60.8", "0.60.7"]);
        assert_eq!(listed[0]["bytes"], 5000);
    }
}
