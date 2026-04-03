// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! RPC communication layer: message routing and request/response engine.
//! Port of Go's pkg/wshutil/wshrouter.go and pkg/wshutil/wshrpc.go.

pub mod engine;
pub mod router;
