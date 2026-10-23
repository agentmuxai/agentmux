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
//!
//! Two views of one cache. [`SlugIndex::all`] is the records `list_active`
//! would list, for resolving names. [`SlugIndex::all_strict`] is every record
//! whose slug fields can be read, newer schemas included (`slug_fields`), and
//! fails when any file or the directory can't be: allocating a new slug
//! against an incomplete view could take one an unseen agent holds.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use serde_json::Value;

/// The slug and former slugs of one active shared definition, and the
/// display names it goes by now (its `name` and `branch_label`, non-empty),
/// which a former display name of another agent never outranks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SlugEntry {
    pub slug: String,
    pub former: Vec<String>,
    pub display: Vec<String>,
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
    /// Its slug fields, or why they can't be read.
    slugs: Result<SlugEntry, String>,
    /// Whether the whole record validates (`list_active` would list it).
    valid: bool,
}

#[derive(Default)]
struct State {
    by_id: HashMap<String, Cached>,
    /// Files seen but not statted on the last scan, for the strict view.
    unstatted: Vec<String>,
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
        s.scan(root)?;
        Ok(s.snapshot())
    }

    /// Every active definition's slug fields, or an error naming what can't
    /// be read: the directory, a file, or a record's slug fields.
    pub fn all_strict(&self, root: &Path) -> Result<HashMap<String, SlugEntry>, String> {
        let mut s = self.0.lock().unwrap_or_else(|e| e.into_inner());
        s.scan(root).map_err(|e| format!("{} can't be listed: {e}", root.display()))?;
        if let Some(file) = s.unstatted.first() {
            return Err(format!("{file} can't be read"));
        }
        s.by_id
            .iter()
            .map(|(id, c)| match &c.slugs {
                Ok(entry) => Ok((id.clone(), entry.clone())),
                Err(why) => Err(format!("{id}.json: {why}")),
            })
            .collect()
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
        Ok(s.by_id.get(id).filter(|c| c.valid).and_then(|c| c.slugs.clone().ok()))
    }

    #[cfg(test)]
    pub fn parses(&self) -> usize {
        self.0.lock().unwrap().parses
    }
}

impl State {
    /// Stat every `.json` file in `root`, reparse the changed ones, and drop
    /// the ones that are gone.
    fn scan(&mut self, root: &Path) -> std::io::Result<()> {
        let mut seen = HashSet::new();
        self.unstatted.clear();
        for dirent in std::fs::read_dir(root)? {
            let path = dirent?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Some(id) = path.file_stem().and_then(|s| s.to_str()).map(str::to_string) else {
                continue;
            };
            // A real stat, not the directory listing's copy of it.
            let meta = match std::fs::metadata(&path) {
                Ok(meta) => meta,
                Err(e) => {
                    self.unstatted.push(format!("{} ({e})", path.display()));
                    continue;
                }
            };
            if !meta.is_file() {
                continue;
            }
            self.refresh(&id, &path, Stamp::of(&meta));
            seen.insert(id);
        }
        let before = self.by_id.len();
        self.by_id.retain(|id, _| seen.contains(id));
        if self.by_id.len() != before {
            self.snapshot = None;
        }
        Ok(())
    }

    fn refresh(&mut self, id: &str, path: &Path, stamp: Stamp) {
        if matches!(self.by_id.get(id), Some(c) if c.stamp == stamp && !c.racy) {
            return;
        }
        let parsed_at = SystemTime::now();
        let (slugs, valid) = match std::fs::read(path) {
            Ok(bytes) => {
                let valid = match super::def_store::parse_and_validate(&bytes, id) {
                    Ok(_) => true,
                    Err(e) => {
                        tracing::warn!(file = %path.display(), error = %e, "def registry: skipping invalid record");
                        false
                    }
                };
                (slug_fields(id, &bytes), valid)
            }
            Err(e) => (Err(format!("can't be read: {e}")), false),
        };
        #[cfg(test)]
        {
            self.parses += 1;
        }
        let racy = stamp.modified.is_none_or(|m| parsed_at.duration_since(m).map_or(true, |age| age < RACY_WINDOW));
        let changed = self.by_id.get(id).is_none_or(|c| c.slugs != slugs || c.valid != valid);
        self.by_id.insert(id.to_string(), Cached { stamp, racy, slugs, valid });
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
                        .filter(|(_, c)| c.valid)
                        .filter_map(|(id, c)| c.slugs.clone().ok().map(|e| (id.clone(), e)))
                        .collect(),
                )
            })
            .clone()
    }
}

/// A record's slug and former slugs, read from its raw JSON so that a record
/// this build can't otherwise load (a newer schema, a field it doesn't know)
/// still answers, as long as these fields are where this build puts them:
/// `data.id` naming the file, `data.slug` a string (absent reads as empty),
/// `data.former_slugs` a list of objects with a string `slug` (absent reads as
/// none). Anything else is an error naming what is wrong. The display names
/// (`data.name`, `data.branch_label`) are read when they are strings and
/// skipped otherwise: they never make a record's slugs unreadable.
pub(super) fn slug_fields(id: &str, bytes: &[u8]) -> Result<SlugEntry, String> {
    let json: Value = serde_json::from_slice(bytes).map_err(|e| format!("not JSON ({e})"))?;
    let data = json.get("data").and_then(Value::as_object).ok_or("no `data` object")?;
    if data.get("id").and_then(Value::as_str) != Some(id) {
        return Err("its `data.id` doesn't name this file".to_string());
    }
    let slug = match data.get("slug") {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(_) => return Err("`data.slug` isn't a string".to_string()),
    };
    let former = match data.get("former_slugs") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(entries)) => entries
            .iter()
            .map(|f| f.get("slug").and_then(Value::as_str).map(str::to_string))
            .collect::<Option<Vec<_>>>()
            .ok_or("a `data.former_slugs` entry has no string `slug`")?,
        Some(_) => return Err("`data.former_slugs` isn't a list".to_string()),
    };
    let display = ["name", "branch_label"]
        .iter()
        .filter_map(|k| data.get(*k).and_then(Value::as_str))
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string)
        .collect();
    Ok(SlugEntry { slug, former, display })
}
