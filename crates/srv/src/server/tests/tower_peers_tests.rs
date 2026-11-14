// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A Tower peer against a real viewer listener over TLS (`backend::tower::peers`). The listener is
//! `server::build_viewer_router`, which belongs to the layer above `backend::tower`, so the test
//! lives here.

use crate::backend::tower::peers::test_support::{peers_in, FP};
use crate::backend::tower::peers::{pair, sample, FILE, PEER_PREFIX};

/// End to end against a real viewer listener over TLS: pair with the
/// link, get refused until the user shares, then read the snapshot; a
/// link pinning another certificate never pairs; a revoked device is
/// told so.
#[tokio::test(flavor = "multi_thread")]
async fn a_peer_pairs_over_pinned_tls_and_reads_once_sharing_is_on() {
    let viewer_dir = tempfile::tempdir().unwrap();
    let mut state = crate::server::tests::test_state();
    let broker = crate::backend::mps::Broker::new();
    state.viewer = crate::backend::viewer::ViewerService::new(Some(viewer_dir.path().to_path_buf()), &broker);
    let tls = state.viewer.tls().unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let token = tokio_util::sync::CancellationToken::new();
    crate::backend::lan::listeners::LanListenerSupervisor::serve_tls(
        crate::server::build_viewer_router(state.clone()),
        listener,
        tokio_rustls::TlsAcceptor::from(tls.server_config().unwrap()),
        token.clone(),
    );
    let link = |fp: &str| {
        let (code, _) = state.viewer.pairing.start();
        format!("agentmux://pair?v=1&host=127.0.0.1&port={port}&fp={fp}&code={code}&hostname=studio")
    };

    let dir = tempfile::tempdir().unwrap();
    let (peers, tokens) = peers_in(dir.path());
    // Another certificate's fingerprint: the handshake fails, no pairing.
    let err = pair(peers, &link(FP), "laptop (Tower)").await.unwrap_err();
    assert!(err.contains("Can't reach studio"), "{err}");
    assert!(peers.list().is_empty());

    let info = pair(peers, &link(&tls.fingerprint), "laptop (Tower)").await.unwrap();
    let id = info.connection.strip_prefix(PEER_PREFIX).unwrap().to_string();
    assert_eq!(tokens.0.lock().unwrap().len(), 1, "the token went to the token store");
    assert!(!std::fs::read_to_string(dir.path().join(FILE)).unwrap().contains("amxv_"));

    let err = sample(peers, &id, "", vec![]).await.unwrap_err();
    assert!(err.contains("doesn't share"), "{err}");

    let mut settings = state.config_watcher.get_settings();
    settings.extra.insert("tower:sharewithpaired".to_string(), serde_json::json!(true));
    state.config_watcher.update_settings(settings);
    let snap = sample(peers, &id, "", vec![]).await.unwrap();
    assert!(snap.remote);
    assert!(snap.host.unwrap().total > 0);

    // The other computer's user revokes this device.
    let device = peers.get(&id).unwrap().device_id;
    state.mstore.viewer_device_delete(&device).unwrap();
    let err = sample(peers, &id, "", vec![]).await.unwrap_err();
    assert!(err.contains("no longer accepts"), "{err}");
    token.cancel();
}
