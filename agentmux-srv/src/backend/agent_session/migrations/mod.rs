// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! One-time / self-idempotent migrations for agent session zones.
//!
//! Submodules are `pub` so every public item (stat structs, marker consts) is
//! reachable as `agent_session::migrations::<submod>::<item>`. The two migration
//! entry points are additionally re-exported here (and again from the parent)
//! to preserve the flat `agent_session::migrate_*` call sites.

pub mod v1_blocks;
pub mod v1_templates;

pub use v1_blocks::migrate_block_zones_v1;
pub use v1_templates::migrate_promote_template_sessions_v1;
