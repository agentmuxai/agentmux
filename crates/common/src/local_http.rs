// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The HTTP client of a program that only talks to the local AgentMux server (the MCP server, bashwrap, the desktop
//! host's credential broker): plain http on the loopback, never https.
//!
//! reqwest 0.13 reads the machine's certificates whenever a client is built, as soon as any crate built in the same
//! cargo invocation turns TLS on (the server package builds `agentmux-srv`, `agentmux-mcp` and `agentmux-bashwrap`
//! together, so all three get it), and on a Linux machine with none it refuses to build the client: `agentmux-mcp`
//! panicked at start on such a machine. These programs never make an https request, so their clients are built to
//! trust nothing: no certificates are read, and a build cannot fail for want of them. srv, which does make https
//! requests, has its own helper (`crates/srv/src/http_client.rs`); `scripts/check-srv-http-clients.mjs` fails CI on a
//! client built any other way in these crates.

/// A client builder that trusts nothing and reads no certificates: set what the site needs and `build()`.
pub fn builder() -> reqwest::ClientBuilder {
    crate::tls_provider::install_ring_provider();
    reqwest::Client::builder().tls_certs_only(Vec::<reqwest::Certificate>::new())
}

/// A client with reqwest's defaults, for a site that sets nothing. Never fails: with the provider installed, trusting
/// nothing is a valid configuration.
pub fn client() -> reqwest::Client {
    builder().build().unwrap_or_else(|e| {
        // Unreachable once the provider is installed.
        panic!("the TLS provider is missing, so no HTTP client can be built: {e}")
    })
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn a_client_is_built_where_reqwests_own_constructor_cannot() {
        // A machine with no certificates: reqwest reads only the file SSL_CERT_FILE names when it is set. The environment
        // is read when a client is built; nextest runs each test in its own process, so changing it here touches no other test.
        let dir = tempfile::tempdir().unwrap();
        let empty = dir.path().join("empty.pem");
        std::fs::write(&empty, "").unwrap();
        std::env::set_var("SSL_CERT_FILE", &empty);
        std::env::remove_var("SSL_CERT_DIR");
        crate::tls_provider::install_ring_provider();
        assert!(reqwest::Client::builder().build().is_err(), "the premise: this machine has no certificates");
        assert!(builder().build().is_ok());
        let _ = client();
    }
}
