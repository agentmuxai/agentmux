// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

//! RPC communication layer: message routing and request/response engine.
//! Port of Go's pkg/wshutil/wshrouter.go and pkg/wshutil/wshrpc.go.

pub mod engine;
pub mod router;
