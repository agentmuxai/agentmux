//! A scoped credential for container agents, in place of the instance auth key.
//!
//! A container agent is meant to see only its `/workspace` mount, but the
//! instance key (`AGENTMUX_AUTH_KEY`) reaches every route srv serves: host
//! shell creation, `stream-local-file`, editor reads, attachment ingest by
//! path. Its `docker exec` env therefore carries a token minted here instead,
//! which srv accepts only on [`container_route_allowed`] and attributes to the
//! agent it was minted for.

use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// Prefix that marks a container token in logs and bug reports.
pub const TOKEN_PREFIX: &str = "amxc_";

/// What a container token was minted for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContainerGrant {
    pub block_id: String,
    /// The agent's own `AGENTMUX_AGENT_TOKEN` when the token was last handed
    /// out. `caller_middleware` attributes every request to it, ignoring any
    /// `X-Agent-Token` header the container sends.
    pub agent_token: Option<String>,
}

#[derive(Default)]
struct Registry {
    by_digest: HashMap<[u8; 32], ContainerGrant>,
    by_block: HashMap<String, String>,
}

fn registry() -> &'static Mutex<Registry> {
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    REGISTRY.get_or_init(Default::default)
}

// Looked up by digest so a lookup's timing depends on the digest, not on how
// much of a guessed token matches a real one.
fn digest(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

fn mint() -> String {
    format!(
        "{TOKEN_PREFIX}{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

/// The block's container token: minted on first use, reused on later turns,
/// with the grant's identity refreshed from this turn's env.
pub fn token_for_block(block_id: &str, agent_token: Option<&str>) -> String {
    let mut reg = registry().lock().unwrap();
    let token = reg.by_block.get(block_id).cloned().unwrap_or_else(mint);
    reg.by_block.insert(block_id.to_string(), token.clone());
    reg.by_digest.insert(
        digest(&token),
        ContainerGrant {
            block_id: block_id.to_string(),
            agent_token: agent_token.map(str::trim).filter(|t| !t.is_empty()).map(str::to_string),
        },
    );
    token
}

pub fn grant_for(token: &str) -> Option<ContainerGrant> {
    if !token.starts_with(TOKEN_PREFIX) {
        return None;
    }
    registry().lock().unwrap().by_digest.get(&digest(token)).cloned()
}

/// Invalidate the block's token; its container can no longer reach srv.
pub fn revoke_block(block_id: &str) {
    let mut reg = registry().lock().unwrap();
    if let Some(token) = reg.by_block.remove(block_id) {
        reg.by_digest.remove(&digest(&token));
    }
}

/// The routes a container agent may call. Everything else is refused, notably
/// host shell/pty creation, file streaming, editor and attachment paths, pane
/// and agent opening, UI automation, cron, and the `/ws` and
/// `/agentmux/service` RPC surfaces. Matched on the raw path, exactly as the
/// router matches it.
pub fn container_route_allowed(path: &str) -> bool {
    const EXACT: &[&str] = &[
        "/agentmux/reactive/inject",
        "/agentmux/reactive/agent",
        "/agentmux/reactive/agent-names",
        "/agentmux/discovery",
        "/agentmux/work",
        "/agentmux/work/claim",
        "/api/v1/self",
        "/api/v1/agent/memory/list",
        "/api/v1/agent/memory/read",
        "/api/v1/agent/memory/write",
        "/api/v1/agent/memory/history",
        "/api/v1/agent/memory/diff",
        "/api/v1/agent/memory/revert",
        // Read-only: Global Memory is composed into every agent's context at
        // launch, so writes from a sandbox would reach host agents.
        "/api/v1/agent/globalmemory/list",
        "/api/v1/agent/globalmemory/read",
        "/api/v1/agent/globalmemory/history",
        "/api/v1/agent/globalmemory/diff",
        "/api/v1/agent/self/quit",
        "/api/v1/agent/pane/close",
    ];
    if EXACT.contains(&path) {
        return true;
    }
    let is_id = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if let Some(rest) = path.strip_prefix("/agentmux/work/") {
        return match rest.split('/').collect::<Vec<_>>().as_slice() {
            [id] => is_id(id),
            [id, "heartbeat" | "complete" | "release"] => is_id(id),
            _ => false,
        };
    }
    if let Some(id) = path.strip_prefix("/api/v1/agent/shutdown/") {
        return is_id(id);
    }
    false
}

/// Routes that act on the calling agent's own data, and so are refused to a
/// container token that carries no agent identity rather than falling back to
/// an agent named in the request body.
pub fn route_needs_identity(path: &str) -> bool {
    path.starts_with("/api/v1/agent/memory/")
}

/// A container turn's `docker exec` env: the container denylist applied, and
/// the instance key replaced by this block's container token.
pub fn container_exec_env(block_id: &str, env_vars: &HashMap<String, String>) -> Vec<(String, String)> {
    let agent_token = env_vars.get("AGENTMUX_AGENT_TOKEN").map(String::as_str);
    // The publish guard's hooks directory is a host path; the agent's other
    // GIT_CONFIG_* settings still apply inside the container.
    let env_vars = crate::backend::publish_guard::without_hooks_path(env_vars);
    env_vars
        .iter()
        .filter(|(k, _)| !crate::backend::container::CONTAINER_ENV_DENYLIST.contains(&k.as_str()))
        .map(|(k, v)| {
            if k == "AGENTMUX_AUTH_KEY" {
                (k.clone(), token_for_block(block_id, agent_token))
            } else {
                (k.clone(), v.clone())
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_reused_per_block_and_revocable() {
        let a = token_for_block("cc-block-a", Some("tok-a"));
        assert!(a.starts_with(TOKEN_PREFIX));
        assert_eq!(token_for_block("cc-block-a", Some("tok-a2")), a);
        assert_ne!(token_for_block("cc-block-b", None), a);
        let grant = grant_for(&a).unwrap();
        assert_eq!(grant.block_id, "cc-block-a");
        assert_eq!(grant.agent_token.as_deref(), Some("tok-a2"));
        revoke_block("cc-block-a");
        assert_eq!(grant_for(&a), None);
        assert_ne!(token_for_block("cc-block-a", None), a);
    }

    #[test]
    fn unknown_and_unprefixed_tokens_have_no_grant() {
        assert_eq!(grant_for("amxc_not-a-real-token"), None);
        assert_eq!(grant_for("0b8e0c52-1b4e-4d3f-9a53-8a0c1d2e3f40"), None);
        assert_eq!(grant_for(""), None);
    }

    #[test]
    fn blank_agent_token_counts_as_none() {
        let t = token_for_block("cc-block-blank", Some("  "));
        assert_eq!(grant_for(&t).unwrap().agent_token, None);
    }

    #[test]
    fn allowlist_admits_agent_routes_only() {
        for ok in [
            "/agentmux/reactive/inject",
            "/agentmux/discovery",
            "/agentmux/work",
            "/agentmux/work/claim",
            "/agentmux/work/7f3a/heartbeat",
            "/agentmux/work/7f3a",
            "/api/v1/self",
            "/api/v1/agent/memory/write",
            "/api/v1/agent/globalmemory/read",
            "/api/v1/agent/pane/close",
            "/api/v1/agent/shutdown/req-12",
        ] {
            assert!(container_route_allowed(ok), "{ok} should be allowed");
        }
        for denied in [
            "/api/v1/shell/create",
            "/api/v1/ptyshell/create",
            "/agentmux/stream-local-file",
            "/agentmux/service",
            "/ws",
            "/api/v1/pane/open",
            "/api/v1/agent/open",
            "/api/v1/attachments/upload",
            "/api/v1/ui/browser/eval",
            "/api/v1/ui/screenshot",
            "/agentmux/cron",
            "/api/v1/fleet/bulk-stop",
            "/api/v1/agent/globalmemory/write",
            "/api/v1/agent/globalmemory/remove",
            "/agentmux/reactive/agents",
            "/agentmux/work/7f3a/steal",
            "/agentmux/work/../service",
            "/api/v1/agent/shutdown/",
            "/api/v1/self/",
            "//api/v1/self",
        ] {
            assert!(!container_route_allowed(denied), "{denied} should be refused");
        }
    }

    #[test]
    fn exec_env_swaps_the_instance_key_for_the_container_token() {
        let env: HashMap<String, String> = [
            ("AGENTMUX_AUTH_KEY", "instance-key-value"),
            ("AGENTMUX_AGENT_TOKEN", "agent-tok"),
            ("AGENTMUX_LOCAL_URL", "http://host.docker.internal:1"),
            ("HOME", "/host/home"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        let out: HashMap<_, _> = container_exec_env("cc-block-env", &env).into_iter().collect();
        let token = &out["AGENTMUX_AUTH_KEY"];
        assert!(token.starts_with(TOKEN_PREFIX));
        assert!(!out.values().any(|v| v == "instance-key-value"));
        assert_eq!(grant_for(token).unwrap().agent_token.as_deref(), Some("agent-tok"));
        assert_eq!(out["AGENTMUX_LOCAL_URL"], "http://host.docker.internal:1");
        assert!(!out.contains_key("HOME"), "the denylist still applies");
    }

    #[test]
    fn exec_env_without_a_key_gets_no_token() {
        let env: HashMap<String, String> = [("FOO".to_string(), "bar".to_string())].into_iter().collect();
        let out = container_exec_env("cc-block-nokey", &env);
        assert!(out.iter().all(|(k, _)| k != "AGENTMUX_AUTH_KEY"));
    }
}
