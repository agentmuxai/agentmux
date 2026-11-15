// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The identity variables in an agent's spawn environment. The server builds the
//! whole environment (`server/agent_handlers/input.rs`); the process launchers
//! under `blockcontroller` carry the identity part of it too, so these helpers
//! sit below both.

pub mod bash_prefix;
pub mod block_identity;
pub mod build;
pub mod endpoint;
pub mod former_names;
pub mod identity;
pub mod run_account;
pub mod workspaces;
