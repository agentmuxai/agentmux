// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The request types of `Agents create` and of the `runtime` choice `Agents spawn` shares with it. Re-exported by
//! `api_types`, where the rest of the agent requests are.

use serde::{Deserialize, Serialize};

use crate::api_types::UiAutomationAuth;

/// Where a new agent runs, for `Agents spawn` and `Agents create`: absent, a worker runs as its leader does
/// and a created agent as its template says.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContainerAsk {
    /// `native` (on the host) or `container`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<String>,
    /// The image of a container agent: `ghcr.io/agentmuxai/agent-base:developer` or `ghcr.io/agentmuxai/agent-base:latest`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container_image: Option<String>,
    /// A container agent's memory limit in MB (`"auto"`, or `0` for no limit).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_mb: Option<serde_json::Value>,
    /// A container agent's CPU limit (fractions allowed; `"auto"`, or `0` for no limit).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpus: Option<serde_json::Value>,
}

impl ContainerAsk {
    /// Whether the caller asked for anything.
    pub fn is_empty(&self) -> bool {
        self.runtime.is_none() && self.container_image.is_none() && self.memory_mb.is_none() && self.cpus.is_none()
    }
}

/// `POST /api/v1/agent/create`: a named agent made from a template, not a worker (`Agents create`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateAgentRequest {
    #[serde(flatten)]
    pub auth: UiAutomationAuth,
    /// The new agent's name.
    pub name: String,
    /// The seeded template to make it from, by id or name.
    pub template: String,
    /// The account to link, by id or name/email; absent, the caller's, or another signed-in one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    #[serde(flatten)]
    pub container: ContainerAsk,
    /// The user's own words in this turn asking for it, verbatim; empty when the user didn't ask.
    #[serde(default)]
    pub user_instruction: String,
}
