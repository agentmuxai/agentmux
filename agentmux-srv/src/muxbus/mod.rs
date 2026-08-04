// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

pub mod agent_credentials;
pub mod cloud_subscriber;
pub mod pkce;

/// Identifier MuxBus's single global credential set is registered under
/// with both the broker scheduler (`crate::broker`) and the OS keychain
/// (`backend::storage::muxbus`'s `secret_store` key) — deliberately the same
/// string in both places so the two are trivially correlated in logs.
pub const CREDENTIAL_ID: &str = "muxbus:global";
