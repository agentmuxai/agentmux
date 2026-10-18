// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What srv shares with the helper: the frame protocol a durable session
//! speaks end to end, the ring its history is kept in, and the file protocol
//! of `serve --stdio` (which also answers Tower's process frames, `procs`).

pub mod frame;
pub mod fsproto;
pub mod procs;
pub mod ring;
pub mod serve;

// This test binary must hold this checkout's code, not another clone's from
// a shared target directory (`agentmux_common::checkout`).
#[cfg(test)]
agentmux_common::checkout_guard!(agentmux_procstats::CHECKOUT_DIR);

/// The folder this library was compiled in, for the checkout guard of the
/// test binaries that link it (`agentmux_common::checkout_guard!`).
#[doc(hidden)]
pub const CHECKOUT_DIR: &str = env!("CARGO_MANIFEST_DIR");
