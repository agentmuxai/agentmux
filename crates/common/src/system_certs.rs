// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What is said when a Linux machine has no system certificates, in one place so that `agentmux doctor`, the server's
//! log and the error a failed https request carries all use the same words.

/// What is wrong: every https request the server makes fails, because there is nothing to check a server's certificate against.
pub const NO_SYSTEM_CERTS: &str = "no system certificates found: every https request the server makes (models, updates, connected accounts) would fail";

/// What to do about it.
pub const NO_SYSTEM_CERTS_FIX: &str = "install the ca-certificates package (Debian and Ubuntu: `apt install ca-certificates`; Fedora: `dnf install ca-certificates`; Alpine: `apk add ca-certificates`), or point SSL_CERT_FILE at a bundle, then restart AgentMux (the server reads the certificates once, when it starts)";

/// The two together, as one sentence for a log line or an error.
pub fn no_system_certs_message() -> String {
    format!("{NO_SYSTEM_CERTS}. To fix it, {NO_SYSTEM_CERTS_FIX}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_message_names_the_problem_and_the_package() {
        let m = no_system_certs_message();
        assert!(m.starts_with("no system certificates found"));
        assert!(m.contains("ca-certificates") && m.contains("SSL_CERT_FILE"));
    }
}
