// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

//! Storage layer: SQLite-backed object store and file store.
//! Port of Go's pkg/wstore and pkg/filestore.

pub mod error;
pub mod filestore;
pub mod migrations;
pub mod wstore;

pub use error::StoreError;
pub use wstore::ForgeAgent;
pub use wstore::ForgeContent;
#[allow(unused_imports)]
pub use wstore::ForgeHistory;
pub use wstore::ForgeSkill;
