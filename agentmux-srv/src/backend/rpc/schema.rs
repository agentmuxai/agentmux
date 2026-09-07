// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Which request and response type each RPC command carries.
//!
//! # Why this exists
//!
//! The frontend talks to srv through 284 hand-written command stubs
//! (`frontend/app/store/rpc-api/*.ts`) and a 2,819-line hand-maintained
//! `frontend/types/gotypes.d.ts`, every one of them headed "keep in sync
//! with the agentmux-srv RPC". `docs/reports/REPORT_DRY_AND_MODULARITY_AUDIT_2026_09_06.md`
//! §2.1 counted 334 such comments and one live drift found by hand.
//!
//! The obvious fix — derive the TypeScript types from the Rust ones — does
//! not work on its own, and `docs/specs/SPEC_RPC_BINDINGS_CODEGEN_2026_09_07.md`
//! §2 explains why: **which command takes which request and returns which
//! response is not data anywhere in srv.** Each handler picks its request
//! type inline (`serde_json::from_value::<CommandXData>(data)`) and builds
//! its response ad hoc (`serde_json::to_value(&x)`, `json!({"ok": true})`).
//! A type generator can emit every struct and still not know how to pair
//! them into a callable stub.
//!
//! This module is that missing mapping, recorded as a side effect of
//! registration itself (`RpcEngine::register_typed`) rather than written
//! down a second time — so it cannot drift from the handlers the way a
//! hand-maintained list would. It is deliberately just the mapping: no
//! TypeScript is emitted here, and adding a code generator on top is a
//! separate change against an already-proven registry (spec §3.4 step 1).
//!
//! # What a binding is
//!
//! One row per command registered through the typed path, holding the fully
//! qualified Rust type names of its request and response. Handlers still on
//! the untyped `register_handler` contribute nothing — they are simply
//! absent, which is what makes an incremental migration possible.

use std::collections::BTreeMap;

/// The request/response pair for one command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RpcBinding {
    /// Wire command name, e.g. `agentdefhide`.
    pub command: String,
    /// Fully qualified request type, from [`std::any::type_name`].
    pub request: &'static str,
    /// Fully qualified response type.
    pub response: &'static str,
}

impl RpcBinding {
    /// Last path segment of `request` — the name a generator would emit.
    ///
    /// Kept alongside the full path rather than replacing it: the short name
    /// is what a binding is called in TypeScript, but two modules can hold
    /// same-named types, so the full path stays authoritative.
    pub fn request_name(&self) -> &str {
        short_name(self.request)
    }

    /// Last path segment of `response`.
    pub fn response_name(&self) -> &str {
        short_name(self.response)
    }
}

/// `a::b::Foo` -> `Foo`; also strips a generic tail so `Vec<a::B>` reads as
/// `Vec<a::B>` rather than being truncated at a `::` inside the parameter.
fn short_name(path: &str) -> &str {
    match path.find('<') {
        // Generic: leave it whole. Truncating `Vec<crate::x::Y>` at the last
        // `::` would produce `Y>`, which names nothing.
        Some(_) => path,
        None => path.rsplit("::").next().unwrap_or(path),
    }
}

/// Every typed command registered on one engine.
///
/// Keyed by command so a duplicate registration overwrites rather than
/// silently recording the command twice — that mirrors the handler map in
/// `engine.rs`, where the second `register_*` for a command wins.
#[derive(Debug, Default)]
pub struct RpcSchema {
    bindings: BTreeMap<String, RpcBinding>,
}

impl RpcSchema {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record (or replace) the binding for `command`.
    pub fn record(&mut self, command: &str, request: &'static str, response: &'static str) {
        self.bindings.insert(
            command.to_string(),
            RpcBinding {
                command: command.to_string(),
                request,
                response,
            },
        );
    }

    /// Bindings in command order. `BTreeMap` makes this stable across runs,
    /// which is what lets a generated artifact be diffed in CI.
    pub fn bindings(&self) -> Vec<&RpcBinding> {
        self.bindings.values().collect()
    }

    /// Forget `command`'s binding, if it had one.
    ///
    /// Called when a command is (re-)registered through an untyped path, so
    /// the schema always describes the handler that is actually installed. A
    /// stale typed pair would be worse than no pair at all: it would generate
    /// a client stub with types the live handler does not accept.
    pub fn remove(&mut self, command: &str) {
        self.bindings.remove(command);
    }

    pub fn len(&self) -> usize {
        self.bindings.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }

    pub fn get(&self, command: &str) -> Option<&RpcBinding> {
        self.bindings.get(command)
    }

    /// Stable JSON, sorted by command, for the dump + CI gate.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::Value::Array(
            self.bindings
                .values()
                .map(|b| {
                    serde_json::json!({
                        "command": b.command,
                        "request": b.request,
                        "requestName": b.request_name(),
                        "response": b.response,
                        "responseName": b.response_name(),
                    })
                })
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_name_takes_the_last_segment() {
        assert_eq!(short_name("agentmux_srv::rpc_types::CommandFooData"), "CommandFooData");
        assert_eq!(short_name("Foo"), "Foo");
    }

    #[test]
    fn short_name_leaves_generics_whole_rather_than_truncating_mid_parameter() {
        // `rsplit("::")` on this would yield `AgentDefinition>` — a name that
        // does not exist. Better to hand back something a generator can
        // reject loudly than something that looks valid and is not.
        let generic = "alloc::vec::Vec<agentmux_srv::storage::AgentDefinition>";
        assert_eq!(short_name(generic), generic);
    }

    #[test]
    fn bindings_come_back_in_command_order_regardless_of_insertion_order() {
        let mut s = RpcSchema::new();
        s.record("zebra", "a::Z", "a::ZR");
        s.record("alpha", "a::A", "a::AR");
        let got: Vec<&str> = s.bindings().iter().map(|b| b.command.as_str()).collect();
        assert_eq!(got, vec!["alpha", "zebra"], "order must not depend on registration order");
    }

    #[test]
    fn re_registering_a_command_replaces_it_the_way_the_handler_map_does() {
        let mut s = RpcSchema::new();
        s.record("dup", "a::First", "a::FirstR");
        s.record("dup", "a::Second", "a::SecondR");
        assert_eq!(s.len(), 1);
        assert_eq!(s.get("dup").unwrap().request, "a::Second");
    }

    #[test]
    fn remove_forgets_a_binding_so_a_replaced_command_stops_advertising_stale_types() {
        let mut s = RpcSchema::new();
        s.record("foo", "a::Req", "a::Resp");
        s.remove("foo");
        assert!(s.get("foo").is_none());
        assert!(s.is_empty());
        // Removing something that was never recorded is a no-op, not a panic:
        // every untyped registration calls this, and most commands were never
        // typed in the first place.
        s.remove("never-registered");
    }

    #[test]
    fn json_is_stable_and_carries_both_the_full_path_and_the_short_name() {
        let mut s = RpcSchema::new();
        s.record("agentdefhide", "crate::rpc_types::CommandAgentDefHideData", "crate::rpc_types::AgentDefHideResult");
        let v = s.to_json();
        assert_eq!(v[0]["command"], "agentdefhide");
        assert_eq!(v[0]["request"], "crate::rpc_types::CommandAgentDefHideData");
        assert_eq!(v[0]["requestName"], "CommandAgentDefHideData");
        assert_eq!(v[0]["responseName"], "AgentDefHideResult");
        assert_eq!(s.to_json(), v, "same input must serialize identically every time");
    }
}
