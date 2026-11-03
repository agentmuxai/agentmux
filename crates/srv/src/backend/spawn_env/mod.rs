// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The identity variables in an agent's spawn environment. The server builds the
//! whole environment (`server/agent_handlers/input.rs`); the process launchers
//! under `blockcontroller` carry the identity part of it too, so these helpers
//! sit below both.

pub(crate) mod block_identity;
pub(crate) mod former_names;
pub(crate) mod identity;
