// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Telling a window that srv dropped events for it. A connection whose consumer falls behind
//! has events dropped from a full lane (`backend::eventbus::LaneDrops`); the window's objects,
//! panes and streams then stay as stale as the last event it did get, until something
//! re-fetches them. This sends `ws:eventsdropped` straight to the socket (not through the full
//! lane) once the window is reading again, and the window re-fetches what it shows
//! (`frontend/app/store/resync.ts`).

use axum::extract::ws::{Message, WebSocket};

use crate::backend::eventbus::{EventBus, EventsDroppedEvent, LaneDrops};
use crate::backend::mps_events::EventsDropped;

/// The frame that tells a window `dropped` priority events never reached it.
pub(super) fn events_dropped_frame(dropped: u64) -> serde_json::Value {
    let dropped = u32::try_from(dropped).unwrap_or(u32::MAX);
    EventBus::typed_frame::<EventsDropped>(&EventsDroppedEvent { dropped })
}

/// Tell the window about any events dropped since it last heard. Returns `true` if the send
/// failed (the caller should end the connection).
pub(super) async fn report_drops(socket: &mut WebSocket, drops: &LaneDrops) -> bool {
    let dropped = drops.take_unreported();
    if dropped == 0 {
        return false;
    }
    tracing::info!("ws: telling the window {dropped} priority events were dropped, so it re-fetches");
    let Ok(msg) = serde_json::to_string(&events_dropped_frame(dropped)) else {
        return false;
    };
    socket.send(Message::Text(msg.into())).await.is_err()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_frame_is_the_eventrecv_the_window_reads() {
        let frame = events_dropped_frame(12);
        assert_eq!(frame["eventtype"], "rpc");
        assert_eq!(frame["data"]["command"], "eventrecv");
        assert_eq!(frame["data"]["data"]["event"], "ws:eventsdropped");
        assert_eq!(frame["data"]["data"]["data"]["dropped"], 12);
    }

    #[test]
    fn a_huge_count_saturates_instead_of_wrapping() {
        assert_eq!(events_dropped_frame(u64::MAX)["data"]["data"]["data"]["dropped"], u32::MAX);
    }
}
