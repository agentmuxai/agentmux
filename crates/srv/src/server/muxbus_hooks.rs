// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The server's side of `muxbus::server_hooks`: the relay's delivery path calls these through the
//! hook, so `muxbus` does not name the jekt modules or `reactive` here.

use std::sync::Arc;

use agentmux_common::jekt_files::CloudFile;

use crate::backend::reactive::handler::ReactiveHandler;
use crate::backend::reactive::types::InjectionRequest;
use crate::backend::storage::store::Store;
use crate::muxbus::server_hooks::{BoxFuture, ServerHooks};

struct Hooks;

static HOOKS: Hooks = Hooks;

/// The server's steps, for `muxbus::server_hooks::install`.
pub(crate) fn hooks() -> &'static dyn ServerHooks {
    &HOOKS
}

/// Register them. Called once at startup, before the relay subscriber delivers anything.
pub(crate) fn install() {
    crate::muxbus::server_hooks::install(hooks());
}

impl ServerHooks for Hooks {
    fn hold_wan_for_unavailable(&self, mstore: &Store, handler: &ReactiveHandler, req: &InjectionRequest) -> bool {
        super::jekt_hold::hold_wan_for_unavailable(mstore, handler, req)
    }

    fn hold_relayed<'a>(&'a self, req: &'a InjectionRequest) -> BoxFuture<'a, bool> {
        Box::pin(super::jekt_cold::hold_relayed(req))
    }

    fn settle_files(&self, req: &InjectionRequest, delivered: bool) {
        super::jekt_files::settle(req, delivered);
    }

    fn resolve_transcript_request_tier_fields(&self, mstore: &Arc<Store>, req: &mut InjectionRequest) {
        super::reactive::resolve_transcript_request_tier_fields(mstore, req);
    }

    fn fetch_files<'a>(
        &'a self,
        base: &'a str,
        http: &'a reqwest::Client,
        token: &'a str,
        agent_id: &'a str,
        req: &'a mut InjectionRequest,
        files: Vec<CloudFile>,
        wan_msg_id: Option<&'a str>,
    ) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(super::jekt_cloud::fetch(base, http, token, agent_id, req, files, wan_msg_id))
    }

    fn prepare_files<'a>(&'a self, req: &'a mut InjectionRequest) -> BoxFuture<'a, ()> {
        Box::pin(super::jekt_files::prepare(req))
    }
}
