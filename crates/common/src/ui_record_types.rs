// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The request and response of `POST /api/v1/ui/record` (the `UIRecord` tool). Re-exported by `api_types`, where the
//! rest of the UI requests are.

use serde::{Deserialize, Serialize};

use crate::api_types::UiAutomationAuth;

/// `POST /api/v1/ui/record`: `op` is `start` (a recording of the caller's own pane, or of `pane`) or `stop`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiRecordRequest {
    #[serde(flatten)]
    pub auth: UiAutomationAuth,
    pub op: String,
    /// As for `UiScreenshotRequest`: a browser pane this agent opened; absent = the caller's own pane.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane: Option<String>,
    /// start: frames a second (1 to 10, default 5).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fps: Option<u32>,
    /// start: the most seconds to record (1 to 30, default 30).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seconds: Option<u32>,
    /// stop: send the finished file's bytes too (`gif_base64`), for a caller that cannot read srv's own folder,
    /// such as an agent in a container.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub inline: bool,
}

/// Response for `POST /api/v1/ui/record`. `status` is `recording` or `saved`; `path` is the finished file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiRecordResponse {
    pub status: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// The finished file's bytes, when the request asked for them (`inline`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gif_base64: Option<String>,
    #[serde(default)]
    pub frames: u32,
    #[serde(default)]
    pub skipped: u32,
    #[serde(default)]
    pub seconds: f32,
}
