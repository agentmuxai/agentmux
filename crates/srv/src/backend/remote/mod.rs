// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Remote terminals: the connection a terminal pane belongs to, and its status.
//!
//! docs/specs/SPEC_REMOTE_TERMINALS_AND_DURABLE_SESSIONS_2026_10_02.md. This is
//! P0, the groundwork: the connection model (§4) and the status registry the
//! frontend's existing connection UI reads (`ConnStatusOverlay`, the connection
//! typeahead, `conn-status.ts`). WSL terminals (P1) and SSH terminals (P2) build
//! on it; nothing here spawns a remote shell yet.

pub mod agent_access;
pub mod askpass;
pub mod conn;
pub mod helper_install;
pub mod ssh;
pub mod ssh_config;
pub mod status;
pub mod wsl;
pub mod wsl_fs;

pub use conn::ConnTarget;
