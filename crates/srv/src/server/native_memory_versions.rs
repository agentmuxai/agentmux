// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Personal Memory versions as the history RPCs and MCP tools serve them,
//! each with its size and estimated tokens (SPEC_MEMORY_PANE_TOKENS_2026_10_15.md §5.5).

use crate::backend::rpc_types::NativeMemoryVersionMeta;
use crate::backend::storage::{NativeMemoryVersion, NativeMemoryVersionSummary};

pub(crate) fn version_summary_to_meta(v: NativeMemoryVersionSummary) -> NativeMemoryVersionMeta {
    NativeMemoryVersionMeta {
        id: v.id,
        content_hash: v.content_hash,
        parent_version_id: v.parent_version_id,
        source: v.source,
        source_detail: v.source_detail,
        session_id: v.session_id,
        created_at: v.created_at,
        size_bytes: v.size_bytes,
        tokens_est: v.tokens_est,
    }
}

pub(crate) fn version_to_meta(v: &NativeMemoryVersion) -> NativeMemoryVersionMeta {
    NativeMemoryVersionMeta {
        id: v.id.clone(),
        content_hash: v.content_hash.clone(),
        parent_version_id: v.parent_version_id.clone(),
        source: v.source.clone(),
        source_detail: v.source_detail.clone(),
        session_id: v.session_id.clone(),
        created_at: v.created_at,
        size_bytes: v.content.len() as u64,
        tokens_est: agentmux_common::tokens::estimate_tokens(&v.content),
    }
}
