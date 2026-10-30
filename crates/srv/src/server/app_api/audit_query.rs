// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `audit.query` (`backend::audit_query`): a page of the Audit view's list. The window's, not an agent API: the
//! Audit is the operator's view of every agent, so a connection registered as an agent is refused.

use super::*;
use crate::backend::audit_query::{self, Ctx, Sources};

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let state = state.clone();
    engine.register_typed(COMMAND_AUDIT_QUERY, move |req: AuditQueryReq, ctx| {
        let state = state.clone();
        async move {
            ctx.operator_only("the Audit is the AgentMux window's, not an agent API")?;
            // Reads files, the whole segment ledger and the registry: off the async workers.
            tokio::task::spawn_blocking(move || {
                let slugs = state.mstore.shared_def_registry().and_then(|reg| reg.slug_index().ok());
                let slug_of_uid = |uid: &str| slugs.as_ref().and_then(|s| s.get(uid)).map(|e| e.slug.clone());
                let uid_of_slug = |slug: &str| {
                    let all = slugs.as_ref()?;
                    let by_slug = all.iter().find(|(_, e)| e.slug.eq_ignore_ascii_case(slug));
                    by_slug.or_else(|| all.iter().find(|(_, e)| e.former.iter().any(|f| f.eq_ignore_ascii_case(slug)))).map(|(uid, _)| uid.clone())
                };
                let pane_of_agent = |slug: &str| match state.reactive_handler.lookup_by_name(slug) {
                    crate::backend::reactive::handler::LookupOutcome::One(reg) => Some(reg.block_id).filter(|b| !b.is_empty()),
                    _ => None,
                };
                let block_exists = |id: &str| state.mstore.get::<crate::backend::obj::Block>(id).ok().flatten().is_some();
                let file = crate::backend::reactive::audit_store::default_path();
                let src = Sources {
                    jekt_file: file.as_deref(),
                    segments: state.global_transcript_store.as_deref(),
                    ctx: Ctx { slug_of_uid: &slug_of_uid, uid_of_slug: &uid_of_slug, pane_of_agent: &pane_of_agent, block_exists: &block_exists },
                };
                audit_query::query(&src, &req)
            })
            .await
            .map_err(|e| format!("audit.query: {e}"))?
        }
    });
}
