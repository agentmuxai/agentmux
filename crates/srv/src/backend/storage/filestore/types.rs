// Copyright 2025, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Wire types matching Go's wshrpc types for file storage.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// File options. Matches Go's `FileOpts`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FileOpts {
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub maxsize: i64,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub circular: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ijson: bool,
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub ijsonbudget: i32,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncate: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub append: bool,
}

use crate::backend::rpc_types::is_zero_i64;

fn is_zero_i32(v: &i32) -> bool {
    *v == 0
}

/// File metadata. Matches Go's `FileMeta = map[string]any`.
pub type FileMeta = HashMap<String, serde_json::Value>;

/// A wave file record. Matches Go's `MuxFile`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MuxFile {
    pub zoneid: String,
    pub name: String,
    pub size: i64,
    pub createdts: i64,
    pub modts: i64,
    pub opts: FileOpts,
    pub meta: FileMeta,
}

/// The meta key holding the first offset still stored once rows below it were
/// deleted (`trim.rs`): it holds when the window is later widened or lifted.
pub(super) const RETAINED_FROM_KEY: &str = "retainedfrom";

impl MuxFile {
    /// Effective data length, accounting for circular files.
    pub fn data_length(&self) -> i64 {
        self.size - self.data_start_idx()
    }

    /// Start index of data: for circular files the window's start, and never
    /// before the part rows a trim left (the window may have been widened since).
    pub fn data_start_idx(&self) -> i64 {
        let window = if self.opts.circular && self.opts.maxsize > 0 && self.size > self.opts.maxsize {
            self.size - self.opts.maxsize
        } else {
            0
        };
        let retained = self.meta.get(RETAINED_FROM_KEY).and_then(serde_json::Value::as_i64).unwrap_or(0);
        window.max(retained.min(self.size))
    }
}
