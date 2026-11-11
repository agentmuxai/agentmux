// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! An agent's own statement of which install hosts it, for a LAN peer's pin
//! (`docs/specs/SPEC_LAN_KEY_CONTINUITY_2026_10_29.md`). A certificate shows only that some trusted
//! install wrapped the agent's public key in one, which any install of the account can do for a key
//! it has seen. This statement is signed with the agent's own LAN key, so only the host that holds
//! the private half can make it.

use crate::jekt_sign::{ed25519_sign_b64, ed25519_verify_b64, FIELD_SEP};

/// Domain separator for the statement. Distinct from every other signed material, so none of their
/// signatures can stand in for one.
const LAN_INSTALL_BINDING_DOMAIN: &str = "amx-lan-install-binding-v1";

fn material(agent_id: &str, instance_id: &str, issued_at: i64) -> String {
    format!("{LAN_INSTALL_BINDING_DOMAIN}{FIELD_SEP}{}{FIELD_SEP}{instance_id}{FIELD_SEP}{issued_at}", agent_id.to_ascii_lowercase())
}

/// The agent's statement, signed with its LAN key, that the install `instance_id` hosts it as of
/// `issued_at` (the time of the install's certificate for the key). `None` for a malformed private key.
pub fn sign_lan_install_binding(private_key: &[u8], agent_id: &str, instance_id: &str, issued_at: i64) -> Option<String> {
    ed25519_sign_b64(private_key, &material(agent_id, instance_id, issued_at))
}

/// Whether `sig_b64` is the agent's statement (see [`sign_lan_install_binding`]) under its public key.
pub fn verify_lan_install_binding(public_key: &[u8; 32], agent_id: &str, instance_id: &str, issued_at: i64, sig_b64: &str) -> bool {
    ed25519_verify_b64(public_key, &material(agent_id, instance_id, issued_at), sig_b64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_statement_names_the_agent_the_install_and_the_time() {
        let (public, private) = crate::jekt_sign::generate_lan_keypair([4u8; 32]);
        let sig = sign_lan_install_binding(&private, "Maop", "inst1", 100).unwrap();
        assert!(verify_lan_install_binding(&public, "maop", "inst1", 100, &sig));
        assert!(!verify_lan_install_binding(&public, "lark", "inst1", 100, &sig));
        assert!(!verify_lan_install_binding(&public, "maop", "inst2", 100, &sig));
        assert!(!verify_lan_install_binding(&public, "maop", "inst1", 101, &sig));
        let (other, _) = crate::jekt_sign::generate_lan_keypair([5u8; 32]);
        assert!(!verify_lan_install_binding(&other, "maop", "inst1", 100, &sig), "another key's statement is not this agent's");
        assert!(sign_lan_install_binding(&[1u8; 5], "maop", "inst1", 100).is_none());
    }
}
