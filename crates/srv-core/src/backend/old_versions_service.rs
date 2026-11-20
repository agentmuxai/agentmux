// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Runs the old-version cleanup (`old_versions.rs`) once a day.
//!
//! Every srv looks at its own channel's version folders and logs what it found. It removes them
//! only on a host whose settings say `oldversions:clean: true`; until then the plan is the report
//! (the log line, and `GET /agentmux/old-versions`). Like the build-cache cleanup it runs only in
//! an srv started by its launcher on the real home, never in a dev, headless or test instance.
//! The exception is a hosted workspace's srv: every pin move there adds a version folder to the
//! person's volume, so it removes by default (a setting of `false` still wins), two minutes after
//! start, and only when this version carried the previous one's data forward.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use crate::backend::old_versions::{self as ov, Policy};
use crate::backend::wconfig::ConfigState;

/// The first pass waits this long after start, so it never competes with startup.
const FIRST_AFTER: Duration = Duration::from_secs(20 * 60);
/// A hosted workspace runs for minutes, not days: its first pass comes sooner, once the start is long done.
const HOSTED_FIRST_AFTER: Duration = Duration::from_secs(2 * 60);
const EVERY: Duration = Duration::from_secs(24 * 3600);

/// The policy the settings ask for. `hosted`: this srv serves a hosted workspace (the one host that vouches for the
/// person's window is registered at boot), which removes by default.
pub fn policy(config: &ConfigState, hosted: bool) -> Policy {
    Policy::from_extra_for(&config.get_settings().extra, hosted)
}

/// A hosted pass is skipped when this version started without the data of the one before it, or carried forward a copy
/// that was itself empty. Then the old folders may hold the only good copy of the person's data, so none of them is
/// judged. A version that was carried forward records where from (`carried-from`), but a marker proves a copy happened,
/// not that its source held good data: a copy of an empty fresh start has one too. So the chain is followed back from the
/// running version, and every hop must hold data and lead, hop by hop, to the origin of the data (the pre-versioning copy,
/// or a version with no older data). A version that started fresh after a failed copy has no marker and older data
/// behind it, so any chain through it fails.
fn start_is_confirmed(layout: &ov::Layout) -> bool {
    use crate::migrations::carry_forward::{version_core, CARRIED_FROM_FILE, PRE_VERSIONING};
    let db_of = |name: &str| layout.versions_root.join(name).join("data").join("db");
    let has_older_data = |than: (u64, u64, u64)| {
        std::fs::read_dir(&layout.versions_root).map(|d| {
            d.filter_map(|e| e.ok()).any(|e| {
                let name = e.file_name();
                name.to_str().and_then(version_core).is_some_and(|v| v < than) && db_of(&name.to_string_lossy()).join("objects.db").is_file()
            })
        })
    };
    let Some(mut at) = version_core(&layout.running) else {
        return false;
    };
    let mut marker = std::fs::read_to_string(db_of(&layout.running).join(CARRIED_FROM_FILE));
    // Each hop goes to a strictly older version, so the walk ends; a marker naming anything else fails.
    loop {
        let Ok(text) = marker else {
            return false;
        };
        let from = text.trim();
        if from == PRE_VERSIONING {
            return true;
        }
        let Some(from_core) = version_core(from).filter(|v| *v < at) else {
            return false;
        };
        let source = db_of(from);
        // A source an earlier pass already removed ends the chain, as an unmarked origin does: that pass was confirmed,
        // and nothing older is left to lose. Older data still on disk behind a gone source cannot be vouched for.
        if !source.join("objects.db").is_file() {
            return has_older_data(from_core).is_ok_and(|older| !older);
        }
        marker = std::fs::read_to_string(source.join(CARRIED_FROM_FILE));
        if marker.is_err() {
            return has_older_data(from_core).is_ok_and(|older| !older);
        }
        at = from_core;
    }
}

fn pass(config: &ConfigState, hosted: bool) {
    let Some(layout) = ov::layout_from_env() else {
        tracing::debug!("old versions: this srv has no per-version folders");
        return;
    };
    let policy = policy(config, hosted);
    if hosted && policy.clean && !start_is_confirmed(&layout) {
        tracing::info!("old versions: this version did not carry its data forward; nothing is removed");
        return;
    }
    let run = ov::run(&layout, &policy, !policy.clean, None, SystemTime::now());
    if !run.scan_ok {
        tracing::warn!("old versions: the running processes could not be read; nothing is removed");
        return;
    }
    let plan = &run.plan;
    match &run.report {
        Some(report) => {
            for (dir, bytes) in &report.removed {
                tracing::info!(dir = %dir.display(), freed = %crate::backend::build_cache::human(*bytes), "old versions: removed");
            }
            for (dir, why) in &report.skipped {
                tracing::info!(dir = %dir.display(), why = %why, "old versions: left alone");
            }
        }
        None if !plan.removable.is_empty() => {
            for c in &plan.removable {
                tracing::info!(dir = %c.dir.display(), kind = ?c.kind, size = %crate::backend::build_cache::human(c.bytes), "old versions: could be removed");
            }
            tracing::info!(
                folders = plan.removable.len(),
                reclaimable = %crate::backend::build_cache::human(plan.reclaimable_bytes()),
                "old versions: a dry run (set oldversions:clean to remove them)"
            );
        }
        None => {}
    }
}

/// Starts the schedule. Safe to call once at boot.
pub fn install(config: Arc<ConfigState>, hosted: bool) {
    // A hosted srv is headless, which `this_srv_may_clean` refuses: it has its own home, the person's volume.
    if !hosted && !crate::backend::build_cache_service::this_srv_may_clean() {
        tracing::debug!("old versions: not an srv that cleans");
        return;
    }
    tokio::spawn(async move {
        tokio::time::sleep(if hosted { HOSTED_FIRST_AFTER } else { FIRST_AFTER }).await;
        let mut tick = tokio::time::interval(EVERY);
        loop {
            tick.tick().await;
            let cfg = config.clone();
            let _ = tokio::task::spawn_blocking(move || pass(&cfg, hosted)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(root: &std::path::Path) -> ov::Layout {
        ov::Layout { versions_root: root.join("versions"), running: "0.60.35".into(), channel: "chan".into(), portable_parents: Vec::new() }
    }

    fn db_dir(l: &ov::Layout) -> std::path::PathBuf {
        let dir = l.versions_root.join(&l.running).join("data").join("db");
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A version folder named `name` with an `objects.db`, and `marker` as its `carried-from` when given.
    fn version_with_data(l: &ov::Layout, name: &str, marker: Option<&str>) {
        let dir = l.versions_root.join(name).join("data").join("db");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("objects.db"), "").unwrap();
        if let Some(m) = marker {
            std::fs::write(dir.join(crate::migrations::carry_forward::CARRIED_FROM_FILE), m).unwrap();
        }
    }

    fn carried_from_old(root: &std::path::Path) -> ov::Layout {
        let l = layout(root);
        std::fs::write(db_dir(&l).join(crate::migrations::carry_forward::CARRIED_FROM_FILE), "0.60.34\n").unwrap();
        l
    }

    #[test]
    fn a_start_that_carried_forward_the_origin_of_the_data_is_confirmed() {
        let root = tempfile::tempdir().unwrap();
        let l = carried_from_old(root.path());
        version_with_data(&l, "0.60.34", None);
        assert!(start_is_confirmed(&l), "the source is the oldest version with data");
    }

    #[test]
    fn a_start_that_carried_forward_a_version_that_was_carried_itself_is_confirmed() {
        let root = tempfile::tempdir().unwrap();
        let l = carried_from_old(root.path());
        version_with_data(&l, "0.60.33", None);
        version_with_data(&l, "0.60.34", Some("0.60.33"));
        assert!(start_is_confirmed(&l));
    }

    #[test]
    fn a_start_from_the_pre_versioning_copy_is_confirmed() {
        let root = tempfile::tempdir().unwrap();
        let l = layout(root.path());
        std::fs::write(db_dir(&l).join(crate::migrations::carry_forward::CARRIED_FROM_FILE), crate::migrations::carry_forward::PRE_VERSIONING).unwrap();
        assert!(start_is_confirmed(&l));
    }

    /// A marker proves a copy, not good data: 0.60.35 copied the empty 0.60.34 and so has a marker, and 0.60.36 copied that.
    /// The chain back from 36 passes 35 (marked) but ends at 34, which has no marker and older data behind it.
    #[test]
    fn a_chain_through_a_copy_of_an_empty_fresh_start_is_not_confirmed() {
        let root = tempfile::tempdir().unwrap();
        let l = layout(root.path());
        version_with_data(&l, "0.60.33", None);
        version_with_data(&l, "0.60.34", None);
        version_with_data(&l, "0.60.35", Some("0.60.34"));
        std::fs::write(db_dir(&l).join(crate::migrations::carry_forward::CARRIED_FROM_FILE), "0.60.35").unwrap();
        assert!(!start_is_confirmed(&l));
    }

    #[test]
    fn a_marker_that_does_not_name_an_older_version_is_not_confirmed() {
        let root = tempfile::tempdir().unwrap();
        let l = carried_from_old(root.path());
        version_with_data(&l, "0.60.34", Some("0.60.34"));
        assert!(!start_is_confirmed(&l), "a loop back to itself");
    }

    /// The data-loss case: 0.60.34 started fresh after a failed copy (empty databases, no record), and this version then
    /// copied that empty copy. The pass would keep it and the running one and remove the only good copy, 0.60.33.
    #[test]
    fn a_start_that_copied_an_empty_fresh_start_is_not_confirmed() {
        let root = tempfile::tempdir().unwrap();
        let l = carried_from_old(root.path());
        version_with_data(&l, "0.60.33", None);
        version_with_data(&l, "0.60.34", None);
        assert!(!start_is_confirmed(&l));
    }

    /// After a pass: 0.60.33 (the origin) was removed, 0.60.34 still names it, and the running 0.60.35 carried 0.60.34.
    /// Every later start must still be confirmed, or the cleanup never runs twice.
    #[test]
    fn a_chain_whose_origin_an_earlier_pass_removed_is_still_confirmed() {
        let root = tempfile::tempdir().unwrap();
        let l = carried_from_old(root.path());
        version_with_data(&l, "0.60.34", Some("0.60.33"));
        assert!(start_is_confirmed(&l));
    }

    /// A source that is gone with older data still on disk behind it is not vouched for.
    #[test]
    fn a_source_folder_that_is_gone_with_older_data_behind_it_is_not_confirmed() {
        let root = tempfile::tempdir().unwrap();
        let l = carried_from_old(root.path());
        version_with_data(&l, "0.60.33", None);
        assert!(!start_is_confirmed(&l), "0.60.34 is gone, 0.60.33 is still there");
    }

    /// A failed copy leaves the new version to start with empty databases: the old folders may hold the only good copy.
    #[test]
    fn a_start_with_no_record_of_a_carry_forward_is_not_confirmed_so_nothing_is_judged() {
        let root = tempfile::tempdir().unwrap();
        let l = layout(root.path());
        db_dir(&l);
        assert!(!start_is_confirmed(&l), "a database folder, but no record of where its data came from");
        assert!(!start_is_confirmed(&layout(&root.path().join("elsewhere"))), "no folder at all");
    }
}
