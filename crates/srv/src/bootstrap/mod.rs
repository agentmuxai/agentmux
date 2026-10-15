// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Server bootstrap sequence, extracted from `main()`.
//!
//! Each function here corresponds to one (or a small cluster of) the
//! numbered phases that used to live inline in `async fn main()`:
//! crash-monitor branch -> parent-process watchdog -> logging init ->
//! PATH enrichment -> CLI/config parsing -> data-dir + migration setup ->
//! DB/store opening -> event bus + subagent watcher + background task
//! spawns -> TCP listener binds -> router/AppState build -> ESTART stderr
//! emission -> stdin-watch thread -> SIGINT/SIGTERM handler.
//!
//! This module is purely a relocation of that logic into named functions;
//! `main()` calls them in the same order with the same effect.

use std::sync::Arc;

use clap::Parser;
use tokio::net::TcpListener;

use crate::backend;
use crate::backend::eventbus::EventBus;
use crate::backend::reactive::{self, Poller, PollerConfig};
use crate::backend::storage::filestore::FileStore;
use crate::backend::storage::migrations::OBJECT_SCHEMA_VERSION;
use crate::backend::storage::snapshot::maybe_snapshot_pre_migration;
use crate::backend::storage::store::Store;
use crate::backend::mps::Broker;
use crate::backend::wconfig;
use crate::backend::{base, docsite, sysinfo, wcore};
use crate::config::{self, CliArgs};
use crate::event_log;
use crate::messaging;
use crate::migrations;
use crate::persist;
use crate::persist_subscriber;
use crate::registry;
use crate::server::{self, AppState};
use crate::state;

/// `eprintln!` that can't panic. srv's stderr is a pipe the launcher reads, so
/// once the launcher is gone a write fails with EPIPE (Rust ignores SIGPIPE),
/// and `eprintln!` panics on a failed write. On the paths that must run when
/// the launcher has died, the parent and stdin watchers, that panic killed
/// the watcher thread before it could shut srv down, which left srv running
/// with no launcher, holding `srv.lock`, so the app could not be opened again.
macro_rules! eprintln_lossy {
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        let _ = writeln!(std::io::stderr(), $($arg)*);
    }};
}

mod watchers;
mod logging;
mod stores;
mod carry_forward;
mod background;
mod network;
mod plumbing;
mod shutdown;
mod delivery;

pub use watchers::*;
pub use logging::*;
pub use stores::*;
pub use background::*;
pub use network::*;
pub use plumbing::*;
pub use shutdown::*;
pub use delivery::*;
