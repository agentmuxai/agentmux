// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The request types of the Browser actions that take back, take, give up and report on control of a pane (`retake`,
//! `take_control`, `release`, `handoff_status`). Re-exported by `api_types`, where the rest of the browser requests are.

use serde::{Deserialize, Serialize};

use crate::api_types::UiAutomationAuth;

/// `POST /api/v1/ui/browser/retake` — backs Browser's `retake`: the agent drives again a browser pane it lost to the
/// person's Take over, once they gave it back. `pane` is required: it names the pane that was lost.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiBrowserRetakeRequest {
    #[serde(flatten)]
    pub auth: UiAutomationAuth,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane: Option<String>,
}

/// `POST /api/v1/ui/browser/take_control` — backs Browser's `take_control`: the agent drives a browser pane it did not open
/// (found with Pane layout). `pane` is required. `allowed_origins` limits it to those sites, as `open` does; without it the
/// pane is limited to the site it is on.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiBrowserTakeControlRequest {
    #[serde(flatten)]
    pub auth: UiAutomationAuth,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_origins: Option<Vec<String>>,
}

/// `POST /api/v1/ui/browser/release` — backs Browser's `release`: the agent stops driving a pane, which then has no driver.
/// `pane` is required.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiBrowserReleaseRequest {
    #[serde(flatten)]
    pub auth: UiAutomationAuth,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane: Option<String>,
}

/// `POST /api/v1/ui/browser/handoff_status` — backs Browser's `handoff_status`: what became of a hand-off the agent asked
/// the person for. By `handoff_id` (the hand-off's answer carries it), or the newest one on `pane`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiBrowserHandoffStatusRequest {
    #[serde(flatten)]
    pub auth: UiAutomationAuth,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff_id: Option<String>,
}
