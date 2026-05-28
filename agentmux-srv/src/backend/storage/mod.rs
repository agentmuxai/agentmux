// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Storage layer: SQLite-backed object store and file store.
//! Port of Go's pkg/wstore and pkg/filestore.

pub mod agents_consolidate;
pub mod error;
pub mod filestore;
pub mod memory_bundles;
pub mod migrations;
pub mod snapshot;
pub mod store;

pub use error::StoreError;
pub use memory_bundles::Memory;
pub use store::AgentDefinition;
pub use store::AgentContent;
#[allow(unused_imports)]
pub use store::AgentHistory;
pub use store::AgentSkill;
