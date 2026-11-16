// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The in-app updater: the check for a newer release, its download and signature, and the install, unpack and restart.

pub mod auto;
pub mod check;
pub mod download;
pub mod install;
pub mod restart;
pub mod signature;
pub mod unpack;

/// Get the updater ready and start its background check (`check::init`, `auto::spawn`).
pub fn init(event_bus: std::sync::Arc<crate::backend::eventbus::EventBus>, config: std::sync::Arc<crate::backend::wconfig::ConfigState>) {
    check::init(event_bus);
    auto::spawn(config);
}
