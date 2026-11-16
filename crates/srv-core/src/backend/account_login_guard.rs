// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! An agent process must not hold the account's cloud login.
//!
//! The srv used to put the user's muxbus access token in every agent's env
//! (`MUXBUS_TOKEN`, with `MUXBUS_COGNITO_DOMAIN`). Nothing in AgentMux reads
//! it there: the srv makes every cloud call with its own stored login, and
//! agents reach the cloud through the srv. What the variable did give an
//! agent is the power to act as the account: publish WAN keys for an install
//! it made up and sign jekts as any agent name, which then verify
//! (`SPEC_WAN_JEKT_VERIFICATION_2026_09_24.md` §6.4). That is why a verified
//! WAN sender couldn't be trusted without a human approving its install.
//!
//! [`strip_account_login`] is called on every path that spawns an agent
//! process, next to `gh_guard::apply_gh_guard`, and removes the variables
//! wherever they came from — a persisted `cmd:env` from before this change,
//! or a caller's override. Like `GH_CONFIG_DIR`, they are reserved.
//!
//! Limit: agents run as the same OS user as the srv, so one that goes
//! looking can still read the srv's stored login from disk (§6.5). This
//! takes the login out of the agent's hands by default; it is not an OS
//! boundary.

use std::collections::HashMap;

/// The account-login variables no agent env may carry.
pub const ACCOUNT_LOGIN_VARS: &[&str] = &["MUXBUS_TOKEN", "MUXBUS_COGNITO_DOMAIN"];

/// Remove every [`ACCOUNT_LOGIN_VARS`] entry from `env_vars`.
pub fn strip_account_login(env_vars: &mut HashMap<String, String>) {
    for var in ACCOUNT_LOGIN_VARS {
        env_vars.remove(*var);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_account_login_is_removed_and_nothing_else() {
        let mut env: HashMap<String, String> = [
            ("MUXBUS_TOKEN", "eyJ.access"),
            ("MUXBUS_COGNITO_DOMAIN", "https://auth.example"),
            ("MUXBUS_AGENT_ID", "agenty"),
            ("AGENTMUX_AGENT_ID", "agenty"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        strip_account_login(&mut env);
        assert!(!env.contains_key("MUXBUS_TOKEN"));
        assert!(!env.contains_key("MUXBUS_COGNITO_DOMAIN"));
        assert_eq!(env.get("MUXBUS_AGENT_ID").map(String::as_str), Some("agenty"), "routing id stays");
        assert_eq!(env.len(), 2);
    }

    #[test]
    fn an_env_without_it_is_unchanged() {
        let mut env: HashMap<String, String> = [("PATH".to_string(), "/bin".to_string())].into_iter().collect();
        strip_account_login(&mut env);
        assert_eq!(env.len(), 1);
    }
}
