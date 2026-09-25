// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Unit tests for `PersistentSubprocessController`, one file per former
//! inline `mod *_tests` block. Each file opens with `use super::super::*;`
//! — the controller module itself — so it sees exactly what the inline
//! module used to see via `use super::*;`, private items included.

mod continuation;
mod fresh_start_disclosure;
mod muxbus_registration;
mod classify_exit_line;
mod send_input;
mod resume_poison;
mod agent_id;
mod shutdown;
mod reopen_guard;
mod eager_resume;
mod turn_boundary;
mod single_live_instance;
