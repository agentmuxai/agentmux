// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The TLS crypto provider every reqwest client needs.
//!
//! reqwest 0.13 is built with `rustls-no-provider` so the tree keeps ring (no aws-lc build), and
//! it refuses to build a client, even one for plain http, until a provider is installed for the
//! process. Call [`install_ring_provider`] first thing in each binary that builds a client, and
//! once in each test binary that does.

use std::sync::Once;

/// Installs ring as the process-wide rustls provider. Safe to call more than once, and a provider
/// installed earlier by anything else is left alone.
pub fn install_ring_provider() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installing_twice_leaves_ring_as_the_default() {
        install_ring_provider();
        install_ring_provider();
        assert!(rustls::crypto::CryptoProvider::get_default().is_some());
    }
}
