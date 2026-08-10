// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Agent-anchored session zones: one zone per agent definition, keyed by
//! `definition_id` (not identity bundle or block).
//!
//! Zone names: active = `agent:<defId>:current`,
//! archived = `agent:<defId>:archive:<unix_ms>`. Each zone holds
//! `output.state.json` (full UI snapshot) and `output` (raw NDJSON stream).
//! See `docs/specs/SPEC_CONTINUATION_SESSION_PERSISTENCE_2026_05_23.md`.
//!
//! ## Public API surface
//!
//! Every public item is reachable as `agent_session::<submod>::<item>` (the
//! submodules are `pub`). The `pub use` re-exports below additionally preserve
//! the pre-split flat paths (`agent_session::<item>`) that callers across the
//! crate already import, so this reorganization changed no call site. The flat
//! re-export set is exactly the names other modules import; the remaining public
//! items (zone-name builders, archive internals, migration stat/marker types)
//! stay reachable through their submodule path — and the test module reaches
//! them there.

pub mod archive;
pub mod global_store;
pub(crate) mod helpers;
pub mod migrations;
pub mod session_io;
pub mod zone_naming;

// Flat re-exports preserving the pre-split `agent_session::<item>` call sites
// used elsewhere in the crate (shell.rs, persistent.rs, transcript_backfill.rs,
// server RPC session.rs, instance.rs, app_api, main.rs, identity migrations).
pub use archive::{archive_session, list_archives};
pub use global_store::{
    agent_zone_for_block_meta, global_transcript_store, set_global_transcript_store,
};
pub use migrations::{migrate_block_zones_v1, migrate_promote_template_sessions_v1};
pub use session_io::{
    append_session_output, heal_global_snapshot_source_block_ids, read_session_state,
    write_session_state, OUTPUT_FILE, SNAPSHOT_FILE, TSIDX_FILE,
};
pub use zone_naming::{agent_current_zone, is_valid_definition_id};

#[cfg(test)]
mod tests;
