// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Unit tests for `PersistentSubprocessController`, one file per former
//! inline `mod *_tests` block. Each file opens with `use super::super::*;`
//! — the controller module itself — so it sees exactly what the inline
//! module used to see via `use super::*;`, private items included.

pub mod continuation;
pub mod fresh_start_disclosure;
pub mod muxbus_registration;
pub mod classify_exit_line;
pub mod send_input;
pub mod resume_poison;
pub mod agent_id;
pub mod shutdown;
pub mod loop_stop;
pub mod cap_interrupt;
pub mod reopen_guard;
pub mod eager_resume;
pub mod immediate_delivery;
pub mod single_live_instance;
pub mod resume_gate;
pub mod cli_native;
pub mod turn_ledger;
pub mod login_failure;
pub mod system_notes;
pub mod message_origin;
pub mod spawn_process;
