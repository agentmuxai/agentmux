// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Thread-safe in-memory configuration holder.

use std::sync::{Arc, RwLock};

use super::types::{FullConfigType, SettingsType};

/// Thread-safe in-memory holder for the live configuration.
///
/// **This does not watch anything.** It was called `ConfigWatcher` until
/// 2026-09-06, on the strength of a comment promising that "the actual file
/// system watching will be integrated with the event loop in a later phase" —
/// which never happened. Watching was instead built separately in
/// `backend::config_watcher_fs`, on top of the shared `fs_watch::pool`, and the
/// two have coexisted since: similar names, one module doing the thing the
/// other is named for. That cost real time while tracing the settings-reload
/// path; see
/// `docs/reports/REPORT_NETWORK_ARCHITECTURE_DRYNESS_AND_ROBUST_LAN_2026_09_06.md` §7.
///
/// Readers get a cheap `Arc` clone of the current snapshot; writers swap the
/// whole `Arc` under the lock, so no reader ever observes a half-applied
/// config.
pub struct ConfigState {
    config: RwLock<Arc<FullConfigType>>,
}

impl ConfigState {
    /// Create a new config watcher with default config.
    pub fn new() -> Self {
        Self {
            config: RwLock::new(Arc::new(FullConfigType::default())),
        }
    }

    /// Create a new config watcher with initial config.
    pub fn with_config(config: FullConfigType) -> Self {
        Self {
            config: RwLock::new(Arc::new(config)),
        }
    }

    /// Get a snapshot of the current config.
    pub fn get_full_config(&self) -> Arc<FullConfigType> {
        self.config.read().unwrap().clone()
    }

    /// Get just the settings.
    pub fn get_settings(&self) -> SettingsType {
        self.config.read().unwrap().settings.clone()
    }

    /// Update the full config (called when files change).
    #[allow(dead_code)]
    pub fn set_config(&self, config: FullConfigType) {
        let mut current = self.config.write().unwrap();
        *current = Arc::new(config);
    }

    /// Update just the settings portion.
    pub fn update_settings(&self, settings: SettingsType) {
        let mut current = self.config.write().unwrap();
        let mut new_config = (**current).clone();
        new_config.settings = settings;
        *current = Arc::new(new_config);
    }
}

impl Default for ConfigState {
    fn default() -> Self {
        Self::new()
    }
}
