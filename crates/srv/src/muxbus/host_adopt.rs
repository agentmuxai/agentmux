// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A refresh that moves the stored sign-in to the host AgentMux Cloud's discovery document names.
//!
//! A sign-in keeps the Cognito host it was made on and refreshes there for good, so when the cloud moves its sign-in
//! to another name an install would go on calling the old one until its next sign-in. After a refresh works against
//! the discovered host, that host is the one stored. The stored host stays the fallback, so a discovery answer that
//! names a host that does not work signs nobody out: the refresh is made there as it always was, and if it fails the
//! error is the stored host's.
//!
//! Only when the discovery document's client id is the one the sign-in was issued to; a different one is a user pool
//! the cloud has left, which `discovery::client_id_superseded` already treats as needing a new sign-in.

use super::discovery::CloudSettings;
use super::pkce::{refresh_token, RefreshTokenError};
use crate::backend::storage::muxbus::MuxBusCredentials;

fn trimmed(host: &str) -> &str {
    host.trim_end_matches('/')
}

/// The discovered host to refresh against first, if there is one: the cloud names a host that is not the stored one, for
/// the client the sign-in was issued to.
pub fn host_to_adopt(discovered: Option<&CloudSettings>, current: &MuxBusCredentials) -> Option<String> {
    let published = discovered?;
    let (host, stored) = (trimmed(&published.cognito.domain), trimmed(&current.cognito_domain));
    let same_client = !current.client_id.is_empty() && published.cognito.client_id == current.client_id;
    (same_client && !host.is_empty() && !host.eq_ignore_ascii_case(stored)).then(|| host.to_string())
}

/// Refreshes `current`: against the discovered host when [`host_to_adopt`] names one (the result then carries that host),
/// else, or when that fails for any reason, against the stored host.
pub async fn refresh(current: &MuxBusCredentials, discovered: Option<&CloudSettings>, http: &reqwest::Client) -> Result<MuxBusCredentials, RefreshTokenError> {
    if let Some(host) = host_to_adopt(discovered, current) {
        let moved = MuxBusCredentials { cognito_domain: host.clone(), ..current.clone() };
        match refresh_token(&moved, http).await {
            Ok(refreshed) => {
                tracing::info!(from = %current.cognito_domain, to = %host, "cloud sign-in: refreshed on the host discovery names; it is the stored host now");
                return Ok(refreshed);
            }
            Err(e) => tracing::info!(discovered = %host, stored = %current.cognito_domain, error = %e, "cloud sign-in: the discovered host did not refresh; staying on the stored one"),
        }
    }
    refresh_token(current, http).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    const STORED: &str = "https://old-signin.example.test";
    const DISCOVERED: &str = "https://new-signin.example.test";

    fn creds(domain: &str) -> MuxBusCredentials {
        MuxBusCredentials {
            cognito_domain: domain.to_string(),
            client_id: "client-a".into(),
            access_token: "old-access".into(),
            refresh_token: "refresh-1".into(),
            id_token: "id-1".into(),
            expires_at: 1,
            user_email: "a@example.test".into(),
            user_sub: "sub-1".into(),
        }
    }

    fn published(domain: &str, client_id: &str) -> CloudSettings {
        let doc = format!(
            r#"{{"version":1,"api":"https://api.example.test","ws":"wss://ws.example.test","cognito":{{"domain":"{domain}","clientId":"{client_id}"}}}}"#
        );
        // A plain-http host is only accepted from a plain-http (local) relay, which the stand-in hosts need.
        super::super::discovery::parse(&doc, "http://localhost:3100").unwrap()
    }

    /// A stand-in sign-in host answering every token call with `status` and a token body; counts the calls.
    async fn host(status: &'static str) -> (String, Arc<AtomicUsize>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let hits = Arc::new(AtomicUsize::new(0));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let counted = hits.clone();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let counted = counted.clone();
                tokio::spawn(async move {
                    let mut buf = [0u8; 4096];
                    let _ = stream.read(&mut buf).await;
                    counted.fetch_add(1, Ordering::SeqCst);
                    let body = if status.starts_with("200") { r#"{"access_token":"new-access","expires_in":3600}"# } else { r#"{"error":"invalid_grant"}"# };
                    let resp = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                    let _ = stream.write_all(resp.as_bytes()).await;
                });
            }
        });
        (url, hits)
    }

    #[test]
    fn a_host_is_adopted_only_for_the_client_the_sign_in_was_issued_to_and_only_when_it_differs() {
        let current = creds(STORED);
        assert_eq!(host_to_adopt(Some(&published(DISCOVERED, "client-a")), &current), Some(DISCOVERED.to_string()));
        assert_eq!(host_to_adopt(Some(&published("https://new-signin.example.test/", "client-a")), &current), Some(DISCOVERED.to_string()), "a trailing slash is not part of the host");
        assert_eq!(host_to_adopt(Some(&published(STORED, "client-a")), &current), None, "already there");
        assert_eq!(host_to_adopt(Some(&published("https://OLD-signin.example.test/", "client-a")), &current), None, "the same host in other letters");
        assert_eq!(host_to_adopt(Some(&published(DISCOVERED, "client-b")), &current), None, "another client is another pool");
        assert_eq!(host_to_adopt(Some(&published(DISCOVERED, "client-a")), &MuxBusCredentials { client_id: String::new(), ..current.clone() }), None, "no recorded client id: can't tell");
        assert_eq!(host_to_adopt(None, &current), None, "no document: nothing to follow");
    }

    #[tokio::test]
    async fn a_refresh_that_works_on_the_discovered_host_makes_it_the_stored_one() {
        let (old, old_hits) = host("200 OK").await;
        let (new, new_hits) = host("200 OK").await;
        let refreshed = refresh(&creds(&old), Some(&published(&new, "client-a")), &reqwest::Client::new()).await.unwrap();
        assert_eq!(refreshed.cognito_domain, new);
        assert_eq!((refreshed.access_token.as_str(), refreshed.refresh_token.as_str(), refreshed.user_sub.as_str()), ("new-access", "refresh-1", "sub-1"));
        assert_eq!((old_hits.load(Ordering::SeqCst), new_hits.load(Ordering::SeqCst)), (0, 1), "one call, to the new host");
    }

    #[tokio::test]
    async fn a_discovered_host_that_fails_leaves_the_stored_one_working() {
        for failing in ["400 Bad Request", "500 Internal Server Error", "502 Bad Gateway", "429 Too Many Requests"] {
            let (old, old_hits) = host("200 OK").await;
            let (new, new_hits) = host(failing).await;
            let refreshed = refresh(&creds(&old), Some(&published(&new, "client-a")), &reqwest::Client::new()).await.unwrap();
            assert_eq!(refreshed.cognito_domain, old, "{failing}: still the stored host");
            assert_eq!(refreshed.access_token, "new-access");
            assert_eq!((old_hits.load(Ordering::SeqCst), new_hits.load(Ordering::SeqCst)), (1, 1), "{failing}");
        }
        // A host nobody answers at (DNS, refused connection) is the same.
        let (old, _) = host("200 OK").await;
        let refreshed = refresh(&creds(&old), Some(&published("http://127.0.0.1:1", "client-a")), &reqwest::Client::new()).await.unwrap();
        assert_eq!(refreshed.cognito_domain, old);
    }

    #[tokio::test]
    async fn when_neither_host_works_the_error_is_the_stored_hosts() {
        let (old, _) = host("400 Bad Request").await;
        let (new, _) = host("500 Internal Server Error").await;
        let err = refresh(&creds(&old), Some(&published(&new, "client-a")), &reqwest::Client::new()).await.unwrap_err();
        assert!(matches!(err, RefreshTokenError::Rejected { status: 400, .. }), "{err:?}");
        let err = refresh(&creds("http://127.0.0.1:1"), Some(&published("http://127.0.0.1:2", "client-a")), &reqwest::Client::new()).await.unwrap_err();
        assert!(matches!(err, RefreshTokenError::Network(_)), "a discovered host that is down does not turn an unreachable one into a rejection: {err:?}");
    }

    #[tokio::test]
    async fn without_a_host_to_adopt_only_the_stored_host_is_called() {
        let (old, old_hits) = host("200 OK").await;
        let (other, other_hits) = host("200 OK").await;
        let http = reqwest::Client::new();
        refresh(&creds(&old), None, &http).await.unwrap();
        refresh(&creds(&old), Some(&published(&old, "client-a")), &http).await.unwrap();
        refresh(&creds(&old), Some(&published(&other, "client-b")), &http).await.unwrap();
        assert_eq!((old_hits.load(Ordering::SeqCst), other_hits.load(Ordering::SeqCst)), (3, 0));
    }

    #[tokio::test]
    async fn an_install_signed_in_on_the_old_host_keeps_working_through_each_phase_of_the_move() {
        // The old name is served by a front that passes token calls on, so it answers throughout; discovery names the old
        // host, then the new host before it exists, then the new host once it does.
        let (old, old_hits) = host("200 OK").await;
        let http = reqwest::Client::new();
        let mut stored = creds(&old);
        refresh(&stored, Some(&published(&old, "client-a")), &http).await.unwrap();
        let refreshed = refresh(&stored, Some(&published("http://127.0.0.1:1", "client-a")), &http).await.unwrap();
        assert_eq!(refreshed.cognito_domain, old, "the new name does not exist yet");
        let (new, _) = host("200 OK").await;
        stored = refresh(&stored, Some(&published(&new, "client-a")), &http).await.unwrap();
        assert_eq!(stored.cognito_domain, new, "the new name works: adopted");
        assert_eq!(old_hits.load(Ordering::SeqCst), 2, "the old host served the first two refreshes; the third went to the new one");
        // Refreshing again with discovery still naming it stays there, without a second hop.
        let again = refresh(&stored, Some(&published(&new, "client-a")), &http).await.unwrap();
        assert_eq!(again.cognito_domain, new);
    }
}
