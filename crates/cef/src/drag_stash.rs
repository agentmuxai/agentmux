// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! In-process stash for file paths captured by `CefDragHandler::on_drag_enter`
//! and read by the `peek_drag_paths` / `consume_drag_paths` IPCs.
//!
//! Why: the HTML5 drop event in a CEF browser only surfaces bare filenames,
//! not full filesystem paths. CEF's DragData exposes the real paths, but only
//! during the OnDragEnter callback. We stash them here for the renderer.
//!
//! One entry per window label, because each window (main, tear-off, floater)
//! is its own renderer and asks only for its own drag. The IPC router has no
//! browser context, so the renderer passes its label (`?windowLabel=`) and
//! `on_drag_enter` maps its browser to the same label.
//!
//! An entry lives as long as the drag: it is replaced by the next drag that
//! enters that window (even one with zero paths — a tombstone, so a pathless
//! drag can't pick up an earlier drag's files), taken by a drop, or dropped by
//! a long backstop against leaks. There is no short TTL: a user may hover for
//! as long as they like before dropping.
//!
//! Specs: docs/specs/SPEC_PANE_FILE_DROP_2026_05_30.md §3.3,
//! SPEC_DRAG_AND_DROP_CONSOLIDATION_2026_09_27.md §5.4.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

/// Leak guard only; a drag in progress never gets near it.
const BACKSTOP: Duration = Duration::from_secs(10 * 60);

/// Key used when the host couldn't map a browser to a label (a window that is
/// still registering).
const UNLABELLED: &str = "";

struct Entry {
    at: Instant,
    paths: Vec<String>,
}

static STASH: LazyLock<Mutex<HashMap<String, Entry>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn prune(map: &mut HashMap<String, Entry>) {
    map.retain(|_, e| e.at.elapsed() <= BACKSTOP);
}

/// Record the paths of a drag that entered `label`'s window. An empty list is
/// stored too: it replaces whatever the previous drag left behind.
pub fn put(label: Option<&str>, paths: Vec<String>) {
    let mut map = STASH.lock().unwrap();
    prune(&mut map);
    let key = label.unwrap_or(UNLABELLED).to_string();
    if !key.is_empty() {
        // A labelled entry supersedes any fallback an earlier, unmapped drag left.
        map.remove(UNLABELLED);
    }
    map.insert(
        key,
        Entry {
            at: Instant::now(),
            paths,
        },
    );
}

/// The entry `take`/`peek` read for `label`: its own, else the unlabelled
/// fallback, else the most recent entry.
///
/// A drop always follows an `on_drag_enter` in the same window, so a window
/// whose host label matches its `?windowLabel=` always finds its own entry
/// and never another window's. The last fallback only matters if the two
/// labels ever disagree (or for an older frontend that sends none). It then
/// behaves exactly like the old single-slot stash, not worse.
fn key_for(map: &HashMap<String, Entry>, label: Option<&str>) -> Option<String> {
    if let Some(l) = label {
        if map.contains_key(l) {
            return Some(l.to_string());
        }
        if map.contains_key(UNLABELLED) {
            return Some(UNLABELLED.to_string());
        }
        if !map.is_empty() {
            tracing::warn!(
                label = l,
                "[drag] no stash entry for this window; using the latest drag"
            );
        }
    }
    map.iter().max_by_key(|(_, e)| e.at).map(|(k, _)| k.clone())
}

/// The paths for `label`'s current drag, consumed.
pub fn take(label: Option<&str>) -> Vec<String> {
    let mut map = STASH.lock().unwrap();
    prune(&mut map);
    key_for(&map, label)
        .and_then(|k| map.remove(&k))
        .map(|e| e.paths)
        .unwrap_or_default()
}

/// The paths for `label`'s current drag, left in place (hover-time lookups).
pub fn peek(label: Option<&str>) -> Vec<String> {
    let mut map = STASH.lock().unwrap();
    prune(&mut map);
    key_for(&map, label)
        .and_then(|k| map.get(&k))
        .map(|e| e.paths.clone())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The stash is process-global; tests share it, so serialise them.
    static SERIAL: Mutex<()> = Mutex::new(());

    fn reset() {
        STASH.lock().unwrap().clear();
    }

    fn v(xs: &[&str]) -> Vec<String> {
        xs.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn take_returns_the_windows_own_paths_once() {
        let _g = SERIAL.lock().unwrap();
        reset();
        put(Some("main"), v(&["/tmp/a", "/tmp/b"]));
        assert_eq!(take(Some("main")), v(&["/tmp/a", "/tmp/b"]));
        assert!(take(Some("main")).is_empty());
    }

    #[test]
    fn windows_dont_take_each_others_drags() {
        let _g = SERIAL.lock().unwrap();
        reset();
        put(Some("main"), v(&["/a"]));
        put(Some("floating-1"), v(&["/f"]));
        assert_eq!(take(Some("floating-1")), v(&["/f"]));
        assert_eq!(take(Some("main")), v(&["/a"]));
    }

    #[test]
    fn peek_leaves_the_entry() {
        let _g = SERIAL.lock().unwrap();
        reset();
        put(Some("main"), v(&["/a"]));
        assert_eq!(peek(Some("main")), v(&["/a"]));
        assert_eq!(peek(Some("main")), v(&["/a"]));
        assert_eq!(take(Some("main")), v(&["/a"]));
    }

    #[test]
    fn a_pathless_drag_replaces_an_earlier_one() {
        let _g = SERIAL.lock().unwrap();
        reset();
        // A path drag entered, then was cancelled; a virtual-file drag follows.
        put(Some("main"), v(&["/secret.txt"]));
        put(Some("main"), Vec::new());
        assert!(
            take(Some("main")).is_empty(),
            "the old drag's paths must not leak into this drop"
        );
    }

    #[test]
    fn unmapped_browsers_use_a_fallback_a_labelled_put_clears() {
        let _g = SERIAL.lock().unwrap();
        reset();
        put(None, v(&["/u"]));
        assert_eq!(peek(Some("window-x")), v(&["/u"]));
        put(Some("main"), v(&["/m"]));
        // The fallback went away with the labelled put; main's entry is main's.
        assert_eq!(take(Some("main")), v(&["/m"]));
        assert!(take(Some("window-x")).is_empty());
    }

    #[test]
    fn an_older_frontend_without_a_label_gets_the_latest_drag() {
        let _g = SERIAL.lock().unwrap();
        reset();
        put(Some("a"), v(&["/1"]));
        std::thread::sleep(Duration::from_millis(5));
        put(Some("b"), v(&["/2"]));
        assert_eq!(take(None), v(&["/2"]));
    }

    #[test]
    fn a_label_the_host_never_saw_behaves_like_the_old_single_slot() {
        let _g = SERIAL.lock().unwrap();
        reset();
        put(Some("host-label"), v(&["/x"]));
        assert_eq!(take(Some("url-label")), v(&["/x"]));
    }

    #[test]
    fn nothing_stashed_is_empty() {
        let _g = SERIAL.lock().unwrap();
        reset();
        assert!(take(Some("main")).is_empty());
        assert!(peek(None).is_empty());
    }
}
