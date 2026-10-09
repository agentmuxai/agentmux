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
