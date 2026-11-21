// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Wire type for accepting the terms (`client.agreetos`).

use serde::{Deserialize, Serialize};

/// `client.agreetos`: when the terms were accepted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct ClientAgreeTosResult {
    /// Unix milliseconds.
    #[ts(type = "number")]
    pub tosagreed: i64,
}
