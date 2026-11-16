// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Listing every channel's `objects.db`, or saying the list is partial.

use std::path::{Path, PathBuf};

use super::migrate::enumerate_sources;

/// Every channel's `objects.db` under `home`, or `None` when a directory that
/// should have been listed could not be read. A caller that must see EVERY
/// channel's store (to know nothing elsewhere still uses a thing) can't trust a
/// partial list.
pub fn enumerate_objects_dbs_complete(home: &Path) -> Option<Vec<PathBuf>> {
    let (sources, incomplete) = enumerate_sources(home);
    (!incomplete).then(|| sources.into_iter().map(|s| s.db_path).collect())
}
