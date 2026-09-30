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

mod watchers;
mod logging;
mod stores;
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
