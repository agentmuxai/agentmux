// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Tower, the read-only task manager pane: what each process is, its CPU and memory,
//! this machine's panes, and the other computers it can show.

pub mod agentmux;
pub mod kind;
pub mod memory;
pub mod memory_ledger;
pub mod panes;
pub mod peers;
pub mod remote;
pub mod sampler;
