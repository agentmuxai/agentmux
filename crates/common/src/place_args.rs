// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use serde::{Deserialize, Serialize};

/// The size and place an agent gives a floating pane (docs/specs/SPEC_AGENT_PANE_PLACEMENT_FLOATING_2026_11_22.md
/// §2.1). Sent as given: srv checks them and says what is wrong.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlaceArgs {
    /// CSS pixels (a number) or a share of the work area (`"60%"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<serde_json::Value>,
    /// `center`, `right`, `left`, `top`, `bottom`, `top-right`, `top-left`, `bottom-right`, `bottom-left`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spot: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<f64>,
}

impl PlaceArgs {
    /// The placement arguments of a tool call. A value of the wrong kind is refused here, in words the agent can act on.
    pub fn of(arguments: &serde_json::Value) -> Result<Self, String> {
        let size = |k: &str| match arguments.get(k) {
            None | Some(serde_json::Value::Null) => Ok(None),
            Some(v @ (serde_json::Value::Number(_) | serde_json::Value::String(_))) => Ok(Some(v.clone())),
            Some(other) => Err(format!("{k} is a number of CSS pixels or a share of the work area such as 60%, not {other}")),
        };
        let coord = |k: &str| match arguments.get(k) {
            None | Some(serde_json::Value::Null) => Ok(None),
            Some(v) => v.as_f64().map(Some).ok_or_else(|| format!("{k} is a number of CSS pixels, not {v}")),
        };
        Ok(Self {
            width: size("width")?,
            height: size("height")?,
            spot: arguments.get("spot").and_then(|v| v.as_str()).map(str::to_string),
            x: coord("x")?,
            y: coord("y")?,
        })
    }
}
