// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Model routes and the gateway (`docs/specs/SPEC_AGENT_MODEL_ROUTES_ANY_MODEL_SPAWN_AND_SWITCH_2026_11_06.md`).
//!
//! An agent runs a harness on a model route with a key. A route is an entry in [`catalog`]; its key is
//! a Vault entry that never leaves srv. The harness is pointed at `/api/v1/model/<route>/...` with a
//! per-agent token ([`token`]), and the handler (`server::model_gateway_handlers`) adds the real key
//! upstream, streams the answer back, and counts what each agent used ([`spend`], [`usage`]).

pub mod ask;
pub mod catalog;
pub mod launch;
pub mod request;
pub mod spend;
pub mod switch;
pub mod token;
pub mod usage;

/// The path prefix the gateway is served under; the route id and the harness's own path follow.
pub const PATH_PREFIX: &str = "/api/v1/model/";
