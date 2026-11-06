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

/// Which harness and model route a new agent runs on, for `Agents spawn` and `Agents create`: absent, a worker
/// has its leader's harness and route, and a created agent its template's. The model is the request's own `model`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RouteAsk {
    /// The harness (`claude`, `codex`, ...). It is fixed when the agent is made.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness: Option<String>,
    /// A model route from `Agents routes` (`openrouter`); `anthropic` or `login` for the harness's own sign-in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<String>,
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
    /// The model, on the route (`route`); absent, the route's default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(flatten)]
    pub route: RouteAsk,
    /// The user's own words in this turn asking for it, verbatim; empty when the user didn't ask.
    #[serde(default)]
    pub user_instruction: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawnAgentRequest {
    #[serde(flatten)]
    pub auth: UiAutomationAuth,
    /// The worker's name; absent, the first free "<leader> worker N".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The worker's model and effort; absent, the leader's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// The account to run the worker on, by id or name/email; absent, the
    /// leader's, or another signed-in one when the leader's can't run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    /// The worker's task, its first message. srv shows its start to the user
    /// when it asks; the MCP server sends it, signed as the leader.
    pub brief: String,
    /// Where it runs, when not as its leader does.
    #[serde(flatten)]
    pub container: ContainerAsk,
    #[serde(flatten)]
    pub route: RouteAsk,
    /// The user's own words in this turn asking for it, verbatim; empty when
    /// the user didn't ask. Checked like `QuitSelf`'s (`self_quit::gate`).
    #[serde(default)]
    pub user_instruction: String,
}

/// One worker of a `POST /api/v1/agent/spawn_many`: what `SpawnAgentRequest` asks of one.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawnWorkerSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    /// The worker's task, its first message.
    pub brief: String,
    #[serde(flatten)]
    pub container: ContainerAsk,
    #[serde(flatten)]
    pub route: RouteAsk,
}

/// `POST /api/v1/agent/spawn_many`: several workers at once, with one approval for all of them
/// (`SPEC_COUNTDOWN_BUTTONS_AND_BATCH_AGENT_ACTIONS_2026_10_28.md` §5). All start or none do.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawnManyRequest {
    #[serde(flatten)]
    pub auth: UiAutomationAuth,
    pub workers: Vec<SpawnWorkerSpec>,
    /// The user's own words in this turn asking for them, verbatim; empty when the user didn't ask.
    #[serde(default)]
    pub user_instruction: String,
}
