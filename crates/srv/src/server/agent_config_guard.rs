// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Who `writeagentconfig` may write for. The AgentMux window, proven by its host, writes any
//! agent's launch files; a connection registered as an agent (`bus:register`) writes only its
//! own, into its own workspace; any other connection writes nothing. The file lands where the
//! caller says and the `.mcp.json` it carries signs as the named agent, so without this an
//! agent could drop files into another agent's workspace or into any folder it can name. That
//! includes an agent that never registers: every agent holds the instance auth key, which opens
//! a connection that is neither the window nor an agent. A leader's edit of another agent's
//! definition goes through the definition API (which records who made it), not through this
//! raw write.

use std::path::Path;

use crate::backend::rpc::actor::{actor_of, Actor};
use crate::backend::rpc_types::RpcContext;
use crate::backend::storage::store::Store;

/// `Ok` if `ctx` may write `cmd_agent_id`'s launch files into `working_dir`: the proven window
/// anywhere, an agent only its own workspace, an unproven connection nowhere.
pub(crate) fn check_agent_config_write(
    store: &Store,
    ctx: &RpcContext,
    cmd_agent_id: Option<&str>,
    working_dir: &Path,
    auto_allocate: bool,
) -> Result<(), String> {
    match actor_of(ctx) {
        Actor::Operator => return Ok(()),
        Actor::Unproven => {
            return Err("FORBIDDEN: only the AgentMux window, or an agent for its own workspace, writes agent config (this connection is neither)".to_string())
        }
        Actor::Agent(_) => {}
    }
    let own_only = "FORBIDDEN: an agent writes only its own config, into its own workspace";
    let named = cmd_agent_id.map(str::trim).filter(|id| !id.is_empty()).ok_or(own_only)?;
    crate::server::app_api::check_s1(store, ctx, named).map_err(|_| own_only.to_string())?;
    // `auto_allocate` makes a sibling folder `<dir>-N`, which is outside the workspace.
    if auto_allocate {
        return Err(own_only.to_string());
    }
    let id = crate::backend::agent_resolve::resolve_agent_id(store, &ctx.agent_id).map_err(|_| own_only.to_string())?;
    let workspace = store
        .agent_def_get(&id)
        .ok()
        .flatten()
        .map(|def| def.working_directory)
        .filter(|dir| !dir.is_empty())
        .ok_or("FORBIDDEN: this agent has no workspace on record to write into")?;
    let inside = match (std::fs::canonicalize(working_dir), std::fs::canonicalize(expand(&workspace))) {
        (Ok(dir), Ok(ws)) => dir.starts_with(ws),
        _ => false,
    };
    if inside { Ok(()) } else { Err(own_only.to_string()) }
}

fn expand(path: &str) -> std::path::PathBuf {
    crate::backend::base::expand_home_dir_safe(path)
}

#[cfg(test)]
mod tests {
    use super::check_agent_config_write;
    use crate::backend::rpc_types::RpcContext;
    use crate::backend::storage::agents::test_agent_def;

    fn agent(slug: &str) -> RpcContext {
        RpcContext { agent_id: slug.into(), ..Default::default() }
    }

    fn store_with(slug: &str, workspace: &std::path::Path) -> crate::backend::storage::store::Store {
        let store = crate::backend::storage::store::Store::open_in_memory().unwrap();
        let mut def = test_agent_def(slug, slug, "claude", "host", 1, "");
        def.working_directory = workspace.to_string_lossy().to_string();
        store.agent_def_insert(&mut def).unwrap();
        store
    }

    #[test]
    fn an_agent_writes_only_its_own_workspace() {
        let tmp = tempfile::tempdir().unwrap();
        let (mine, other) = (tmp.path().join("mine"), tmp.path().join("other"));
        std::fs::create_dir_all(mine.join("sub")).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        let store = store_with("mine", &mine);

        let ok = |dir: &std::path::Path, id: &str, alloc| check_agent_config_write(&store, &agent("mine"), Some(id), dir, alloc);
        assert!(ok(&mine, "mine", false).is_ok());
        assert!(ok(&mine.join("sub"), "mine", false).is_ok());
        assert!(ok(&other, "mine", false).is_err(), "another agent's workspace");
        assert!(ok(&mine.join("..").join("other"), "mine", false).is_err(), "out through ..");
        assert!(ok(&mine, "victim", false).is_err(), "files that sign as another agent");
        assert!(ok(&mine, "", false).is_err(), "no agent named");
        assert!(ok(&mine, "mine", true).is_err(), "auto_allocate makes a sibling folder");
        assert!(ok(&mine.join("not-yet"), "mine", false).is_err(), "a folder that doesn't exist");
    }

    #[test]
    fn the_proven_window_is_left_alone_and_a_connection_that_is_neither_it_nor_an_agent_writes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with("mine", tmp.path());
        assert!(check_agent_config_write(&store, &RpcContext::operator_for_tests(), Some("victim"), tmp.path(), true).is_ok());
        let e = check_agent_config_write(&store, &RpcContext::default(), Some("mine"), tmp.path(), false).unwrap_err();
        assert!(e.starts_with("FORBIDDEN"), "{e}");
    }
}
