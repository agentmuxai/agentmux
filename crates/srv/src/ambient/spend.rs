// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Tells the frontend what every ambient call spent, for the status bar's token
//! totals.
//!
//! Each call's tokens are published once, from the one place every call ends
//! (`call::Slot::run`), as an `ambient:spent` event the frontend records
//! (`frontend/app/store/ambient-spend.ts`). Before, only the calls a pane itself
//! asked for were counted, by whichever pane got the reply, and five purposes
//! that run in the background (recovered titles, workflow names, previews,
//! narration, running summaries) were never counted at all.
//! docs/reports/REPORT_AMBIENT_FRAMEWORK_REASSESSMENT_2026_10_08.md section 5.8.

use std::sync::{Arc, OnceLock};

use serde_json::json;

use crate::agents::TokenCounts;
use crate::backend::eventbus::{EventBus, WSEventType, WS_EVENT_RPC};

/// The event a purpose's spend is published as.
pub const EVENT_AMBIENT_SPENT: &str = "ambient:spent";

static BUS: OnceLock<Arc<EventBus>> = OnceLock::new();

/// Publish spend on `bus` from now on. Set once at startup; without it (tests,
/// tools) spend is not published.
pub fn publish_on(bus: Arc<EventBus>) {
    let _ = BUS.set(bus);
}

/// Publish what one call for `purpose` spent.
pub fn report(purpose: &str, tokens: &TokenCounts) {
    let Some(bus) = BUS.get() else { return };
    bus.broadcast_event(&WSEventType {
        eventtype: WS_EVENT_RPC.to_string(),
        oref: String::new(),
        data: Some(json!({
            "command": "eventrecv",
            "data": {
                "event": EVENT_AMBIENT_SPENT,
                "data": { "purpose": purpose, "tokens": tokens },
            }
        })),
    });
}
