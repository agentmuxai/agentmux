// Copyright 2025, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

export const CHORD_TIMEOUT = 2000;

/**
 * The HTTP header that carries srv's auth key. The Rust side's twin is
 * `agentmux_common::AUTH_KEY_HEADER` (crates/common/src/lib.rs).
 */
export const AUTH_KEY_HEADER = "X-AuthKey";

/** A signed-in browser's per-tab session token (`server/local_session_routes.rs`). */
export const SESSION_HEADER = "X-AM-Session";
/** The WebSocket protocol srv selects; the token is the other protocol the page offers. */
export const SESSION_WS_PROTOCOL = "amx-session";
