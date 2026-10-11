// Copyright 2025, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

export const CHORD_TIMEOUT = 2000;

/**
 * The HTTP header that carries srv's auth key. The Rust side's twin is
 * `agentmux_common::AUTH_KEY_HEADER` (crates/common/src/lib.rs).
 */
export const AUTH_KEY_HEADER = "X-AuthKey";
