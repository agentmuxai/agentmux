// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A sign-in value a container's CLI refreshes into its own staged copy is hidden from transcripts within moments of
//! the write, as one in the account's own file is (`sign_in_watch`).
//!
//! A container agent holds a copy of its account's login under `<AgentMux root>/container-credentials/<container>/
//! <account>/`. [`start`] watches that tree through the shared [`FsWatchPool`], so a container that is added is
//! followed with no further step, and reads a copy again when its `.credentials.json` changes (written in place, or
//! replaced by a renamed temp file).
//!
//! A container can write its own copy, so what a copy holds is not trusted: its values are withheld under owners of
//! their own, one pair per staging folder, with the usual cap per owner (`withhold::KEPT_PER_OWNER`), and the copy's
//! `expiresAt` is not used, so a copy holds a bounded number of values and cannot push out what the account's own
//! file holds. It only reads: it never writes a copy and never logs a value.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::broadcast::error::{RecvError, TryRecvError};

use crate::backend::container::keychain_credentials::STAGING_ROOT;
use crate::backend::fs_watch::{FsWatchEventKind, FsWatchPool};
use crate::identity::resolver::oauth_probe::withhold_sign_in;

/// The file a staged login is kept in, and what a change to it is looked at as.
const COPY_FILE: &str = ".credentials.json";

/// How long a burst of events (a copy written in pieces, or replaced) settles before it is read.
const SETTLE: Duration = Duration::from_millis(150);

/// Withholds the values of `json`, the parsed copy in the container's staging folder `folder`, under owners named by
/// that folder. Whoever reads a staged copy registers what it finds through this, not under the account's owners.
pub fn withhold_copy(folder: &Path, json: &serde_json::Value) {
    // The copy's own expiry is not trusted, in either shape the parser reads: a value is withheld without one,
    // under the folder's cap.
    let mut json = json.clone();
    if let Some(top) = json.as_object_mut() {
        top.remove("expiresAt");
    }
    if let Some(oauth) = json.get_mut("claudeAiOauth").and_then(serde_json::Value::as_object_mut) {
        oauth.remove("expiresAt");
    }
    withhold_sign_in(&format!("staged:{}", folder.to_string_lossy()), &json);
}

/// Withholds the values in the staged copy at `file` (a `.credentials.json` in a container's staging folder). An
/// unreadable copy (it can be mid-write) withholds nothing; the next change reads it again.
fn look(file: &Path) {
    let Some(folder) = file.parent() else { return };
    let Ok(text) = std::fs::read_to_string(file) else { return };
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
        withhold_copy(folder, &json);
    }
}

/// Every copy under the staging tree at `tree` is read once.
fn look_at_all(tree: &Path) {
    let Ok(containers) = std::fs::read_dir(tree) else { return };
    for container in containers.flatten() {
        for account in std::fs::read_dir(container.path()).into_iter().flatten().flatten() {
            look(&account.path().join(COPY_FILE));
        }
    }
}

fn is_copy(path: &Path) -> bool {
    path.file_name().is_some_and(|name| name == COPY_FILE)
}

/// The staging tree exists (private to the user) before it is watched, so a first container's copy is seen.
fn ensure_tree(tree: &Path) {
    if std::fs::create_dir_all(tree).is_err() {
        return;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(tree, std::fs::Permissions::from_mode(0o700));
    }
}

async fn run(pool: Arc<FsWatchPool>, tree: PathBuf) {
    // events() before the subscription, or a change landing between them is missed.
    let mut events = pool.events();
    ensure_tree(&tree);
    let _subscription = pool.subscribe_dir_recursive(&tree);
    // What the copies hold now: after a restart nothing is registered yet, and a copy that does not change is
    // never announced.
    let scan_tree = tree.clone();
    let _ = tokio::task::spawn_blocking(move || look_at_all(&scan_tree)).await;
    loop {
        let mut changed: HashSet<PathBuf> = HashSet::new();
        let mut all = false;
        match events.recv().await {
            Ok(e) if is_copy(&e.path) && matches!(e.kind, FsWatchEventKind::Created | FsWatchEventKind::Modified | FsWatchEventKind::Other) => {
                changed.insert(e.path);
            }
            Ok(_) => continue,
            Err(RecvError::Lagged(_)) => all = true,
            Err(RecvError::Closed) => break,
        }
        // A burst settles, then each copy that changed is read once.
        tokio::time::sleep(SETTLE).await;
        loop {
            match events.try_recv() {
                Ok(e) if is_copy(&e.path) => {
                    changed.insert(e.path);
                }
                Ok(_) => {}
                Err(TryRecvError::Lagged(_)) => all = true,
                Err(_) => break,
            }
        }
        let tree = tree.clone();
        let _ = tokio::task::spawn_blocking(move || {
            if all {
                look_at_all(&tree);
            }
            changed.iter().for_each(|file| look(file));
        })
        .await;
    }
}

/// Starts following the staged sign-in copies under the AgentMux root. Needs a running tokio runtime.
pub fn start(pool: Arc<FsWatchPool>) {
    let Ok(root) = agentmux_common::data_paths::agentmux_root() else { return };
    start_at(pool, root.join(STAGING_ROOT));
}

/// [`start`] over the staging tree at `tree`.
fn start_at(pool: Arc<FsWatchPool>, tree: PathBuf) {
    tokio::spawn(run(pool, tree));
}

#[cfg(test)]
#[path = "staged_watch_tests.rs"]
mod tests;
