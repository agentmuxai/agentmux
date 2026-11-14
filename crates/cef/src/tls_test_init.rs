// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! reqwest 0.13 builds no client, not even for plain http, until a rustls crypto provider is
//! installed for the process (`agentmux_common::tls_provider`). Runs once before this test
//! binary's tests.

#[ctor::ctor(unsafe)]
fn install_ring_provider_for_tests() {
    agentmux_common::tls_provider::install_ring_provider();
}
