// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Thread-safe configuration holder with change notification.

use std::sync::{Arc, RwLock};

use super::types::{FullConfigType, SettingsType};

/// Thread-safe configuration holder with change notification.
/// The actual file system watching will be integrated with the
/// event loop in a later phase.
pub struct ConfigWatcher {
    config: RwLock<Arc<FullConfigType>>,
}

impl ConfigWatcher {
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

impl Default for ConfigWatcher {
    fn default() -> Self {
        Self::new()
    }
}
