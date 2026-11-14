// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What the relay's delivery path asks of the server layer.
//!
//! A message pulled from the relay is checked, its files fetched and the message held or injected,
//! and a few of those steps (the holds, the files, the transcript-request tier rules) live in
//! `server/` because they read the server's state and handlers. This module is the seam: muxbus
//! names the [`ServerHooks`] it needs, and the server registers them once at startup
//! ([`install`]), the way `server::jekt_cold::install` hands the cold hold its state. It is also
//! the one place `muxbus` stops naming `server`, which the srv core split needs (Area54 #1350).

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};

use agentmux_common::jekt_files::CloudFile;

use crate::backend::reactive::handler::ReactiveHandler;
use crate::backend::reactive::types::InjectionRequest;
use crate::backend::storage::store::Store;

pub(crate) type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The server-side steps of delivering a relayed message.
pub(crate) trait ServerHooks: Send + Sync {
    /// Verified and its agent can't work now: keep it here until it can. `true` when it is kept.
    fn hold_wan_for_unavailable(&self, mstore: &Store, handler: &ReactiveHandler, req: &InjectionRequest) -> bool;

    /// A routine message to a session gone cold waits for its next activity. `true` when it is kept.
    fn hold_relayed<'a>(&'a self, req: &'a InjectionRequest) -> BoxFuture<'a, bool>;

    /// A message's inline files are charged until it is delivered; give the charge back when it was not.
    fn settle_files(&self, req: &InjectionRequest, delivered: bool);

    /// The two tier fields a transcript request carries, from the responding agent's own definition.
    fn resolve_transcript_request_tier_fields(&self, mstore: &Arc<Store>, req: &mut InjectionRequest);

    /// Check a cloud manifest, then fetch, decrypt and store each file. `Err` when the relay could
    /// not be reached, so the caller releases the claim.
    #[allow(clippy::too_many_arguments)]
    fn fetch_files<'a>(
        &'a self,
        base: &'a str,
        http: &'a reqwest::Client,
        token: &'a str,
        agent_id: &'a str,
        req: &'a mut InjectionRequest,
        files: Vec<CloudFile>,
        wan_msg_id: Option<&'a str>,
    ) -> BoxFuture<'a, Result<(), String>>;

    /// List the fetched files in the message the agent reads.
    fn prepare_files<'a>(&'a self, req: &'a mut InjectionRequest) -> BoxFuture<'a, ()>;
}

static HOOKS: OnceLock<&'static dyn ServerHooks> = OnceLock::new();

/// Register the server's steps. Later calls change nothing.
pub(crate) fn install(hooks: &'static dyn ServerHooks) {
    let _ = HOOKS.set(hooks);
}

/// The registered steps, or `None` before the server has registered them. A delivery round that finds `None`
/// claims nothing, so the relay offers the same messages again on the next pull. Under test, the server's own.
pub(crate) fn installed() -> Option<&'static dyn ServerHooks> {
    #[cfg(test)]
    {
        Some(HOOKS.get().copied().unwrap_or(crate::server::muxbus_hooks::hooks()))
    }
    #[cfg(not(test))]
    {
        HOOKS.get().copied()
    }
}

/// The registered steps, or ones that do nothing and report the relay as unreachable. A delivery
/// round checks [`installed`] before it claims anything, so this is only what a step reads if the
/// server's registration is somehow lost mid-round.
pub(crate) fn hooks() -> &'static dyn ServerHooks {
    installed().unwrap_or(&NotReady)
}

struct NotReady;

impl ServerHooks for NotReady {
    fn hold_wan_for_unavailable(&self, _: &Store, _: &ReactiveHandler, _: &InjectionRequest) -> bool {
        false
    }
    fn hold_relayed<'a>(&'a self, _: &'a InjectionRequest) -> BoxFuture<'a, bool> {
        Box::pin(async { false })
    }
    fn settle_files(&self, _: &InjectionRequest, _: bool) {}
    fn resolve_transcript_request_tier_fields(&self, _: &Arc<Store>, _: &mut InjectionRequest) {}
    fn fetch_files<'a>(
        &'a self,
        _: &'a str,
        _: &'a reqwest::Client,
        _: &'a str,
        _: &'a str,
        _: &'a mut InjectionRequest,
        _: Vec<CloudFile>,
        _: Option<&'a str>,
    ) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async { Err("AgentMux isn't ready to fetch files yet".to_string()) })
    }
    fn prepare_files<'a>(&'a self, _: &'a mut InjectionRequest) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req() -> InjectionRequest {
        InjectionRequest { target_agent: "hook-target".into(), request_id: Some("hook-msg".into()), ..Default::default() }
    }

    #[tokio::test]
    async fn the_steps_a_lost_registration_falls_back_to_hold_nothing_and_fetch_nothing() {
        let not_ready: &dyn ServerHooks = &NotReady;
        let store = Arc::new(Store::open_in_memory().unwrap());
        let mut r = req();
        assert!(!not_ready.hold_relayed(&r).await);
        not_ready.settle_files(&r, false);
        not_ready.resolve_transcript_request_tier_fields(&store, &mut r);
        not_ready.prepare_files(&mut r).await;
        assert!(!r.is_transcript_request && r.files.is_none() && r.files_delivery.is_none());
        let http = reqwest::Client::new();
        let err = not_ready.fetch_files("http://127.0.0.1:1", &http, "t", "a", &mut r, Vec::new(), None).await;
        assert!(err.is_err(), "a fetch with no registered steps gives the claim back");
    }

    #[test]
    fn under_test_the_servers_own_steps_stand_in_until_something_registers() {
        assert!(installed().is_some());
    }

    #[tokio::test]
    async fn the_servers_steps_leave_a_message_with_no_files_and_no_transcript_request_alone() {
        let steps = installed().unwrap();
        let store = Arc::new(Store::open_in_memory().unwrap());
        let mut r = req();
        r.message = "plain words".into();
        steps.resolve_transcript_request_tier_fields(&store, &mut r);
        steps.settle_files(&r, false);
        steps.prepare_files(&mut r).await;
        assert!(!r.is_transcript_request);
        assert!(r.files.is_none() && r.files_delivery.is_none());
    }
}
