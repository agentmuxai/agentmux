// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Messages agents send each other through the Claude CLI's own messaging
//! (its `SendMessage` tool between sessions), seen from both ends and written
//! to the audit log next to the jekts.
//!
//! The receiving CLI starts a turn on its own and writes nothing about the
//! message to the stream srv reads, unless it is launched with
//! `--replay-user-messages`. With it, the message arrives as a `user` frame
//! carrying `origin.kind == "peer"`, and the same flag replays every user turn
//! srv writes, as `isReplay` frames the pane already records itself.
//! [`reader::CliNativeReader`] therefore takes those two kinds of frame out of
//! the stream the pane sees, and audits the peer ones. The CLI's own `isReplay`
//! frames (a local command's output) are none of those and stay ([`sent`]).
//!
//! Who sent a message is decided here, from the process id the CLI verified for
//! the sender's connection ([`sender`]), never from the name the message gives.

pub mod frame;
pub mod live;
pub mod pids;
pub mod reader;
pub mod sender;
pub mod sent;
