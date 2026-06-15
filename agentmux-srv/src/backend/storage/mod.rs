// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Storage layer: SQLite-backed object store and file store.
//! Port of Go's pkg/wstore and pkg/filestore.

pub mod agents;
pub mod agents_consolidate;
pub mod content;
pub mod def_registry_mirror;
pub mod dual_write;
pub mod error;
pub mod filestore;
pub mod history;
pub mod identities;
pub mod memory_bundles;
pub mod migrations;
pub mod muxbus;
pub mod registry_mirror;
pub mod skills;
pub mod snapshot;
pub mod store;

pub use agents::{AgentDefinition, AgentInstance, InstanceStatus, InstanceUpdate};
pub use content::AgentContent;
pub use error::StoreError;
#[allow(unused_imports)]
pub use history::AgentHistory;
pub use identities::{AgentIdentityLink, Identity, IdentityAccount, IdentityBinding, SecretRef};
pub use memory_bundles::Memory;
pub use skills::AgentSkill;
