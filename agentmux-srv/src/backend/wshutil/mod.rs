// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! WSH RPC transport layer.
//! Port of Go's `pkg/wshutil/` — OSC encoding, RPC proxy, event system, and I/O adapters.
//!
//! This module provides the communication layer between AgentMux terminals and the
//! backend RPC system. Key components:
//!
//! - **OSC encoding/decoding:** Terminal escape sequences for RPC messages
//! - **WshRpc:** Main RPC client with message routing and response handling
//! - **WshRpcProxy:** Single-connection proxy for remote RPC
//! - **WshMultiProxy:** Broadcast proxy for multiple connections
//! - **EventListener:** Pub/sub event system
//! - **I/O adapters:** Stream, PTY, WebSocket message conversion

pub mod osc;
pub mod event;
pub mod proxy;
pub mod rpcio;
pub mod wshrpc;
pub mod cmdreader;

