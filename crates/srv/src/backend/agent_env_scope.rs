//! Which of the variables srv puts in an agent's environment reach a container agent, and how.
//!
//! One table, with a reason on each line, in place of the denylist `container_exec_env` used to apply
//! to a list written apart from the code that sets the variables (task #13666, step 2). A name srv sets
//! on an agent turn needs an entry here; `every_name_srv_sets_on_a_turn_has_a_scope` runs the code that
//! sets them and fails on one that has none, so a new variable is placed when it is added, not found
//! later when it turns up inside a container.
//!
//! What a name the agent's own `cmd:env` carries gets is not decided here: it passes through unchanged,
//! as it did.

/// Where a variable goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvScope {
    /// Native and container agents get it as it is.
    Everywhere,
    /// A native agent only: it names the host (a path, a user, a temp folder) and means nothing in an image.
    HostOnly,
    /// A container agent gets srv's own value for the container (a token, a URL it can reach, a build label);
    /// `container_exec_env` sets it, and it replaces any the agent's environment carried.
    Rewrite,
}

pub struct EnvSpec {
    pub name: &'static str,
    pub scope: EnvScope,
    pub why: &'static str,
}

const fn spec(name: &'static str, scope: EnvScope, why: &'static str) -> EnvSpec {
    EnvSpec { name, scope, why }
}

use EnvScope::{Everywhere, HostOnly, Rewrite};

/// The variables srv sets or decides on an agent's turn, by name.
pub const AGENT_ENV: &[EnvSpec] = &[
    // The agent's identity: the same in or out of a container.
    spec("AGENTMUX_AGENT_ID", Everywhere, "who the agent is, for MCP routing, memory and jekts"),
    spec("MUXBUS_AGENT_ID", Everywhere, "AGENTMUX_AGENT_ID under the prefix muxbus-client checks first"),
    spec("AGENTMUX_AGENT_DISPLAY", Everywhere, "the agent's display name"),
    spec("AGENTMUX_AGENT_SLUG", Everywhere, "the agent's slug"),
    spec("AGENTMUX_AGENT_UID", Everywhere, "the agent's row id"),
    spec("AGENTMUX_AGENT_TOKEN", Everywhere, "the agent's own token; a container's exec token is minted from it"),
    spec("AGENTMUX_AGENT_FORMER_NAMES", Everywhere, "the names the agent had before a rename"),
    spec("AGENTMUX_AGENT_FORMER_SLUGS", Everywhere, "the slugs the agent had before a rename"),
    spec("AGENTMUX_BLOCKID", Everywhere, "the pane the agent's tools speak for"),
    spec("GIT_AUTHOR_NAME", Everywhere, "git identity: the agent's id"),
    spec("GIT_AUTHOR_EMAIL", Everywhere, "git identity: the agent's id"),
    spec("GIT_COMMITTER_NAME", Everywhere, "git identity: the agent's id"),
    spec("GIT_COMMITTER_EMAIL", Everywhere, "git identity: the agent's id"),
    // Switches the agent CLI reads.
    spec("CLAUDE_CODE_HARBOR_KITE", Everywhere, "cli_messaging's gate for the CLI"),
    spec("CLAUDE_CODE_SUBAGENT_MODEL", Everywhere, "the model the CLI's subagents use"),
    // GitHub through the broker: the same hand-off in a container, where the image finds the hooks.
    spec("AGENTMUX_GITHUB_BROKER", Everywhere, "tells the shell wrappers to ask srv for a GitHub token"),
    spec("AGENTMUX_GIT_HOST", Everywhere, "the git host the broker serves"),
    spec("GIT_TERMINAL_PROMPT", Everywhere, "git never asks the terminal for a password"),
    // What a container is given in place of the host's.
    spec("AGENTMUX_AUTH_KEY", Rewrite, "the instance key is replaced by this block's container token"),
    spec("AGENTMUX_LOCAL_URL", Rewrite, "a container reaches srv at host.docker.internal or the agents network's gateway, not loopback"),
    spec("AGENTMUX_VERSION", Rewrite, "which AgentMux this is: a container exec does not go through pane_env, so srv adds it"),
    spec("AGENTMUX_BUILD", Rewrite, "which AgentMux this is: a container exec does not go through pane_env, so srv adds it"),
    spec("AGENTMUX_IN_CONTAINER", Rewrite, "a hint for the in-container MCP and bashwrap, which are not given signing keys"),
    spec("CHOKIDAR_USEPOLLING", Rewrite, "file watchers poll where the workspace is a file-sharing mount that sends no change events (Docker Desktop)"),
    spec("CHOKIDAR_INTERVAL", Rewrite, "as CHOKIDAR_USEPOLLING"),
    spec("WATCHPACK_POLLING", Rewrite, "as CHOKIDAR_USEPOLLING"),
    // Host paths and host users: the image has its own.
    spec("CLAUDE_CONFIG_DIR", HostOnly, "a host path: the image supplies its own, and the bound account is mounted instead"),
    spec("GH_CONFIG_DIR", HostOnly, "the gh guard's folder is a host path; the image controls its own gh state"),
    spec("PATH", HostOnly, "a host path list: the image has its own tools"),
    spec("HOME", HostOnly, "a host path: the image has its own home"),
    spec("USERPROFILE", HostOnly, "a host path: the image has its own home"),
    spec("HOMEDRIVE", HostOnly, "a host path: the image has its own home"),
    spec("HOMEPATH", HostOnly, "a host path: the image has its own home"),
    spec("TMPDIR", HostOnly, "a host temp folder"),
    spec("TEMP", HostOnly, "a host temp folder"),
    spec("TMP", HostOnly, "a host temp folder"),
    spec("AGENTMUX_AGENT_WORKDIR", HostOnly, "the host path of the agent's workspace; srv sets it for host agents only"),
    spec("CLAUDE_CODE_SHELL_PREFIX", HostOnly, "a host path; the image's own wrapper may predate prefix mode"),
];

/// Names that come in numbered or generated families.
const FAMILIES: &[(&str, EnvScope, &str)] = &[
    ("GIT_CONFIG_", Rewrite, "git settings: the publish guard's hooks path is a host path and is dropped, the container gets its own hooks"),
    ("AWS_", Rewrite, "the AWS SDK asks srv for credentials through the mounted config, where the host shares AWS with containers"),
    ("BASH_FUNC_", Everywhere, "the broker's gh and git shell functions"),
];

/// How `name` goes into a container, or `None` if srv has not placed it.
pub fn scope_of(name: &str) -> Option<EnvScope> {
    AGENT_ENV
        .iter()
        .find(|s| s.name == name)
        .map(|s| s.scope)
        .or_else(|| FAMILIES.iter().find(|(prefix, ..)| name.starts_with(prefix)).map(|(_, scope, _)| *scope))
}

/// Whether a container agent is not given `name` from the host's environment.
pub fn host_only(name: &str) -> bool {
    scope_of(name) == Some(HostOnly)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeSet, HashMap};

    #[test]
    fn every_entry_has_a_reason_and_no_name_twice() {
        let mut seen = BTreeSet::new();
        for s in AGENT_ENV {
            assert!(!s.why.trim().is_empty(), "{} needs a reason", s.name);
            assert!(seen.insert(s.name), "{} is listed twice", s.name);
        }
    }

    /// What srv sets on a turn's environment, from the code that sets it. The agent's own `cmd:env` is the
    /// `base` and is not counted.
    fn names_srv_sets(base: &HashMap<String, String>) -> BTreeSet<String> {
        use crate::server::agent_handlers::{identity_env, input};
        let mut env = base.clone();
        let identity = input::PersistedAgentIdentity {
            uid: "11111111-2222-3333-4444-555555555555".into(),
            slug: Some("fixture-agent".into()),
            token: Some("tok".into()),
            former: Default::default(),
        };
        input::carry_agent_uid_env(&mut env, Some(&identity));
        identity_env::carry_slug_identity_env(&mut env, "fixture-agent");
        let mut meta = crate::backend::obj::MetaMapType::default();
        meta.insert("cmd:cwd".into(), serde_json::json!(std::env::temp_dir().to_string_lossy()));
        input::carry_agent_workdir_env(&mut env, &meta);
        env.insert("AGENTMUX_AUTH_KEY".into(), "k".into());
        env.insert("AGENTMUX_BLOCKID".into(), "b".into());
        crate::backend::tool_store::prepend_agent_tools_path(&mut env);
        crate::backend::gh_guard::apply_gh_guard(&mut env);
        crate::backend::account_login_guard::strip_account_login(&mut env);
        // What the container side adds to or does with the same map.
        let out: HashMap<String, String> = crate::backend::container_credential::container_exec_env("scope-block", &env)
            .into_iter()
            .collect();
        env.keys().chain(out.keys()).filter(|k| !base.contains_key(*k)).cloned().collect()
    }

    #[test]
    fn every_name_srv_sets_on_a_turn_has_a_scope() {
        let base: HashMap<String, String> = [("MY_OWN_SETTING".to_string(), "1".to_string())].into();
        let unplaced: Vec<_> = names_srv_sets(&base).into_iter().filter(|n| scope_of(n).is_none()).collect();
        assert!(unplaced.is_empty(), "add these to AGENT_ENV in backend/agent_env_scope.rs, with a scope and a reason: {unplaced:?}");
    }

    #[test]
    fn a_host_only_name_is_not_given_to_a_container() {
        let env: HashMap<String, String> =
            AGENT_ENV.iter().filter(|s| s.scope == HostOnly).map(|s| (s.name.to_string(), "host".to_string())).collect();
        let out: HashMap<String, String> = crate::backend::container_credential::container_exec_env("scope-block", &env)
            .into_iter()
            .collect();
        let leaked: Vec<_> = env.keys().filter(|k| out.contains_key(*k)).collect();
        assert!(leaked.is_empty(), "host-only names reached a container: {leaked:?}");
    }
}
