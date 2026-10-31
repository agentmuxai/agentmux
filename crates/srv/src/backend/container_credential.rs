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

/// Whether a container agent may call `path`. Every route is open to it except the ones
/// [`crate::backend::container_routes`] refuses, with a reason each: commands on the host, the host's
/// files outside its mounts, the app's own plumbing. Matched on the raw path, exactly as the router
/// matches it.
pub fn container_route_allowed(path: &str) -> bool {
    crate::backend::container_routes::refusal(path).is_none()
}

/// The routes a container token has always been accepted on. They work for a grant that carries no
/// agent identity (they bind to the grant's own block, or to nobody in particular).
fn accepted_without_identity(path: &str) -> bool {
    const EXACT: &[&str] = &[
        "/agentmux/reactive/inject",
        "/agentmux/reactive/agent",
        "/agentmux/reactive/agent-names",
        "/agentmux/discovery",
        "/agentmux/work",
        "/agentmux/work/claim",
        "/api/v1/self",
        "/api/v1/agent/self/quit",
        "/api/v1/agent/pane/close",
        "/api/v1/agent/tool-chunk",
    ];
    let is_id = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    EXACT.contains(&path)
        || path == "/"
        || path == "/health"
        || ["/docsite/", "/schema/", "/agentmux/widget-sdk/"].iter().any(|p| path.starts_with(p))
        || path.strip_prefix("/agentmux/work/").is_some_and(|rest| match rest.split('/').collect::<Vec<_>>().as_slice() {
            [id] => is_id(id),
            [id, "heartbeat" | "complete" | "release" | "update" | "ask" | "submit"] => is_id(id),
            _ => false,
        })
        || path.strip_prefix("/api/v1/agent/shutdown/").is_some_and(is_id)
}

/// Routes that act on the calling agent's own data, and so are refused to a
/// container token that carries no agent identity rather than falling back to
/// an agent named in the request body. Every route a container has been given since the
/// denylist (`container_routes`) opened them all does: a handler that takes the agent from
/// the caller serves it, one that takes it from the body does not. The memory routes always
/// did, the session-start delivery included.
pub fn route_needs_identity(path: &str) -> bool {
    path.starts_with("/api/v1/agent/memory/") || !accepted_without_identity(path)
}

/// File watchers poll where the workspace is a file-sharing mount that sends no change events:
/// Docker Desktop on Windows and macOS (a 9p or virtiofs share). On Linux a bind mount reports changes
/// and polling would only cost CPU. What the agent's own environment already sets is left alone.
pub(crate) fn watcher_polling_env(shared_mount: bool, set: &[(String, String)]) -> Vec<(String, String)> {
    if !shared_mount {
        return Vec::new();
    }
    [("CHOKIDAR_USEPOLLING", "1"), ("CHOKIDAR_INTERVAL", "300"), ("WATCHPACK_POLLING", "true")]
        .into_iter()
        .filter(|(k, _)| !set.iter().any(|(have, _)| have == k))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// A container turn's `docker exec` env: the container denylist applied, and
/// the instance key replaced by this block's container token.
pub fn container_exec_env(block_id: &str, env_vars: &HashMap<String, String>) -> Vec<(String, String)> {
    let agent_token = env_vars.get("AGENTMUX_AGENT_TOKEN").map(String::as_str);
    // The publish guard's hooks directory is a host path; the agent's other
    // GIT_CONFIG_* settings still apply inside the container.
    let env_vars = crate::backend::publish_guard::without_hooks_path(env_vars);
    // A host agent's git setup is for the host; a container gets its own where tokens can be given
    // (`github_tokens::container_git_hooks`), with the hooks where the image finds them.
    let mut env_vars = crate::identity::github_tokens::without_agent_git(&env_vars);
    if crate::identity::github_tokens::container_git_hooks().is_some() {
        crate::identity::github_tokens::apply_container_git(&mut env_vars);
    }
    let build_info = crate::backend::pane_env::build_info();
    let is_build_info = |k: &str| build_info.iter().any(|(b, _)| *b == k);
    let mut out: Vec<(String, String)> = env_vars
        .iter()
        .filter(|(k, _)| !crate::backend::container::CONTAINER_ENV_DENYLIST.contains(&k.as_str()))
        .filter(|(k, _)| !is_build_info(k))
        .map(|(k, v)| {
            if k == "AGENTMUX_AUTH_KEY" {
                (k.clone(), token_for_block(block_id, agent_token))
            } else {
                (k.clone(), v.clone())
            }
        })
        .collect();
    // Which AgentMux this is, as every host pane is told: a container exec
    // doesn't go through `pane_env`'s sanitizers, and srv's values win over
    // any the agent's own env carried.
    out.extend(build_info.iter().map(|(k, v)| (k.to_string(), v.clone())));
    // A hint for the client only: the in-container MCP doesn't ask srv for signing keys it is refused. srv
    // never trusts this variable for anything (it comes from the container's own environment).
    out.push(("AGENTMUX_IN_CONTAINER".to_string(), "1".to_string()));
    out.extend(watcher_polling_env(cfg!(any(windows, target_os = "macos")), &out));
    // The AWS SDK and CLI ask srv for credentials through the mounted config, where the host shares AWS with
    // container agents (`identity::aws_broker`). Its variables replace any of the same names the agent's own
    // environment carried; the rest of the agent's `AWS_*` is its own business.
    let aws = crate::identity::aws_broker::container::exec_env();
    out.retain(|(k, _)| !aws.iter().any(|(a, _)| a == k));
    out.extend(aws);
    // On Linux a container cannot reach srv's loopback; when srv listens on the agents network's
    // gateway that is where the container is pointed.
    crate::backend::container_listener::point_env_at_listener(&mut out);
    out
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
    fn a_container_agent_has_the_routes_a_native_agent_has_except_the_hosts() {
        for ok in [
            "/agentmux/reactive/inject",
            "/agentmux/discovery",
            "/agentmux/work",
            "/agentmux/work/claim",
            "/agentmux/work/7f3a/heartbeat",
            "/api/v1/self",
            "/api/v1/agent/memory/write",
            "/api/v1/agent/globalmemory/read",
            "/api/v1/agent/pane/close",
            "/api/v1/agent/shutdown/req-12",
            "/api/v1/agent/tool-chunk",
            "/api/v1/agent/dev_server/register",
            "/api/v1/work-facts/who",
            "/api/v1/vault",
            "/api/v1/ui/browser/open",
            "/api/v1/ui/browser/set_files",
            "/api/v1/ui/screenshot",
            "/api/v1/ui/shortcuts/list",
            "/agentmux/cron",
            "/api/v1/agent/open",
            "/api/v1/attachments/upload",
            "/api/v1/pane/open",
            "/api/v1/widgets/install",
        ] {
            assert!(container_route_allowed(ok), "{ok} should be allowed");
        }
        for denied in [
            "/api/v1/shell/create",
            "/api/v1/ptyshell/create",
            "/agentmux/stream-local-file",
            "/agentmux/service",
            "/ws",
            "/api/v1/host/browser_bypass",
            "/api/v1/agent/desktop/authorize",
            "/api/v1/agent/globalmemory/write",
            "/api/v1/agent/globalmemory/remove",
            "/api/v1/agent/globalmemory/reach",
            "/agentmux/wps/publish",
            "/agentmux/jobs/stop-run",
            "/agentmux/agents/self/keys",
            "/api/v1/agent/self/mcp-beat",
        ] {
            assert!(!container_route_allowed(denied), "{denied} should be refused");
        }
    }

    #[test]
    fn only_the_routes_a_container_always_had_work_without_an_identity() {
        for old in ["/agentmux/reactive/inject", "/agentmux/work/claim", "/agentmux/work/7f3a/ask", "/api/v1/self", "/api/v1/agent/tool-chunk", "/health"] {
            assert!(!route_needs_identity(old), "{old}");
        }
        for new in ["/api/v1/agent/memory/session-start/part", "/api/v1/agent/memory/write", "/api/v1/agent/dev_server/register", "/api/v1/github/token", "/api/v1/vault", "/api/v1/ui/browser/open", "/api/v1/ui/screenshot", "/agentmux/cron", "/api/v1/agent/open", "/agentmux/work/7f3a/hold"] {
            assert!(route_needs_identity(new), "{new}");
        }
    }

    #[test]
    fn file_watchers_poll_only_where_the_workspace_is_a_file_sharing_mount() {
        assert!(watcher_polling_env(false, &[]).is_empty(), "a Linux bind mount reports changes");
        let on = watcher_polling_env(true, &[]);
        let get = |k: &str| on.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        assert_eq!((get("CHOKIDAR_USEPOLLING"), get("CHOKIDAR_INTERVAL"), get("WATCHPACK_POLLING")), (Some("1"), Some("300"), Some("true")));
        // The agent's own choice wins.
        let own = watcher_polling_env(true, &[("CHOKIDAR_USEPOLLING".into(), "0".into())]);
        assert!(own.iter().all(|(k, _)| k != "CHOKIDAR_USEPOLLING") && own.len() == 2);
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
        assert_eq!(out["AGENTMUX_IN_CONTAINER"], "1", "the hint the in-container MCP reads");
        assert!(
            !out.keys().any(|k| k.ends_with("_JEKT_KEY") || k.ends_with("_LAN_KEY") || k.ends_with("_WAN_KEY")),
            "and no signing key"
        );
    }

    #[test]
    fn exec_env_without_a_key_gets_no_token() {
        let env: HashMap<String, String> = [("FOO".to_string(), "bar".to_string())].into_iter().collect();
        let out = container_exec_env("cc-block-nokey", &env);
        assert!(out.iter().all(|(k, _)| k != "AGENTMUX_AUTH_KEY"));
    }

    #[test]
    fn a_container_agent_is_told_which_build_it_runs_in() {
        let env: HashMap<String, String> = [("AGENTMUX_VERSION".to_string(), "0.0.1-stale".to_string())].into_iter().collect();
        let out = container_exec_env("cc-block-build", &env);
        let versions: Vec<&str> = out.iter().filter(|(k, _)| k == "AGENTMUX_VERSION").map(|(_, v)| v.as_str()).collect();
        assert_eq!(versions, [env!("CARGO_PKG_VERSION")], "srv's value, once, over the agent's own");
        assert!(out.iter().any(|(k, _)| k == "AGENTMUX_BUILD"));
    }
}
