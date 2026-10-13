// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Which request and response type each RPC command carries.
//!
//! # Why this exists
//!
//! The frontend talks to srv through 284 hand-written command stubs
//! (`frontend/app/store/rpc-api/*.ts`) and a 2,819-line hand-maintained
//! `frontend/types/srv-types.d.ts`, every one of them headed "keep in sync
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
    /// The request as TypeScript, from ts-rs. `None` only for bindings
    /// recorded by [`RpcSchema::record`] without one (the unit tests here).
    pub request_ts: Option<TsType>,
    /// The response as TypeScript.
    pub response_ts: Option<TsType>,
}

/// How a Rust type reads in TypeScript, as ts-rs itself renders it.
///
/// Taken from ts-rs rather than derived from the Rust path, because the two
/// differ: `#[ts(rename = "DeleteDroneResp")] struct DeleteResp` exports as
/// `DeleteDroneResp`, so matching on the Rust name would bind a stub to a type
/// that does not exist (or, worse, to an unrelated one of that name).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TsType {
    /// The type expression, e.g. `AgentDefinition`, `Array<Skill>`,
    /// `CommandFooData | null`, `null` (for `()`).
    pub name: String,
    /// Every exported type the expression names, which a generated stub
    /// imports from `frontend/types/rpc/`. Sorted and de-duplicated.
    pub imports: Vec<String>,
}

/// `T` as TypeScript: its ts-rs name, and the exported types that name uses.
///
/// A type with its own exported file (`#[ts(export)]`) is imported by name;
/// its fields are that file's business. A wrapper without one (`Option<X>`,
/// `Vec<X>`, `HashMap<String, X>`) is looked through, via its generic
/// parameters: ts-rs reports `X` as a generic of `Option<X>`, not as a
/// dependency, so `TS::dependencies()` alone would miss it.
pub fn ts_type<T: ts_rs::TS + 'static + ?Sized>() -> TsType {
    struct Collect(std::collections::BTreeSet<String>);
    impl ts_rs::TypeVisitor for Collect {
        fn visit<U: ts_rs::TS + 'static + ?Sized>(&mut self) {
            if U::output_path().is_some() {
                self.0.insert(U::ident());
            } else {
                U::visit_generics(self);
            }
        }
    }
    let mut c = Collect(Default::default());
    ts_rs::TypeVisitor::visit::<T>(&mut c);
    TsType { name: T::name(), imports: c.0.into_iter().collect() }
}

impl RpcBinding {
    /// Last path segment of `request` — the name a generator would emit.
    ///
    /// Kept alongside the full path rather than replacing it: the short name
    /// is what a binding is called in TypeScript, but two modules can hold
    /// same-named types, so the full path stays authoritative.
    #[cfg(test)]
    pub fn request_name(&self) -> &str {
        short_name(self.request)
    }

    /// Last path segment of `response`.
    #[cfg(test)]
    pub fn response_name(&self) -> &str {
        short_name(self.response)
    }
}

/// `a::b::Foo` -> `Foo`; also strips a generic tail so `Vec<a::B>` reads as
/// `Vec<a::B>` rather than being truncated at a `::` inside the parameter.
#[cfg(test)]
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

    /// Record (or replace) the binding for `command`, without TypeScript.
    pub fn record(&mut self, command: &str, request: &'static str, response: &'static str) {
        self.bindings.insert(
            command.to_string(),
            RpcBinding {
                command: command.to_string(),
                request,
                response,
                request_ts: None,
                response_ts: None,
            },
        );
    }

    /// Record (or replace) `command`'s binding, with its request and response
    /// as TypeScript. This is what `register_typed` records.
    pub fn record_typed<Req, Resp>(&mut self, command: &str)
    where
        Req: ts_rs::TS + 'static,
        Resp: ts_rs::TS + 'static,
    {
        self.bindings.insert(
            command.to_string(),
            RpcBinding {
                command: command.to_string(),
                request: std::any::type_name::<Req>(),
                response: std::any::type_name::<Resp>(),
                request_ts: Some(ts_type::<Req>()),
                response_ts: Some(ts_type::<Resp>()),
            },
        );
    }

    /// Bindings in command order. `BTreeMap` makes this stable across runs,
    /// which is what lets a generated artifact be diffed in CI.
    #[cfg(test)]
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

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.bindings.len()
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }

    #[cfg(test)]
    pub fn get(&self, command: &str) -> Option<&RpcBinding> {
        self.bindings.get(command)
    }

    /// Stable JSON, sorted by command, for the dump + CI gate.
    #[cfg(test)]
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::Value::Array(
            self.bindings
                .values()
                .map(|b| {
                    let ts = |t: &Option<TsType>| match t {
                        Some(t) => serde_json::json!({ "name": t.name, "imports": t.imports }),
                        None => serde_json::Value::Null,
                    };
                    serde_json::json!({
                        "command": b.command,
                        "request": b.request,
                        "requestName": b.request_name(),
                        "requestTs": ts(&b.request_ts),
                        "response": b.response,
                        "responseName": b.response_name(),
                        "responseTs": ts(&b.response_ts),
                    })
                })
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(ts_rs::TS)]
    #[ts(export_to = "../../../frontend/types/rpc/")]
    #[ts(rename = "Renamed")]
    #[allow(dead_code)]
    struct RustName {
        v: bool,
    }

    #[test]
    fn ts_type_uses_the_exported_name_not_the_rust_one() {
        // The reason the schema asks ts-rs instead of trimming the Rust path:
        // `DeleteResp` exports as `DeleteDroneResp`, and a stub must name that.
        let t = ts_type::<RustName>();
        assert_eq!(t.name, "Renamed");
        assert_eq!(t.imports, vec!["Renamed".to_string()]);
    }

    #[test]
    fn ts_type_looks_through_option_and_vec_to_the_type_to_import() {
        // ts-rs reports `X` as a generic parameter of `Option<X>`, not as a
        // dependency, so collecting `dependencies()` alone would import nothing.
        let opt = ts_type::<Option<RustName>>();
        assert_eq!(opt.name, "Renamed | null");
        assert_eq!(opt.imports, vec!["Renamed".to_string()]);
        let vec = ts_type::<Vec<RustName>>();
        assert_eq!(vec.name, "Array<Renamed>");
        assert_eq!(vec.imports, vec!["Renamed".to_string()]);
    }

    #[test]
    fn ts_type_of_unit_and_primitives_imports_nothing() {
        assert_eq!(ts_type::<()>(), TsType { name: "null".into(), imports: vec![] });
        assert_eq!(ts_type::<String>().imports, Vec::<String>::new());
    }

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
