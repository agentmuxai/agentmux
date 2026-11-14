// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! ShellController: manages lifecycle of shell and command blocks.
//! Port of Go's pkg/blockcontroller/shellcontroller.go.
//!
//! State machine:
//!   INIT ─(start)─> RUNNING ─(exit/stop)─> DONE
//!   DONE ─(resync+force)─> RUNNING
//!
//! I/O model (3 async tasks when running):
//! 1. PTY read loop: process stdout → FileStore + MPS event
//! 2. Input loop: input channel → process stdin
//! 3. Wait loop: monitor process exit, update status
//!
//! ## Module layout
//!
//! This was one 2,394-line file; it is now a submodule directory (pure
//! reorganization, no logic changes). The `impl Controller for ShellController`
//! block is kept whole in [`lifecycle`] — a trait impl cannot be split across
//! files — and it calls the free-function helpers extracted into [`pty`],
//! [`file_ops`], [`indexing`], and [`translation`]. The struct definitions,
//! constructor, and small accessor/meta helpers live in [`controller`].
//!
//! The `pub use` re-exports below preserve the pre-split flat paths
//! (`shell::<item>`) that callers elsewhere in the crate already import, so the
//! reorganization changed no call site.

pub mod agent_identity;
pub mod container_pane;
pub mod controller;
pub mod file_ops;
pub mod indexing;
pub mod lifecycle;
pub mod post_spawn;
pub mod pty;
pub mod pty_tasks;
pub mod spawn_command;
pub mod spawn_cwd;
pub mod ssh_pane;
pub mod stamps;
pub mod start_io;
pub mod start_steps;
pub mod term_cap;
pub mod translation;
pub mod wait_task;

// Flat re-exports preserving the pre-split `shell::<item>` call sites used
// elsewhere in the crate (blockcontroller/mod.rs, acp, persistent, subprocess,
// watchdog, agent_handlers/input, blockfile, app_api).
pub use controller::ShellController;
pub use file_ops::handle_append_block_file;
pub use indexing::{extend_output_idx, lines_before_byte, output_index, output_now, read_via_index, read_via_stale_index};
// A full rebuild from byte 0: the readers extend instead (`extend_output_idx`
// falls back to one itself), so only tests call it directly.
#[cfg(any(test, feature = "test-support"))]
pub use indexing::rebuild_output_idx;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_transcript;
