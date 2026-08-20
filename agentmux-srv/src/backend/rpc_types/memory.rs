// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Memory bundle command shapes (v7) + native memory RPC payloads
//! (agent:memory:list / read / write).

use serde::{Deserialize, Serialize};

// ---- v7 Memory bundle command shapes ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandGetMemoryData {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandDeleteMemoryData {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandReorderGlobalBrainData {
    /// Full ordered list of global bundle ids. Each id's `sort_order`
    /// becomes its position in this list.
    pub ids: Vec<String>,
}

// ---- Native memory RPCs — agent:memory:list / read / write ----

/// Metadata for one `*.md` file in the agent's native memory folder.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeMemoryFileMeta {
    pub filename: String,
    /// True only for `MEMORY.md` (the Claude Code index file).
    pub is_index: bool,
    /// Parsed from YAML frontmatter `type:` field. Null when absent.
    pub metadata_type: Option<String>,
    pub size_bytes: u64,
    /// Unix timestamp in milliseconds.
    pub modified_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandNativeMemoryListData {
    pub agent_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeMemoryListResult {
    pub files: Vec<NativeMemoryFileMeta>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandNativeMemoryReadFileData {
    pub agent_id: String,
    pub filename: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeMemoryReadFileResult {
    pub content: String,
}

/// Optional caller-supplied context for why a write happened — advisory,
/// not enforced server-side (a careless/compromised caller could mis-tag
/// it). Omitting this entirely defaults to `source: "agent_inferred"` at
/// the handler layer — fully backward compatible with callers that don't
/// know about this field yet.
/// See docs/specs/SPEC_MEMORY_VERSION_CONTROL_AND_ARMORY_AUDIT_2026_08_19.md §4.1.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeMemoryWriteProvenance {
    /// `"human"` | `"agent_inferred"` | `"jekt"` — not validated against an
    /// enum at this layer; an unrecognized value is stored as-is (the UI
    /// simply won't have a special tag for it, matching the version
    /// table's own forward-compatible `source` column).
    pub source: String,
    /// Arbitrary JSON — the jekt marker fields when `source == "jekt"`.
    /// Stored verbatim as the version row's `source_detail`.
    #[serde(default)]
    pub detail: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandNativeMemoryWriteFileData {
    pub agent_id: String,
    pub filename: String,
    pub content: String,
    #[serde(default)]
    pub provenance: Option<NativeMemoryWriteProvenance>,
}

// ---- Native memory version history — agent:memory:history / diff / revert ----
// See docs/specs/SPEC_MEMORY_VERSION_CONTROL_AND_ARMORY_AUDIT_2026_08_19.md §4.3.

/// Wire-level (serializable) shape of a version's metadata, without its
/// full content — mirrors `NativeMemoryVersionSummary` in
/// `backend::storage::agent_native_memory_versions` exactly. Kept as a
/// separate type (rather than deriving Serialize on the storage struct
/// directly) so the storage layer never needs a serde dependency just to
/// satisfy an RPC wire shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeMemoryVersionMeta {
    pub id: String,
    pub content_hash: String,
    pub parent_version_id: Option<String>,
    pub source: String,
    pub source_detail: String,
    pub session_id: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandNativeMemoryHistoryData {
    pub agent_id: String,
    pub filename: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeMemoryHistoryResult {
    /// Newest first — matches `agent_native_memory_version_list`'s own
    /// ordering.
    pub versions: Vec<NativeMemoryVersionMeta>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandNativeMemoryDiffData {
    pub from_version_id: String,
    pub to_version_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeMemoryDiffResult {
    /// A minimal line-based diff: one line per input line, prefixed `"  "`
    /// (context), `"- "` (removed, present in `from` only), or `"+ "`
    /// (added, present in `to` only). No `@@` hunk headers in v1.
    pub diff: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandNativeMemoryRevertData {
    pub agent_id: String,
    pub filename: String,
    pub target_version_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeMemoryRevertResult {
    /// The newly created version (source `"revert"`) whose content now
    /// matches `target_version_id` — the prior latest version is left
    /// untouched, per the version chain's append-only guarantee.
    pub version: NativeMemoryVersionMeta,
}
