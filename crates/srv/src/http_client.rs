// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The one place srv builds an HTTP client.
//!
//! reqwest 0.13 reads the machine's certificates when it builds a client, and on Linux it refuses to build one when
//! there are none ("No CA certificates were loaded from the system"). `Client::new()` turns that refusal into a panic,
//! so a server on a machine without `ca-certificates` died at start. Here a client is always built:
//!
//!   * With system certificates the client trusts exactly them, as before.
//!   * Without any, the client trusts nothing: every https request fails (an untrusted certificate), plain http to the
//!     local machine and the LAN keeps working, and the log and the connectivity error say why. Nothing is bundled,
//!     no other trust source is read, and no verification is relaxed.
//!
//! Every client srv builds goes through [`builder`], [`client_with`] or [`client`]; `scripts/check-srv-http-clients.mjs` fails CI on a
//! direct `reqwest::Client::new()` or `reqwest::Client::builder()` anywhere else in srv's own code.

use std::sync::OnceLock;

use agentmux_common::system_certs::no_system_certs_message;

/// What srv's https requests are checked against on this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Trust {
    /// The machine's own certificates.
    System,
    /// Nothing: the machine has no certificates (the reason says what reqwest reported).
    Nothing { reason: String },
}

/// Finds out what the machine offers by building a client the way reqwest does by default. Reads the certificates
/// each time it is called; [`trust`] keeps the answer.
fn probe() -> Trust {
    agentmux_common::tls_provider::install_ring_provider();
    match reqwest::Client::builder().build() {
        Ok(_) => Trust::System,
        Err(e) => Trust::Nothing { reason: chain(&e) },
    }
}

/// An error and its causes, on one line.
fn chain(e: &dyn std::error::Error) -> String {
    let mut out = e.to_string();
    let mut cause = e.source();
    while let Some(c) = cause {
        out.push_str(": ");
        out.push_str(&c.to_string());
        cause = c.source();
    }
    out
}

/// What the machine offers, found out once per process. The first call that finds no certificates says so in the log.
pub fn trust() -> &'static Trust {
    static TRUST: OnceLock<Trust> = OnceLock::new();
    TRUST.get_or_init(|| {
        let found = probe();
        if let Trust::Nothing { reason } = &found {
            tracing::warn!(reason = %reason, "{}", no_system_certs_message());
        }
        found
    })
}

/// True on a machine with no system certificates, where every https request fails.
pub fn no_system_certs() -> bool {
    matches!(trust(), Trust::Nothing { .. })
}

/// A client builder for srv's own requests: set what the site needs (timeouts, headers, proxy) and `build()`.
/// On a machine with no system certificates it trusts nothing, so `build()` still works.
pub fn builder() -> reqwest::ClientBuilder {
    agentmux_common::tls_provider::install_ring_provider();
    let builder = reqwest::Client::builder();
    match trust() {
        Trust::System => builder,
        Trust::Nothing { .. } => builder.tls_certs_only(Vec::<reqwest::Certificate>::new()),
    }
}

/// A client with reqwest's defaults, for a site that sets nothing. Never fails: the one way a build can fail with
/// nothing to trust is a missing TLS provider, which this module installs itself.
pub fn client() -> reqwest::Client {
    builder().build().unwrap_or_else(|e| {
        tracing::error!(error = %chain(&e), "building the HTTP client failed; using one that trusts nothing");
        distrusting_client()
    })
}

/// A client built from the site's own settings (timeouts, headers, redirect rules, proxy), for a site with nowhere to
/// return an error to. Never fails. The settings come as a function because a builder is used up by `build()`, and a
/// build that fails (the certificates the probe saw are gone since: a bundle removed or replaced while the server runs)
/// is tried again with the same settings and nothing to trust, so a site's redirect rule or `no_proxy` is never lost
/// and every https request still fails closed. Only if that fails too, [`client`] is the last step.
pub fn client_with(settings: impl Fn(reqwest::ClientBuilder) -> reqwest::ClientBuilder) -> reqwest::Client {
    match settings(builder()).build() {
        Ok(c) => c,
        Err(first) => {
            tracing::error!(error = %chain(&first), "building the HTTP client failed; trying again with no certificates to trust");
            agentmux_common::tls_provider::install_ring_provider();
            match settings(reqwest::Client::builder().tls_certs_only(Vec::<reqwest::Certificate>::new())).build() {
                Ok(c) => c,
                Err(second) => {
                    tracing::error!(error = %chain(&second), "the site's settings cannot build a client; using one with the defaults that trusts nothing");
                    distrusting_client()
                }
            }
        }
    }
}

/// The client that trusts nothing, built with no other setting: the last step when a build fails.
fn distrusting_client() -> reqwest::Client {
    reqwest::Client::builder().tls_certs_only(Vec::<reqwest::Certificate>::new()).build().unwrap_or_else(|e| {
        // Unreachable once the provider is installed: an empty set of roots is a valid configuration.
        panic!("the TLS provider is missing, so no HTTP client can be built: {}", chain(&e))
    })
}

/// What to say when an https request failed: on a machine with no system certificates and a failed certificate check,
/// that there are none and what to install; otherwise the error as it is.
pub fn explain(e: &reqwest::Error) -> String {
    explain_with(e, no_system_certs())
}

fn explain_with(e: &reqwest::Error, no_certs: bool) -> String {
    let https = e.url().is_some_and(|u| u.scheme() == "https");
    if no_certs && https && chain(e).contains("invalid peer certificate") {
        format!("{} ({})", no_system_certs_message(), chain(e))
    } else {
        e.to_string()
    }
}

#[cfg(test)]
#[path = "http_client_tests.rs"]
mod tests;
