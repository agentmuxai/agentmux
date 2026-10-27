// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Messages agents send each other through the Claude CLI's own messaging
//! (its `SendMessage` tool between sessions), seen from both ends and written
//! to the audit log next to the jekts.
//!
//! The receiving CLI starts a turn on its own and writes nothing about the
//! message to the stream srv reads, unless it is launched with
//! `--replay-user-messages`. With it, the message arrives as a `user` frame
//! carrying `origin.kind == "peer"`, and the same flag replays the human's own
//! typed prompts as `isReplay` user frames, which srv and the pane already
//! record themselves. [`reader::CliNativeReader`] therefore takes every
//! replayed user frame out of the stream the pane sees (as before the flag) and
//! audits the peer ones.
//!
//! Who sent a message is decided here, from the process id the CLI verified for
//! the sender's connection ([`sender`]), never from the name the message gives.

pub mod frame;
pub mod live;
pub mod pids;
pub mod reader;
pub mod sender;
