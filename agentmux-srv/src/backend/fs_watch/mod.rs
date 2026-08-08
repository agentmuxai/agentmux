// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Shared filesystem-watcher framework.
//!
//! Before this module, three independent `notify`-based watchers
//! (`config_watcher_fs.rs`, `editor_file_watcher.rs`, `media_file_watcher.rs`)
//! each hand-rolled the same ~80 lines of plumbing — `notify` construction,
//! sync-callback-to-async bridging, refcounted watch/unwatch, and the
//! correctness-critical "watch the parent directory, not the file itself, so
//! an atomic rename-over-target save is still detected" rule. None of the
//! three (nor `subagent_watcher/`, the fourth) had any recovery if a watch
//! failed to start or silently died later — see
//! `docs/specs/SPEC_SHARED_FS_WATCHER_FRAMEWORK_2026_08_07.md` for the full
//! audit and design rationale this module implements.
//!
//! `FsWatchPool` owns exactly the shared plumbing plus the new recovery
//! layer (retry-with-backoff, a `PollWatcher` fallback, and a periodic
//! self-healing sweep). It does NOT own domain-specific debounce or event
//! payload/publish logic — callers subscribe, get a broadcast stream of raw
//! change events, and apply their own filtering/debounce on top, the same
//! way `editor_file_watcher.rs`/`media_file_watcher.rs` already do internally
//! today (that part is genuinely domain-specific: editor wants per-path
//! debounce, media wants per-directory+extension filtering).
//!
//! This is a pure addition in its first PR — no existing watcher has been
//! migrated onto it yet. See the spec's §5 migration path.

mod pool;
mod recovery;

pub use pool::{FsWatchEvent, FsWatchEventKind, FsWatchPool, Subscription};
pub use recovery::{FsWatchHealth, WatchBackend};
