// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Wire types for whether cloud (MuxBus) messages are reaching this channel's
//! agents: `muxbus.status`'s `delivery` field and the `muxbus:status` event.
//! Derived in `muxbus::delivery_status`.

use serde::{Deserialize, Serialize};

/// Where cloud delivery stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub enum MuxBusDeliveryState {
    /// This channel isn't signed in to MuxBus, by choice or never was.
    SignedOut,
    /// Signed in and connected to the relay: messages arrive.
    Connected,
    /// The sign-in is fine but the relay can't be reached right now.
    Reconnecting,
    /// The sign-in stopped working; only a new sign-in brings messages back.
    NeedsSignIn,
}

impl MuxBusDeliveryState {
    /// Cloud messages aren't arriving although this channel means to get them.
    pub fn is_paused(self) -> bool {
        matches!(self, MuxBusDeliveryState::Reconnecting | MuxBusDeliveryState::NeedsSignIn)
    }
}

/// `muxbus.status`'s `delivery`, and the payload of the `muxbus:status`
/// event sent whenever it changes. Times are this computer's clock (unix ms).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct MuxBusDeliveryStatus {
    pub state: MuxBusDeliveryState,
    /// When `state` began.
    #[ts(type = "number")]
    pub since_ms: i64,
    /// When the relay last answered this channel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub last_ok_ms: Option<i64>,
    /// Why messages aren't arriving, in a few words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub last_error: Option<String>,
    /// The account this channel is (or was last) signed in as.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub account_email: Option<String>,
}

impl Default for MuxBusDeliveryStatus {
    fn default() -> Self {
        MuxBusDeliveryStatus {
            state: MuxBusDeliveryState::SignedOut,
            since_ms: 0,
            last_ok_ms: None,
            last_error: None,
            account_email: None,
        }
    }
}
