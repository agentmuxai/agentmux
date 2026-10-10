// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The cloud presence publisher's status and "Publish now"
//! (`muxbus::wan_presence`), for Settings' "Cloud presence" line.

use super::*;

/// Why there is no status: no publisher runs (no WAN identity), or it has
/// not finished its first try.
pub(crate) const NO_STATUS: &str = "Cloud presence hasn't started on this install.";

pub fn register(engine: &Arc<WshRpcEngine>, _state: &AppState) {
    engine.register_typed(COMMAND_PRESENCE_STATUS, |_req: Option<NoArgsReq>, _ctx| async move {
        crate::muxbus::wan_presence::status().ok_or_else(|| NO_STATUS.to_string())
    });
    engine.register_typed(COMMAND_PRESENCE_PUBLISH_NOW, |_req: Option<NoArgsReq>, _ctx| async move {
        Ok(PresencePublishNowResult { started: crate::muxbus::wan_presence::publish_now() })
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn call(command: &str) -> (serde_json::Value, String) {
        let state = crate::server::tests::test_state();
        let (engine, mut rx) = WshRpcEngine::new();
        register(&engine, &state);
        engine.handle_message(RpcMessage {
            command: command.to_string(),
            reqid: "req-1".to_string(),
            data: Some(json!({})),
            ..Default::default()
        });
        let resp = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv()).await.unwrap().unwrap();
        (resp.data.unwrap_or_default(), resp.error)
    }

    #[test]
    fn the_status_is_snake_case_and_leaves_out_what_it_doesnt_know() {
        let status = PresenceStatusResult {
            state: PresenceState::SignedOut,
            since_ms: 5,
            last_ok_ms: None,
            last_error: None,
            next_try_ms: None,
            relay_version: None,
            offset_ms: None,
            record_version: 2,
            note: None,
            off_reason: None,
        };
        assert_eq!(
            serde_json::to_value(&status).unwrap(),
            json!({ "state": "signed_out", "since_ms": 5, "record_version": 2 })
        );
        let full = PresenceStatusResult {
            state: PresenceState::Retrying,
            last_ok_ms: Some(1),
            last_error: Some("cloud unreachable".into()),
            next_try_ms: Some(9),
            relay_version: Some("1.1.0".into()),
            offset_ms: Some(-720_000),
            note: Some("Your clock differs from the cloud's by 12 min".into()),
            ..status
        };
        assert_eq!(
            serde_json::to_value(&full).unwrap(),
            json!({
                "state": "retrying",
                "since_ms": 5,
                "last_ok_ms": 1,
                "last_error": "cloud unreachable",
                "next_try_ms": 9,
                "relay_version": "1.1.0",
                "offset_ms": -720_000,
                "record_version": 2,
                "note": "Your clock differs from the cloud's by 12 min",
            })
        );
        for (state, wire) in [
            (PresenceState::Publishing, "publishing"),
            (PresenceState::Unsupported, "unsupported"),
            (PresenceState::Rejected, "rejected"),
            (PresenceState::Off, "off"),
            (PresenceState::SignedOff, "signed_off"),
        ] {
            assert_eq!(serde_json::to_value(state).unwrap(), json!(wire));
        }
        let off = PresenceStatusResult {
            state: PresenceState::Off,
            off_reason: Some(PresenceOffReason::DevBuild),
            ..status
        };
        assert_eq!(
            serde_json::to_value(&off).unwrap(),
            json!({ "state": "off", "since_ms": 5, "record_version": 2, "off_reason": "dev_build" })
        );
        for (reason, wire) in [
            (PresenceOffReason::Setting, "setting"),
            (PresenceOffReason::Headless, "headless"),
            (PresenceOffReason::IsolatedHome, "isolated_home"),
            (PresenceOffReason::TestHarness, "test_harness"),
        ] {
            assert_eq!(serde_json::to_value(reason).unwrap(), json!(wire));
        }
    }

    #[tokio::test]
    async fn without_a_publisher_the_status_says_why_and_publish_now_starts_nothing() {
        let (_, error) = call(COMMAND_PRESENCE_STATUS).await;
        assert_eq!(error, NO_STATUS);
        let (data, error) = call(COMMAND_PRESENCE_PUBLISH_NOW).await;
        assert!(error.is_empty(), "{error}");
        assert_eq!(data, json!({ "started": false }));
    }
}
