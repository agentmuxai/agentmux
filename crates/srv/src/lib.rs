// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The server's core: everything that does not name the server layer (`server`, `sagas`, `drone`,
//! `messaging`, `bootstrap`, `config`, `headless`, `open_cli`). It is its own compile unit, so the
//! binary's tests and the core's tests are type-checked apart; the binary reaches these modules
//! through the `use agentmux_srv::...` lines at the top of `main.rs`.

// A build that ships never has the test-only feature. Tests run in the dev profile, which keeps
// debug assertions on; a release-profile build with the feature on does not compile.
#[cfg(all(feature = "test-support", not(test), not(debug_assertions)))]
compile_error!("the `test-support` feature is for test builds only; a release build must not enable it");

pub mod adoption;
pub mod agents;
pub mod ambient;
pub mod backend;
pub mod boot_timing;
pub mod broker;
pub mod connectivity;
pub mod event_log;
pub mod http_client;
pub mod identity;
pub mod migrations;
pub mod muxbus;
pub mod persist;
pub mod persist_subscriber;
pub mod reducer;
pub mod registry;
pub mod srv_info;
pub mod state;
pub mod util;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

#[cfg(any(test, feature = "test-support"))]
mod tls_test_init;
