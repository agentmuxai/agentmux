// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Wire types for the connection-scoped commands registered per websocket.
//!
//! These are the highest-traffic RPCs in the app — every pane subscribes,
//! every keystroke in a terminal is a `controllerinput` — and they were the
//! last block of commands whose request/response shapes lived only as
//! hand-written TypeScript.

use serde::{Deserialize, Serialize};

/// Request for the four commands that ignore their payload: `appinfo`,
/// `getwaveairatelimit`, `eventunsuball` and `getfullconfig`.
///
/// One type for all four rather than four identical empty structs: there is
/// nothing to diverge, and three names for "no arguments" is noise. Registered
/// as `Option<Self>`, which is what makes both encodings of "no argument"
/// deserialize — the stub sends `{}`, a client that omits `data` sends `null`,
/// and serde accepts each from only one of `()` and a struct.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct NoArgsReq {}

/// Response for `appinfo`.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct AppInfoResult {
    pub version: String,
}

/// Response for `getwaveairatelimit`.
///
/// AgentMux does not rate-limit, so every field is a fixed placeholder and
/// `unknown` is always true. The shape exists because the renderer inherited a
/// rate-limit widget that expects it; generating it at least means the
/// placeholder and the type that describes it cannot drift apart.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct AiRateLimitResult {
    #[ts(type = "number")]
    pub req: i64,
    #[ts(type = "number")]
    pub reqlimit: i64,
    #[ts(type = "number")]
    pub preq: i64,
    #[ts(type = "number")]
    pub preqlimit: i64,
    #[ts(type = "number")]
    pub resetepoch: i64,
    pub unknown: bool,
}
