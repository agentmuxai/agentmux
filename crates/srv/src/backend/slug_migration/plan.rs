// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The plan: which agents are candidates (§5.1), what each would be renamed
//! to, whether that id is free everywhere it could be held, and every place
//! the old one is stored. Reads only; nothing here writes.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::home::{fold, open_ro, place, table_exists, Home, Install};
use crate::backend::rpc_types::{ForkSlugBlocker, ForkSlugBlockerKind, ForkSlugCandidate, ForkSlugPlan, ForkSlugSkipped};
use agentmux_common::slug::{definition_slug, fork_slug, is_agent_id};

/// One shared definition, as the plan needs it.
#[derive(Debug, Clone)]
pub(super) struct Def {
    pub uid: String,
    pub name: String,
    pub slug: String,
    pub former: Vec<String>,
    pub parent_id: String,
    pub branch_label: String,
    pub path: PathBuf,
}

/// Every active shared definition, and the files that can't be read. A
/// file whose slug can't be read leaves the collision check incomplete, so
/// it blocks the run.
pub(super) fn load_defs(home: &Home) -> Result<(Vec<Def>, Vec<String>), String> {
    let dir = home.definitions();
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((Vec::new(), Vec::new())),
        Err(e) => return Err(format!("{} can't be listed: {e}", dir.display())),
    };
    let mut defs = Vec::new();
    let mut unreadable = Vec::new();
    for entry in entries {
        let path = entry.map_err(|e| format!("{} can't be listed: {e}", dir.display()))?.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") || !path.is_file() {
            continue;
        }
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_string();
        match std::fs::read(&path).map_err(|e| e.to_string()).and_then(|b| parse_def(&stem, &b, &path)) {
            Ok(def) => defs.push(def),
            Err(why) => unreadable.push(format!("{}: {why}", path.display())),
        }
    }
    defs.sort_by(|a, b| a.uid.cmp(&b.uid));
    Ok((defs, unreadable))
}

fn parse_def(stem: &str, bytes: &[u8], path: &Path) -> Result<Def, String> {
    let json: Value = serde_json::from_slice(bytes).map_err(|e| format!("not JSON ({e})"))?;
    let data = json.get("data").and_then(Value::as_object).ok_or("no `data` object")?;
    let text = |k: &str| data.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    if text("id") != stem {
        return Err("its `data.id` doesn't name this file".into());
    }
    let former = match data.get("former_slugs") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(list)) => list
            .iter()
            .map(|f| f.get("slug").and_then(Value::as_str).map(str::to_string))
            .collect::<Option<Vec<_>>>()
            .ok_or("a `former_slugs` entry has no slug")?,
        Some(_) => return Err("`former_slugs` isn't a list".into()),
    };
    Ok(Def {
        uid: stem.to_string(),
        name: text("name"),
        slug: text("slug"),
        former,
        parent_id: text("parent_id"),
        branch_label: text("branch_label"),
        path: path.to_path_buf(),
    })
}

/// A candidate before its collision check and locations.
#[derive(Debug, Clone)]
pub(super) struct Found {
    pub def: Def,
    pub new_slug: String,
    pub depth: usize,
}

/// §5.1: a fork whose slug is its parent's slug, `-`, and its label's slug
/// (the rule before #4746), and differs from `fork_slug` of its lineage root.
/// The parent's former slugs count as its slug: a parent migrated first (or in
/// an earlier, interrupted run) no longer has the slug the child chained.
pub(super) fn find_candidates(defs: &[Def]) -> (Vec<Found>, Vec<ForkSlugSkipped>) {
    let by_id: BTreeMap<&str, &Def> = defs.iter().map(|d| (d.uid.as_str(), d)).collect();
    let mut found = Vec::new();
    let mut skipped = Vec::new();
    for def in defs.iter().filter(|d| !d.parent_id.trim().is_empty()) {
        let skip = |reason: &str| ForkSlugSkipped {
            uid: def.uid.clone(),
            name: def.name.clone(),
            slug: def.slug.clone(),
            reason: reason.to_string(),
        };
        let Some(parent) = by_id.get(def.parent_id.as_str()) else {
            skipped.push(skip("its parent is missing, so the old rule can't be recomputed"));
            continue;
        };
        let Some((root, depth)) = lineage_root(&by_id, def) else {
            skipped.push(skip("its lineage root is missing"));
            continue;
        };
        if def.branch_label.trim().is_empty() {
            skipped.push(skip("it has no branch label"));
            continue;
        }
        if root.slug.trim().is_empty() {
            skipped.push(skip("its lineage root has no slug"));
            continue;
        }
        let new_slug = fork_slug(&root.slug, &root.name, &def.branch_label);
        if fold(&def.slug) == fold(&new_slug) {
            continue;
        }
        let label = definition_slug(&def.branch_label);
        let chained = std::iter::once(&parent.slug).chain(&parent.former).any(|p| {
            let old_rule = format!("{p}-{label}");
            fold(&def.slug) == fold(&old_rule) || fold(&def.slug) == definition_slug(&old_rule)
        });
        if !chained {
            skipped.push(skip("not a pre-fix chain: its slug isn't its parent's plus its label"));
            continue;
        }
        if !is_agent_id(&new_slug) {
            skipped.push(skip(&format!("its new id {new_slug:?} isn't a valid agent id")));
            continue;
        }
        found.push(Found { def: def.clone(), new_slug, depth });
    }
    // Parent first (§5.2): by generation, then by name.
    found.sort_by(|a, b| a.depth.cmp(&b.depth).then_with(|| a.def.name.cmp(&b.def.name)));
    (found, skipped)
}

/// The lineage root of `def` (following `parent_id`) and how many
/// generations below it `def` is; `None` if a link is missing or loops.
fn lineage_root<'a>(by_id: &BTreeMap<&str, &'a Def>, def: &'a Def) -> Option<(&'a Def, usize)> {
    let mut seen = HashSet::new();
    let mut cur = def;
    let mut depth = 0;
    while !cur.parent_id.trim().is_empty() {
        if !seen.insert(cur.uid.as_str()) {
            return None;
        }
        cur = by_id.get(cur.parent_id.as_str())?;
        depth += 1;
    }
    Some((cur, depth))
}

/// Who holds `target` other than `uid`, if anyone: a shared definition's
/// slug or former slug, a row or former slug in any `objects.db` on disk,
/// another candidate's new id, a live cross-channel registration, or a live
/// per-version registration of another agent's pane. Held only by `uid`
/// itself (its own display binding) is free. An `Err` is a store that can't
/// be read: the answer isn't known.
pub(super) fn holder_of(
    home: &Home,
    installs: &[Install],
    defs: &[Def],
    taken: &BTreeMap<String, String>,
    uid: &str,
    target: &str,
) -> Result<Option<String>, String> {
    let t = fold(target);
    for d in defs.iter().filter(|d| d.uid != uid) {
        if fold(&d.slug) == t {
            return Ok(Some(format!("agent {} ({}) has it as its id", d.name, d.uid)));
        }
        if d.former.iter().any(|f| fold(f) == t) {
            return Ok(Some(format!("agent {} ({}) was renamed from it, and it stays reserved", d.name, d.uid)));
        }
    }
    if let Some(other) = taken.get(&t).filter(|o| o.as_str() != uid) {
        return Ok(Some(format!("this run renames agent {other} to it")));
    }
    for install in installs {
        for version in &install.versions {
            let conn = open_ro(&version.objects_db())?;
            let row: Option<String> = conn
                .query_row("SELECT id FROM db_agents WHERE slug = ?1 COLLATE NOCASE AND id != ?2 LIMIT 1", [&t, uid], |r| r.get(0))
                .ok();
            if let Some(id) = row {
                return Ok(Some(format!("agent {id} has it in {}", place(install, version))));
            }
            if table_exists(&conn, "db_agent_former_slugs") {
                let former: Option<String> = conn
                    .query_row(
                        "SELECT agent_uid FROM db_agent_former_slugs WHERE slug = ?1 COLLATE NOCASE AND agent_uid != ?2 LIMIT 1",
                        [&t, uid],
                        |r| r.get(0),
                    )
                    .ok();
                if let Some(id) = former {
                    return Ok(Some(format!("agent {id} was renamed from it in {}", place(install, version))));
                }
            }
            if let Some(entry) = super::running::registration(&version.registrations(), &t) {
                let owner = super::running::block_agent(&conn, &entry.block_id);
                if owner.as_deref() != Some(uid) {
                    return Ok(Some(format!("a running pane is registered as it in {}", place(install, version))));
                }
            }
        }
    }
    for entry in super::running::shared_entries(home, &t) {
        if entry.uid != uid {
            return Ok(Some(format!("a running pane in channel {} is registered as it", entry.channel)));
        }
    }
    Ok(None)
}

/// The plan: candidates, each refused or not, with its locations and where
/// it runs; the forks left alone; and what would block an apply now.
/// `quick` stops after finding the candidates.
pub fn plan(home: &Home, quick: bool) -> Result<ForkSlugPlan, String> {
    plan_with(home, quick, true)
}

/// [`plan`]; a run that holds the migration lock itself passes `false`
/// for `check_lock`.
pub(super) fn plan_with(home: &Home, quick: bool, check_lock: bool) -> Result<ForkSlugPlan, String> {
    let (defs, unreadable) = load_defs(home)?;
    let (found, skipped) = find_candidates(&defs);
    if quick {
        let candidates = found.into_iter().map(|f| bare(&f)).collect::<Vec<_>>();
        let summary = summary(&candidates, &skipped, &[]);
        return Ok(ForkSlugPlan { candidates, skipped, blockers: Vec::new(), summary });
    }
    let installs = home.installs()?;
    let mut blockers: Vec<ForkSlugBlocker> = unreadable
        .iter()
        .map(|u| ForkSlugBlocker::new(ForkSlugBlockerKind::Unreadable, format!("A shared definition can't be read, so no new id can be checked: {u}")))
        .collect();
    let mut taken: BTreeMap<String, String> = BTreeMap::new();
    let mut candidates = Vec::new();
    for f in &found {
        let mut c = bare(f);
        if unreadable.is_empty() {
            match holder_of(home, &installs, &defs, &taken, &f.def.uid, &f.new_slug)? {
                Some(holder) => {
                    c.refused = Some(format!("{} is taken: {holder}", f.new_slug));
                    c.suggestion = suggest(home, &installs, &defs, &taken, &f.def.uid, &f.new_slug)?;
                }
                None => {
                    taken.insert(fold(&f.new_slug), f.def.uid.clone());
                }
            }
        } else {
            c.refused = Some("the shared definitions can't all be read".into());
        }
        c.locations = super::locations::locations(home, &installs, &f.def, &f.new_slug)?;
        c.running = super::running::running(home, &installs, &f.def)?;
        candidates.push(c);
    }
    if candidates.iter().any(|c| c.refused.is_none()) {
        blockers.extend(super::preconditions::blockers(home, &installs, &candidates, check_lock));
    }
    let summary = summary(&candidates, &skipped, &blockers);
    Ok(ForkSlugPlan { candidates, skipped, blockers, summary })
}

fn bare(f: &Found) -> ForkSlugCandidate {
    ForkSlugCandidate {
        uid: f.def.uid.clone(),
        name: f.def.name.clone(),
        old_slug: f.def.slug.clone(),
        new_slug: f.new_slug.clone(),
        refused: None,
        suggestion: None,
        locations: Vec::new(),
        running: Vec::new(),
    }
}

/// The first of `target-2`, `target-3`, … nobody holds. Suggested, never
/// taken: a collision suffix is the user's choice (§5.1).
fn suggest(
    home: &Home,
    installs: &[Install],
    defs: &[Def],
    taken: &BTreeMap<String, String>,
    uid: &str,
    target: &str,
) -> Result<Option<String>, String> {
    for n in 2..100 {
        let next = format!("{target}-{n}");
        if is_agent_id(&next) && holder_of(home, installs, defs, taken, uid, &next)?.is_none() {
            return Ok(Some(next));
        }
    }
    Ok(None)
}

fn summary(candidates: &[ForkSlugCandidate], skipped: &[ForkSlugSkipped], blockers: &[ForkSlugBlocker]) -> String {
    let ready = candidates.iter().filter(|c| c.refused.is_none()).count();
    let mut lines = Vec::new();
    if candidates.is_empty() {
        lines.push("No agent has an outdated id.".to_string());
    } else {
        lines.push(format!(
            "{} agent{} {} an outdated id; {ready} can be updated now.",
            candidates.len(),
            if candidates.len() == 1 { "" } else { "s" },
            if candidates.len() == 1 { "has" } else { "have" },
        ));
    }
    for c in candidates {
        match &c.refused {
            None => lines.push(format!("  {} : {} -> {}", c.name, c.old_slug, c.new_slug)),
            Some(why) => lines.push(format!("  {} : {} -> {} refused: {why}", c.name, c.old_slug, c.new_slug)),
        }
    }
    if !skipped.is_empty() {
        lines.push(format!("{} fork{} left alone (not a pre-fix chain).", skipped.len(), if skipped.len() == 1 { "" } else { "s" }));
    }
    for b in blockers {
        lines.push(format!("Blocked: {}", b.message));
    }
    lines.join("\n")
}
