// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A sign-in value the CLI has just refreshed is hidden from transcripts within moments of the write, not at the
//! next account tick.
//!
//! The values in an account's sign-in file are withheld where srv reads the file (`oauth_probe`), which the spawn
//! check and the 15 s account tick do. A CLI renews its tokens in between, so [`start`] watches each account's
//! sign-in file (Claude's `.credentials.json`, Codex's and Grok's `auth.json`, Muse's `muse/auth.json`) through the
//! shared [`FsWatchPool`] and reads the file again when it changes, whether the CLI writes it in place or replaces it
//! (a temp file renamed over it). The accounts followed are read again every [`RECONCILE`], so an account that is added
//! or deleted is followed or let go. It only reads: it never writes a sign-in file or logs a value.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::broadcast::error::{RecvError, TryRecvError};

use crate::backend::fs_watch::{FsWatchEventKind, FsWatchPool, Subscription};
use crate::backend::providers::resolve_provider_alias;
use crate::backend::storage::store::{SecretRef, Store};
use crate::identity::resolver::{probe_oauth_status, sign_in_file};

/// How often the accounts to follow are read again.
const RECONCILE: Duration = Duration::from_secs(15);

/// How long a burst of events (a file written in pieces, or replaced) settles before the file is read.
const SETTLE: Duration = Duration::from_millis(150);

/// A sign-in file followed, and what it is.
struct Followed {
    provider: String,
    dir: String,
    subscription: Subscription,
}

/// `path` as the pool reports it: the directory made canonical (the pool watches the canonical directory).
fn normal(path: &Path) -> PathBuf {
    let canonical_parent = path.parent().and_then(|p| p.canonicalize().ok());
    match (canonical_parent, path.file_name()) {
        (Some(parent), Some(name)) => parent.join(name),
        _ => path.to_path_buf(),
    }
}

/// The sign-in files of every account that keeps its sign-in in a config dir: file -> (provider, dir).
fn wanted(accounts: &Store) -> HashMap<PathBuf, (String, String)> {
    let mut out = HashMap::new();
    for account in accounts.identity_list(None).unwrap_or_default() {
        let SecretRef::OAuthConfigDir { dir } = &account.secret_ref else { continue };
        let provider = resolve_provider_alias(&account.provider).to_string();
        if let Some(file) = sign_in_file(&provider, Path::new(dir)) {
            out.insert(normal(&file), (provider, dir.clone()));
        }
    }
    out
}

/// Follows the files in `wanted` and lets go of the ones no longer in it.
fn reconcile(pool: &Arc<FsWatchPool>, wanted: HashMap<PathBuf, (String, String)>, followed: &mut HashMap<PathBuf, Followed>) {
    let gone: Vec<PathBuf> = followed.keys().filter(|file| !wanted.contains_key(*file)).cloned().collect();
    for file in gone {
        if let Some(f) = followed.remove(&file) {
            pool.unsubscribe(f.subscription);
        }
    }
    for (file, (provider, dir)) in wanted {
        followed.entry(file.clone()).or_insert_with(|| Followed { subscription: pool.subscribe_file(&file), provider, dir });
    }
}

/// Reads each of `files` once, which withholds what is in it.
async fn look(files: HashSet<PathBuf>, followed: &HashMap<PathBuf, Followed>) {
    let looks: Vec<(String, String)> = files.iter().filter_map(|f| followed.get(f)).map(|f| (f.provider.clone(), f.dir.clone())).collect();
    let _ = tokio::task::spawn_blocking(move || {
        let now = agentmux_common::time::now_ms();
        for (provider, dir) in looks {
            probe_oauth_status(&provider, &dir, now);
        }
    })
    .await;
}

async fn run(pool: Arc<FsWatchPool>, accounts: Arc<Store>) {
    // events() before the first subscription, or a change landing between them is missed.
    let mut events = pool.events();
    let mut followed: HashMap<PathBuf, Followed> = HashMap::new();
    reconcile(&pool, wanted(&accounts), &mut followed);
    let mut tick = tokio::time::interval(RECONCILE);
    tick.tick().await;
    loop {
        let mut changed: HashSet<PathBuf> = HashSet::new();
        tokio::select! {
            _ = tick.tick() => {
                let accounts = accounts.clone();
                if let Ok(now_wanted) = tokio::task::spawn_blocking(move || wanted(&accounts)).await {
                    reconcile(&pool, now_wanted, &mut followed);
                }
                continue;
            }
            event = events.recv() => match event {
                Ok(e) if matches!(e.kind, FsWatchEventKind::Created | FsWatchEventKind::Modified | FsWatchEventKind::Other) => {
                    changed.insert(normal(&e.path));
                }
                Ok(_) => continue,
                Err(RecvError::Lagged(_)) => changed.extend(followed.keys().cloned()),
                Err(RecvError::Closed) => break,
            },
        }
        // A burst settles, then each file that changed is read once.
        tokio::time::sleep(SETTLE).await;
        loop {
            match events.try_recv() {
                Ok(e) => {
                    changed.insert(normal(&e.path));
                }
                Err(TryRecvError::Lagged(_)) => changed.extend(followed.keys().cloned()),
                Err(_) => break,
            }
        }
        changed.retain(|file| followed.contains_key(file));
        if !changed.is_empty() {
            look(changed, &followed).await;
        }
    }
}

/// Starts following the sign-in files of the accounts in `accounts`. Needs a running tokio runtime.
pub fn start(pool: Arc<FsWatchPool>, accounts: Arc<Store>) {
    tokio::spawn(run(pool, accounts));
}

#[cfg(test)]
#[path = "sign_in_watch_tests.rs"]
mod tests;
