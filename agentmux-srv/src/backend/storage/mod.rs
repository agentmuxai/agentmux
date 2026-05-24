// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Storage layer: SQLite-backed object store and file store.
//! Port of Go's pkg/wstore and pkg/filestore.

pub mod agents_consolidate;
pub mod error;
pub mod filestore;
pub mod migrations;
pub mod wstore;

pub use error::StoreError;
pub use wstore::AgentDefinition;
pub use wstore::AgentContent;
#[allow(unused_imports)]
pub use wstore::AgentHistory;
pub use wstore::AgentSkill;
