// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0


//! Native memory RPC payloads (agent:memory:list / read / write) and
//! version history (agent:memory:history / diff / revert). Native memory is
//! what the agent writes about itself; it is distinct from a Bundle.



use serde::{Deserialize, Serialize};

// ---- Native memory RPCs — agent:memory:list / read / write ----

/// Metadata for one `*.md` file in the agent's native memory folder.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct NativeMemoryFileMeta {
    pub filename: String,
    /// True only for `MEMORY.md` (the Claude Code index file).
    pub is_index: bool,
    /// Parsed from YAML frontmatter `type:` field. Null when absent.
    pub metadata_type: Option<String>,
    // ts-rs maps 64-bit integers to `bigint`; every consumer treats this as a
    // plain JS number. See BrowserBookmark::created_at (PR #3293).
    #[ts(type = "number")]
    pub size_bytes: u64,
    // ts-rs maps 64-bit integers to `bigint`; every consumer treats this as a
    // plain JS number. See BrowserBookmark::created_at (PR #3293).
    #[ts(type = "number")]
    /// Unix timestamp in milliseconds.
    pub modified_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandNativeMemoryListData {
    pub agent_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct NativeMemoryListResult {
    pub files: Vec<NativeMemoryFileMeta>,
    /// The folder was found by a guess (a blank working directory and no
    /// spawn on record), not from the agent's own launch: shown read-only
    /// until the agent launches (SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md
    /// §2.1.2).
    #[serde(default)]
    pub unverified: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandNativeMemoryReadFileData {
    pub agent_id: String,
    pub filename: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct NativeMemoryReadFileResult {
    pub content: String,
}

/// Optional caller-supplied context for why a write happened — advisory,
/// not enforced server-side (a careless/compromised caller could mis-tag
/// it). Omitting this entirely defaults to `source: "agent_inferred"` at
/// the handler layer — fully backward compatible with callers that don't
/// know about this field yet.
/// See docs/specs/SPEC_MEMORY_VERSION_CONTROL_AND_ARMORY_AUDIT_2026_08_19.md §4.1.
// DELIBERATELY NOT ts-rs-generated, unlike every other type in this file.
// `detail` is a `serde_json::Value` that the frontend has always treated as an
// OPTIONAL property (`detail?: unknown`) — every caller constructs
// `{ source: "human" }` with no detail. ts-rs rejects `#[ts(optional)]` on
// anything that is not `Option<T>` ("optional can only be used on an Option<T>
// type"), and it has no other way to emit an optional property, so this shape
// is not expressible by the generator today.
//
// The alternative — changing the field to `Option<serde_json::Value>` — would
// be letting the codegen dictate wire behaviour: `default_detail()` exists
// precisely so an omitted detail becomes `{}` rather than `null`, which the
// comment on that function documents as a real bug it fixed. So this one type
// stays hand-written in srv-types.d.ts and is referenced by name below.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeMemoryWriteProvenance {
    /// `"human"` | `"agent_inferred"` | `"jekt"` — not validated against an
    /// enum at this layer; an unrecognized value is stored as-is (the UI
    /// simply won't have a special tag for it, matching the version
    /// table's own forward-compatible `source` column).
    pub source: String,
    /// Arbitrary JSON — the jekt marker fields when `source == "jekt"`.
    /// Stored verbatim as the version row's `source_detail`.
    ///
    /// reagent P2: `#[serde(default)]` on a bare `serde_json::Value` yields
    /// `Value::Null` when the caller supplies `source` but omits `detail`
    /// — `.to_string()` on that is the literal string `"null"`, not the
    /// `"{}"` every no-provenance write elsewhere in this module already
    /// uses as its default `source_detail`. `default_detail()` below keeps
    /// the two cases consistent.
    // ts-rs cannot derive TS for `serde_json::Value` (no impl), and the
    // frontend has always typed this `detail?: unknown`. `optional` is
    // load-bearing, not cosmetic: every caller constructs provenance as
    // `{ source: "human" }` with no detail (agent-native-memory-model.ts),
    // so a required field would fail to compile on the frontend.
    #[serde(default = "default_detail")]
    pub detail: serde_json::Value,
}

fn default_detail() -> serde_json::Value {
    serde_json::json!({})
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandNativeMemoryWriteFileData {
    pub agent_id: String,
    pub filename: String,
    pub content: String,
    // Points at the hand-written `NativeMemoryWriteProvenance` in
    // srv-types.d.ts (see the note on that struct above for why it is not
    // generated). `optional` is valid here because THIS field really is
    // `Option<T>`.
    #[serde(default)]
    #[ts(optional, type = "NativeMemoryWriteProvenance")]
    pub provenance: Option<NativeMemoryWriteProvenance>,
    /// SHA-256 (lowercase hex) of the content the caller's draft was based
    /// on — the UTF-8 bytes of exactly what `agent:memory:read_file`
    /// returned. When present and the file's current content hashes to
    /// anything else (or the file no longer exists), the write is REFUSED
    /// with a `conflict:` error and nothing is written. When absent the
    /// write behaves exactly as it always has (last writer wins). See
    /// docs/specs/SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.4.
    #[serde(default)]
    #[ts(optional)]
    pub base_sha256: Option<String>,
}

// ---- Native memory version history — agent:memory:history / diff / revert ----
// See docs/specs/SPEC_MEMORY_VERSION_CONTROL_AND_ARMORY_AUDIT_2026_08_19.md §4.3.

/// Wire-level (serializable) shape of a version's metadata, without its
/// full content — mirrors `NativeMemoryVersionSummary` in
/// `backend::storage::agent_native_memory_versions` exactly. Kept as a
/// separate type (rather than deriving Serialize on the storage struct
/// directly) so the storage layer never needs a serde dependency just to
/// satisfy an RPC wire shape.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct NativeMemoryVersionMeta {
    pub id: String,
    pub content_hash: String,
    pub parent_version_id: Option<String>,
    pub source: String,
    pub source_detail: String,
    pub session_id: String,
    // ts-rs maps 64-bit integers to `bigint`; every consumer treats this as a
    // plain JS number. See BrowserBookmark::created_at (PR #3293).
    #[ts(type = "number")]
    pub created_at: i64,
}

/// `agent:memory:claims` — the memory folders an agent has claimed, for
/// the human "release this folder" action. Releasing goes through the
/// host's confirmation window, not an RPC.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandNativeMemoryClaimsData {
    pub agent_id: String,
}

/// `agent:memory:adoption_list` — the agent's memory folders under its
/// earlier accounts, offered for adoption
/// (SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.1.4). Adopting one goes
/// through the host's confirmation window, not an RPC.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandNativeMemoryAdoptionListData {
    pub agent_id: String,
}

#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct NativeMemoryAdoptionListResult {
    /// `None` until the agent has a verified memory folder (its first spawn,
    /// or a working directory).
    pub list: Option<crate::backend::memory_adopt::AdoptionList>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandNativeMemoryHistoryData {
    pub agent_id: String,
    pub filename: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct NativeMemoryHistoryResult {
    /// Newest first — matches `agent_native_memory_version_list`'s own
    /// ordering.
    pub versions: Vec<NativeMemoryVersionMeta>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandNativeMemoryDiffData {
    /// reagent P1: required so the handler can verify BOTH versions belong
    /// to this agent before returning their content — unlike list/read/
    /// write/history/revert (all of which take an agent_id and scope to
    /// it), diff originally took bare version ids with no ownership check
    /// at all. Since every caller shares one instance-wide X-AuthKey, that
    /// let any caller read any other agent's memory content by version id.
    pub agent_id: String,
    pub from_version_id: String,
    pub to_version_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct NativeMemoryDiffResult {
    /// A minimal line-based diff: one line per input line, prefixed `"  "`
    /// (context), `"- "` (removed, present in `from` only), or `"+ "`
    /// (added, present in `to` only). No `@@` hunk headers in v1.
    pub diff: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandNativeMemoryRevertData {
    pub agent_id: String,
    pub filename: String,
    pub target_version_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct NativeMemoryRevertResult {
    /// The newly created version (source `"revert"`) whose content now
    /// matches `target_version_id` — the prior latest version is left
    /// untouched, per the version chain's append-only guarantee.
    pub version: NativeMemoryVersionMeta,
}
