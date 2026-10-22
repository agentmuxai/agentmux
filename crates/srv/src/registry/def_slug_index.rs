// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A cached index of every shared definition's slug and former slugs, for
//! the slug-to-uid reads that run on every message
//! (`docs/specs/SPEC_FORK_SLUG_MIGRATION_2026_10_22.md` P0.1).
//!
//! A file is parsed again only when it may have changed. Each lookup stats
//! the files (no reads) and reparses one whose modification time or size
//! differs from what was cached, or that this process wrote since
//! ([`SlugIndex::invalidate`]). Other channels and instances write these
//! files too, so the on-disk stamp, not an in-process signal, is what keeps
//! the cache current. A stamp is trusted only once the file is older than
//! [`RACY_WINDOW`] at parse time, as git does for its index: a filesystem with
//! coarse timestamps could otherwise give a quick rewrite of the same size
//! the same stamp.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

/// The slug and former slugs of one active shared definition.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SlugEntry {
    pub slug: String,
    pub former: Vec<String>,
}

/// Wider than the coarsest common timestamp granularity (FAT's 2 s).
const RACY_WINDOW: Duration = Duration::from_secs(3);

/// What a file looked like when it was parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stamp {
    modified: Option<SystemTime>,
    len: u64,
}

impl Stamp {
    fn of(meta: &std::fs::Metadata) -> Stamp {
        Stamp { modified: meta.modified().ok(), len: meta.len() }
    }
}

struct Cached {
    stamp: Stamp,
    /// The stamp was taken too close to a write to rule out a second one.
    racy: bool,
    /// `None` for a file that didn't parse or validate (`list_active` skips it).
    entry: Option<SlugEntry>,
}

#[derive(Default)]
struct State {
    by_id: HashMap<String, Cached>,
    /// Rebuilt from `by_id` when a file changed; `None` until then.
    snapshot: Option<Arc<HashMap<String, SlugEntry>>>,
    #[cfg(test)]
    parses: usize,
}

/// Owned by a `DefinitionStore`; one lock, held for a lookup's stats and any
/// reparse, so concurrent readers never see a half-updated index.
#[derive(Default)]
pub(super) struct SlugIndex(Mutex<State>);

impl SlugIndex {
    /// Forget `id`, after this process wrote, retired or deleted its file.
    pub fn invalidate(&self, id: &str) {
        let mut s = self.0.lock().unwrap_or_else(|e| e.into_inner());
        s.by_id.remove(id);
        s.snapshot = None;
    }

    /// Every valid active definition in `root`, by id.
    pub fn all(&self, root: &Path) -> std::io::Result<Arc<HashMap<String, SlugEntry>>> {
        let mut s = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let mut seen = HashSet::new();
        for dirent in std::fs::read_dir(root)? {
            let path = dirent?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Some(id) = path.file_stem().and_then(|s| s.to_str()).map(str::to_string) else {
                continue;
            };
            // A real stat, not the directory listing's copy of it.
            let Ok(meta) = std::fs::metadata(&path) else { continue };
            if !meta.is_file() {
                continue;
            }
            s.refresh(&id, &path, Stamp::of(&meta));
            seen.insert(id);
        }
        let before = s.by_id.len();
        s.by_id.retain(|id, _| seen.contains(id));
        if s.by_id.len() != before {
            s.snapshot = None;
        }
        Ok(s.snapshot())
    }

    /// One definition, statting only its own file.
    pub fn one(&self, root: &Path, id: &str) -> std::io::Result<Option<SlugEntry>> {
        let path = root.join(format!("{id}.json"));
        let meta = match std::fs::metadata(&path) {
            Ok(m) if m.is_file() => m,
            Ok(_) => return Ok(None),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                self.invalidate(id);
                return Ok(None);
            }
            Err(e) => return Err(e),
        };
        let mut s = self.0.lock().unwrap_or_else(|e| e.into_inner());
        s.refresh(id, &path, Stamp::of(&meta));
        Ok(s.by_id.get(id).and_then(|c| c.entry.clone()))
    }

    #[cfg(test)]
    pub fn parses(&self) -> usize {
        self.0.lock().unwrap().parses
    }
}

impl State {
    fn refresh(&mut self, id: &str, path: &Path, stamp: Stamp) {
        if matches!(self.by_id.get(id), Some(c) if c.stamp == stamp && !c.racy) {
            return;
        }
        let parsed_at = SystemTime::now();
        let entry = match super::def_store::read_and_validate(path, id) {
            Ok(rec) => Some(SlugEntry {
                slug: rec.data.slug,
                former: rec.data.former_slugs.into_iter().map(|f| f.slug).collect(),
            }),
            Err(e) => {
                tracing::warn!(file = %path.display(), error = %e, "def registry: skipping invalid record");
                None
            }
        };
        #[cfg(test)]
        {
            self.parses += 1;
        }
        let racy = stamp.modified.is_none_or(|m| parsed_at.duration_since(m).map_or(true, |age| age < RACY_WINDOW));
        let changed = self.by_id.get(id).is_none_or(|c| c.entry != entry);
        self.by_id.insert(id.to_string(), Cached { stamp, racy, entry });
        if changed {
            self.snapshot = None;
        }
    }

    fn snapshot(&mut self) -> Arc<HashMap<String, SlugEntry>> {
        self.snapshot
            .get_or_insert_with(|| {
                Arc::new(
                    self.by_id
                        .iter()
                        .filter_map(|(id, c)| c.entry.clone().map(|e| (id.clone(), e)))
                        .collect(),
                )
            })
            .clone()
    }
}
