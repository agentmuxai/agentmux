// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

pub mod agent_credentials;
pub mod cloud_subscriber;
pub mod delivery_status;
pub mod discovery;
pub mod wan_lease;
pub mod pkce;
pub mod relay;
pub mod wan_presence;
pub mod wan_publish;
pub mod wan_verify;

use crate::backend::storage::muxbus::MuxBusCredentials;
use crate::broker::CredentialState;

/// Identifier MuxBus's single global credential set is registered under
/// with both the broker scheduler (`crate::broker`) and the OS keychain
/// (`backend::storage::muxbus`'s `secret_store` key) — deliberately the same
/// string in both places so the two are trivially correlated in logs.
pub const CREDENTIAL_ID: &str = "muxbus:global";

/// Why the stored AgentMux Cloud sign-in can't work anymore, or `None` if it
/// still can (docs/specs/SPEC_CLOUD_SETTINGS_DISCOVERY_2026_09_27.md §3.3).
/// Stale means only a new sign-in helps:
/// - the cloud publishes a different desktop client id than the one the
///   credential was issued to (it belongs to a user pool the cloud has left);
/// - or the broker's refresh was refused outright (`NeedsReauth` with
///   `rejected`), and the stored token isn't fresh. A fresh one means someone
///   signed in since, possibly from another channel sharing the store; the
///   broker catches up on its next check.
///
/// Transient failures that merely piled up into `NeedsReauth` are not stale:
/// the network coming back can still cure them.
pub(crate) fn stale_sign_in_reason(
    broker: Option<&CredentialState>,
    creds: &MuxBusCredentials,
    discovered: Option<&discovery::CloudSettings>,
) -> Option<String> {
    if discovery::client_id_superseded(discovered, &creds.client_id) {
        return Some("the sign-in belongs to a user pool AgentMux Cloud no longer uses".to_string());
    }
    match broker {
        Some(CredentialState::NeedsReauth { rejected: true, reason, .. })
            if !(creds.is_valid() && !creds.nearly_expired()) =>
        {
            Some(format!("the sign-in was refused: {reason}"))
        }
        _ => None,
    }
}

/// [`stale_sign_in_reason`] against the global broker and the cloud's
/// published settings (cached, see `discovery::cloud_settings`).
pub(crate) async fn stale_sign_in(creds: &MuxBusCredentials, http: &reqwest::Client) -> Option<String> {
    let broker = crate::broker::get_global().and_then(|s| s.state(CREDENTIAL_ID));
    let discovered = discovery::cloud_settings(http).await;
    stale_sign_in_reason(broker.as_ref(), creds, discovered.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn creds(client_id: &str, expires_in: i64) -> MuxBusCredentials {
        MuxBusCredentials {
            client_id: client_id.to_string(),
            access_token: "at".to_string(),
            refresh_token: "rt".to_string(),
            expires_at: agentmux_common::time::now_secs() + expires_in,
            ..Default::default()
        }
    }

    fn needs_reauth(rejected: bool) -> CredentialState {
        CredentialState::NeedsReauth { since_unix: 0, reason: "invalid_grant".to_string(), rejected }
    }

    fn published(client_id: &str) -> discovery::CloudSettings {
        let doc = format!(
            r#"{{"version":1,"api":"https://a.test","ws":"wss://b.test","cognito":{{"domain":"https://c.test","clientId":"{client_id}"}}}}"#
        );
        discovery::parse(&doc, "https://a.test").unwrap()
    }

    #[test]
    fn a_refused_refresh_is_stale_until_a_fresh_token_is_stored() {
        let expired = creds("client", -10);
        assert!(stale_sign_in_reason(Some(&needs_reauth(true)), &expired, None).is_some());
        // Nearly expired counts too: that is when the refresh was attempted.
        assert!(stale_sign_in_reason(Some(&needs_reauth(true)), &creds("client", 60), None).is_some());
        // Signed in again (here or from another channel): not stale.
        assert!(stale_sign_in_reason(Some(&needs_reauth(true)), &creds("client", 3600), None).is_none());
    }

    #[test]
    fn piled_up_transient_failures_and_healthy_states_are_not_stale() {
        let expired = creds("client", -10);
        assert!(stale_sign_in_reason(Some(&needs_reauth(false)), &expired, None).is_none());
        assert!(stale_sign_in_reason(Some(&CredentialState::Fresh), &expired, None).is_none());
        let failed = CredentialState::Failed { consecutive_failures: 2, last_error: "net".to_string() };
        assert!(stale_sign_in_reason(Some(&failed), &expired, None).is_none());
        assert!(stale_sign_in_reason(None, &expired, None).is_none());
    }

    #[test]
    fn a_sign_in_from_a_client_the_cloud_no_longer_publishes_is_stale_even_while_its_token_lasts() {
        let valid = creds("old-client", 3600);
        let reason = stale_sign_in_reason(Some(&CredentialState::Fresh), &valid, Some(&published("new-client")));
        assert!(reason.unwrap().contains("user pool"));
        assert!(stale_sign_in_reason(Some(&CredentialState::Fresh), &valid, Some(&published("old-client"))).is_none());
    }
}
