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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandNativeMemoryWriteFileData {
    pub agent_id: String,
    pub filename: String,
    pub content: String,
}
