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
//! 1. PTY read loop: process stdout → FileStore + WPS event
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

mod controller;
mod file_ops;
mod indexing;
mod lifecycle;
mod pty;
mod translation;

// Flat re-exports preserving the pre-split `shell::<item>` call sites used
// elsewhere in the crate (blockcontroller/mod.rs, acp, persistent, subprocess,
// watchdog, agent_handlers/input, blockfile, app_api).
pub use controller::ShellController;
pub use file_ops::{handle_append_block_file, persist_to_blockfile_silent};
// These are `pub(crate)` at their definition (crate-internal API), so they must
// be re-exported at the same visibility — `pub use` of a `pub(crate)` item is
// rejected (E0364).
pub(crate) use file_ops::resolve_global_output_zone;
pub(crate) use indexing::{extend_output_idx, rebuild_output_idx, OUTPUT_IDX_HEADER_LEN};

#[cfg(test)]
mod tests;
