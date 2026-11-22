// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A sign-in value a container's CLI refreshes into its own staged copy is hidden from transcripts within moments of
//! the write, as one in the account's own file is (`sign_in_watch`).
//!
//! A container agent holds a copy of its account's login under `<AgentMux root>/container-credentials/<container>/
//! <account>/`, with a `meta.json` that names the account's config dir. [`start`] watches that tree through the shared
//! [`FsWatchPool`], so a container that is added is followed with no further step, and reads a copy again when its
//! `.credentials.json` changes (written in place, or replaced by a renamed temp file). What it finds is withheld under
//! the account's own owners, so the copies and the account's file share one set of held values. It only reads: it
//! never writes a copy and never logs a value.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::broadcast::error::{RecvError, TryRecvError};

use crate::backend::container::keychain_credentials::{staged_config_dir, STAGING_ROOT};
use crate::backend::fs_watch::{FsWatchEventKind, FsWatchPool};
use crate::identity::resolver::oauth_probe::withhold_sign_in;

/// The file a staged login is kept in, and what a change to it is looked at as.
const COPY_FILE: &str = ".credentials.json";

/// How long a burst of events (a copy written in pieces, or replaced) settles before it is read.
const SETTLE: Duration = Duration::from_millis(150);

/// Withholds the values in the staged copy at `file` (a `.credentials.json` in a container's staging folder), under the
/// owners of the account its `meta.json` names. Nothing is withheld when the folder names no account or the copy is
/// unreadable (it can be mid-write; the next change reads it again).
fn look(file: &Path) {
    let Some(folder) = file.parent() else { return };
    let Some(config_dir) = staged_config_dir(folder) else { return };
    let Ok(text) = std::fs::read_to_string(file) else { return };
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
        withhold_sign_in(&config_dir, &json);
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

async fn run(pool: Arc<FsWatchPool>, tree: PathBuf) {
    // events() before the subscription, or a change landing between them is missed.
    let mut events = pool.events();
    let _subscription = pool.subscribe_dir_recursive(&tree);
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
