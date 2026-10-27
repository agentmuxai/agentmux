// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The plan: which agents are candidates (§5.1), what each would be renamed
//! to, whether that id is free everywhere it could be held, and every place
//! the old one is stored. Reads only; nothing here writes.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::home::{fold, has_agent_slugs, has_column, open_ro, place, table_exists, Home, Install};
use crate::backend::rpc_types::{
    ForkSlugBlocker, ForkSlugBlockerKind, ForkSlugCandidate, ForkSlugChoice, ForkSlugDuplicate, ForkSlugPlan, ForkSlugSkipped,
};
use agentmux_common::slug::{agent_route_name, definition_slug, fork_slug, is_agent_id};
use rusqlite::OptionalExtension;

/// One shared definition, as the plan needs it.
#[derive(Debug, Clone, Default)]
pub(super) struct Def {
    pub uid: String,
    pub name: String,
    pub slug: String,
    pub former: Vec<String>,
    pub parent_id: String,
    pub branch_label: String,
    pub created_at: i64,
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
        created_at: data.get("created_at").and_then(Value::as_i64).unwrap_or(0),
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
    // Only forks: an agent without a branch label is a lineage root, whatever
    // its `parent_id` says (one made from a template keeps the template's id
    // there), as `fork_lineage_root_id` reads it.
    for def in defs.iter().filter(|d| !d.parent_id.trim().is_empty() && !d.branch_label.is_empty()) {
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

/// The lineage root of `def` and how many generations below it `def` is:
/// `parent_id` is followed only while the agent is a fork (it has a branch
/// label), so the root is the first agent without one, and a template in
/// its `parent_id` ends the walk there (template.rs `fork_lineage_root_id`).
/// `None` if a fork's parent is missing or the links loop.
fn lineage_root<'a>(by_id: &BTreeMap<&str, &'a Def>, def: &'a Def) -> Option<(&'a Def, usize)> {
    let mut seen = HashSet::new();
    let mut cur = def;
    let mut depth = 0;
    // Exactly `fork_lineage_root_id`'s test: a whitespace-only label still
    // makes a fork (#5366).
    while !cur.branch_label.is_empty() && !cur.parent_id.is_empty() {
        if !seen.insert(cur.uid.as_str()) {
            return None;
        }
        cur = by_id.get(cur.parent_id.as_str())?;
        depth += 1;
    }
    Some((cur, depth))
}

/// Whether a display or instance name routes to `folded`, as a message's
/// target does (`agent_route_name`, case-insensitive).
fn routes_to(name: &str, folded: &str) -> bool {
    !name.trim().is_empty() && fold(&agent_route_name(name)) == folded
}

/// Who holds `target` other than `uid`, if anyone: a shared definition's
/// slug, former slug or display route; a row's slug, former slug, display or
/// instance route, or former display name's route (which still addresses
/// its agent, as `resolve_slug_collision` keeps it) in any `objects.db` on
/// disk; another candidate's new id; a
/// live cross-channel registration; or a live per-version registration of
/// another agent's pane. A display route counts because a message to the
/// target would otherwise reach whichever agent holds it as its slug, and
/// the agent it was meant for would never see it. Held only by `uid` itself
/// (its own display binding) is free. An `Err` is a store that can't be
/// read: the answer isn't known.
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
        if routes_to(&d.name, &t) {
            return Ok(Some(format!("messages to it reach agent {} ({}), whose name routes there", d.name, d.uid)));
        }
        // A fork's name is its branch label, and messages reach it too.
        if routes_to(&d.branch_label, &t) {
            return Ok(Some(format!("messages to it reach agent {} ({}), whose branch label routes there", d.name, d.uid)));
        }
    }
    if let Some(other) = taken.get(&t).filter(|o| o.as_str() != uid) {
        return Ok(Some(format!("this run renames agent {other} to it")));
    }
    for install in installs {
        for version in &install.versions {
            let db = version.objects_db();
            let failed = |e: rusqlite::Error| format!("{} can't be read: {e}", db.display());
            let conn = open_ro(&db)?;
            let slugs = has_agent_slugs(&conn);
            let row: Option<String> = if slugs {
                conn.query_row("SELECT id FROM db_agents WHERE slug = ?1 COLLATE NOCASE AND id != ?2 LIMIT 1", [&t, uid], |r| r.get(0))
                    .optional()
                    .map_err(failed)?
            } else {
                None
            };
            if let Some(id) = row {
                return Ok(Some(format!("agent {id} has it in {}", place(install, version))));
            }
            let routed: Option<(String, String)> = if !slugs {
                None
            } else {
                // An older db_agents may lack a route column: it routes nothing.
                let col = |c: &str| if has_column(&conn, "db_agents", c) { format!("COALESCE({c}, '')") } else { "''".to_string() };
                let (name, instance, label) = (col("name"), col("instance_name"), col("branch_label"));
                let mut stmt = conn
                    .prepare(&format!("SELECT id, {name}, {instance}, {label} FROM db_agents WHERE id != ?1"))
                    .map_err(failed)?;
                let rows = stmt
                    .query_map([uid], |r| {
                        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?))
                    })
                    .map_err(failed)?;
                let mut hit = None;
                for row in rows {
                    let (id, name, instance, branch_label) = row.map_err(failed)?;
                    if let Some(route) = [name, instance, branch_label].into_iter().find(|n| routes_to(n, &t)) {
                        hit = Some((id, route));
                        break;
                    }
                }
                hit
            };
            if let Some((id, name)) = routed {
                return Ok(Some(format!("messages to it reach agent {id} in {}, whose name {name:?} routes there", place(install, version))));
            }
            if table_exists(&conn, "db_agent_former_slugs") {
                let former: Option<String> = conn
                    .query_row(
                        "SELECT agent_uid FROM db_agent_former_slugs WHERE slug = ?1 COLLATE NOCASE AND agent_uid != ?2 LIMIT 1",
                        [&t, uid],
                        |r| r.get(0),
                    )
                    .optional()
                    .map_err(failed)?;
                if let Some(id) = former {
                    return Ok(Some(format!("agent {id} was renamed from it in {}", place(install, version))));
                }
            }
            if table_exists(&conn, "db_agent_former_names") {
                let mut stmt = conn.prepare("SELECT agent_id, name FROM db_agent_former_names WHERE agent_id != ?1").map_err(failed)?;
                let rows = stmt.query_map([uid], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))).map_err(failed)?;
                for row in rows {
                    let (id, name) = row.map_err(failed)?;
                    if routes_to(&name, &t) {
                        return Ok(Some(format!(
                            "agent {id} was called {name:?} in {}, and messages to that name still reach it",
                            place(install, version)
                        )));
                    }
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
    plan_with(home, quick, true, &[])
}

/// [`plan`] with ids the user picked for some agents.
pub fn plan_choosing(home: &Home, quick: bool, choices: &[ForkSlugChoice]) -> Result<ForkSlugPlan, String> {
    plan_with(home, quick, true, choices)
}

/// Give each chosen agent the id the user picked in place of the computed one. The pick goes
/// through the same collision check as any other new id (an agent whose pick is taken is
/// refused again, with the new reason), so a choice can't claim an id something holds.
/// An error for a pick that can't be an id for that agent at all.
fn apply_choices(found: &mut [Found], choices: &[ForkSlugChoice]) -> Result<(), String> {
    for choice in choices {
        let slug = choice.new_slug.trim();
        let Some(f) = found.iter_mut().find(|f| f.def.uid == choice.uid) else {
            return Err(format!("{} isn't an agent with an outdated id", choice.uid));
        };
        if !is_agent_id(slug) {
            return Err(format!("{slug:?} isn't a valid agent id"));
        }
        if fold(slug) == fold(&f.def.slug) {
            return Err(format!("{} already has the id {slug}", f.def.name));
        }
        f.new_slug = slug.to_string();
    }
    Ok(())
}

/// [`plan`]; a run that holds the migration lock itself passes `false`
/// for `check_lock`. `choices` give agents an id of the user's picking.
pub(super) fn plan_with(home: &Home, quick: bool, check_lock: bool, choices: &[ForkSlugChoice]) -> Result<ForkSlugPlan, String> {
    let (defs, unreadable) = load_defs(home)?;
    let (mut found, skipped) = find_candidates(&defs);
    if !quick {
        apply_choices(&mut found, choices)?;
    }
    if quick {
        let candidates = found.into_iter().map(|f| bare(&f)).collect::<Vec<_>>();
        let summary = summary(&candidates, &skipped, &[], &[]);
        return Ok(ForkSlugPlan { candidates, skipped, blockers: Vec::new(), summary, last_run: last_run(home), duplicates: Vec::new() });
    }
    let installs = home.installs()?;
    let mut blockers: Vec<ForkSlugBlocker> = unreadable
        .iter()
        .map(|u| ForkSlugBlocker::new(ForkSlugBlockerKind::Unreadable, format!("A shared definition can't be read, so no new id can be checked: {u}")))
        .collect();
    blockers.extend(unreadable_stores(home, &installs).into_iter().map(|u| {
        ForkSlugBlocker::new(ForkSlugBlockerKind::Unreadable, format!("A database can't be read, so no new id can be checked: {u}"))
    }));
    if !blockers.is_empty() {
        // Nothing past this point can be trusted: a store that can't be read
        // may hold any target. Every candidate waits, none is "free".
        let candidates: Vec<ForkSlugCandidate> = found
            .iter()
            .map(|f| ForkSlugCandidate { refused: Some("not every store can be read, so its new id can't be checked".into()), ..bare(f) })
            .collect();
        // The shared definitions alone: a store that can't be read may hold
        // more, and no proposal can be checked.
        let duplicates = super::duplicates::duplicates(home, &installs, &defs, &BTreeMap::new(), false);
        let summary = summary(&candidates, &skipped, &blockers, &duplicates);
        return Ok(ForkSlugPlan { candidates, skipped, blockers, summary, last_run: last_run(home), duplicates });
    }
    let mut taken: BTreeMap<String, String> = BTreeMap::new();
    let mut candidates = Vec::new();
    for f in &found {
        let mut c = bare(f);
        match holder_of(home, &installs, &defs, &taken, &f.def.uid, &f.new_slug) {
            Ok(Some(holder)) => {
                c.refused = Some(format!("{} is taken: {holder}", f.new_slug));
                c.suggestion = suggest(home, &installs, &defs, &taken, &f.def.uid, &f.new_slug).unwrap_or(None);
            }
            Ok(None) => {
                taken.insert(fold(&f.new_slug), f.def.uid.clone());
            }
            Err(e) => {
                c.refused = Some("its new id can't be checked".into());
                blockers.push(ForkSlugBlocker::new(ForkSlugBlockerKind::Unreadable, format!("A database can't be read, so no new id can be checked: {e}")));
                candidates.push(c);
                continue;
            }
        }
        c.locations = super::locations::locations(home, &installs, &f.def, &f.new_slug)?;
        c.running = super::running::running(home, &installs, &f.def)?;
        candidates.push(c);
    }
    if candidates.iter().any(|c| c.refused.is_none()) {
        blockers.extend(super::preconditions::blockers(home, &installs, &candidates, check_lock));
    }
    let duplicates = super::duplicates::duplicates(home, &installs, &defs, &taken, true);
    let summary = summary(&candidates, &skipped, &blockers, &duplicates);
    Ok(ForkSlugPlan { candidates, skipped, blockers, summary, last_run: last_run(home), duplicates })
}

/// The latest run, unless it was rolled back. A journal that can't be read
/// counts as no run: the plan still shows, and an apply reports the error.
fn last_run(home: &Home) -> Option<crate::backend::rpc_types::ForkSlugLastRun> {
    let (path, journal) = super::journal::journals(&home.backups())
        .into_iter()
        .find_map(|p| super::journal::Journal::read(&p).ok().map(|j| (p, j)))?;
    if journal.rolled_back() {
        return None;
    }
    Some(crate::backend::rpc_types::ForkSlugLastRun {
        journal: path.display().to_string(),
        finished: journal.finished().is_some(),
        failed: journal.last_failure(),
    })
}

/// Every database a run reads or writes that can't be read now, by path
/// and why: a corrupt or incompatible `objects.db` (its agents), `wan.db` or
/// host store. A query that finds no row is not a failure; one that can't
/// run is.
pub(super) fn unreadable_stores(home: &Home, installs: &[Install]) -> Vec<String> {
    let mut out = Vec::new();
    let probe = |db: &Path, sql: &str, out: &mut Vec<String>| {
        let result = open_ro(db).and_then(|c| {
            c.query_row(sql, [], |_| Ok(())).optional().map(|_| ()).map_err(|e| e.to_string())
        });
        if let Err(e) = result {
            out.push(format!("{} ({e})", db.display()));
        }
    };
    for install in installs {
        for version in &install.versions {
            // An objects.db older than `db_agents.slug` holds no slug: only a
            // database that can't be read at all blocks (#5342).
            probe(&version.objects_db(), "SELECT name FROM sqlite_master LIMIT 1", &mut out);
        }
        if let Some(wan) = install.wan_db() {
            probe(&wan, "SELECT name FROM sqlite_master LIMIT 1", &mut out);
        }
    }
    for db in home.host_stores(installs) {
        probe(&db, "SELECT name FROM sqlite_master LIMIT 1", &mut out);
    }
    out
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

fn summary(candidates: &[ForkSlugCandidate], skipped: &[ForkSlugSkipped], blockers: &[ForkSlugBlocker], duplicates: &[ForkSlugDuplicate]) -> String {
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
    for d in duplicates {
        lines.push(format!("{} agents share the id {} (an update doesn't change it):", d.holders.len(), d.slug));
        for (i, h) in d.holders.iter().enumerate() {
            let what = match (i, &h.proposed) {
                (0, _) => "keeps it, as the oldest".to_string(),
                (_, Some(p)) => format!("could be renamed to {p}"),
                (_, None) => "needs a new id".to_string(),
            };
            lines.push(format!("  {} ({}, {}): {what}", h.name, h.uid, h.place));
        }
    }
    lines.join("\n")
}
