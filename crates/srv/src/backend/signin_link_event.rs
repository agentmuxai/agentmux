// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The `identity:signinlink` event (`server/signin_notice.rs`), kept below the server so
//! `mps_events` can name it.

use serde::Serialize;

/// The `identity:signinlink` event: show this link as a notice.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct SigninLinkEvent {
    pub provider: String,
    /// The machine that is signing in: "Sign narko in to Claude".
    pub machine: String,
    pub url: String,
    /// The one-time code to enter on that page, for a sign-in that uses one.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub code: Option<String>,
}
