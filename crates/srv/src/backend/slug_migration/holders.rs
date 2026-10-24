// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Who holds a slug on this host outside the shared definitions: every
//! `objects.db` on disk (a row's slug, a former slug, a former display name's
//! route) and every live registration. A new agent's slug is picked against
//! these as well as the shared definitions, under the host-wide slug
//! allocation lock, so a row only one channel or version knows (an agent from
//! before the shared definitions, a template launch, a copy that was never
//! published) can't be given to a second agent; and the definition mirror
//! reads them before it publishes a slug. The same holders `plan::holder_of`
//! checks, read in one pass.
//!
//! Each `objects.db` is read once and kept until its file (or its WAL)
//! changes: most of them are older versions nobody runs, and a host can have
//! hundreds.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

use super::home::{fold, has_agent_slugs, open_ro, place, table_exists, Home, Install};

/// What a row holds a name as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Slug,
    FormerSlug,
    /// A former display name, by its route: it still addresses its agent.
    FormerName,
}

#[derive(Debug, Clone)]
struct Held {
    kind: Kind,
    /// Folded as every key table folds.
    name: String,
    uid: String,
}

/// One `objects.db`'s file state: a change to the database or its WAL
/// changes one of these.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Stamp(Vec<(u64, Option<SystemTime>)>);

fn stamp(db: &Path) -> Stamp {
    let mut wal = db.as_os_str().to_owned();
    wal.push("-wal");
    Stamp(
        [db.to_path_buf(), PathBuf::from(wal)]
            .iter()
            .map(|p| std::fs::metadata(p).map(|m| (m.len(), m.modified().ok())).unwrap_or((0, None)))
            .collect(),
    )
}

type Cache = Mutex<HashMap<PathBuf, (Stamp, Arc<Vec<Held>>)>>;

fn cache() -> &'static Cache {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// One `objects.db`'s holders, read now. An `objects.db` older than
/// `db_agents.slug` holds none; one that can't be read is an error.
fn read_db(db: &Path) -> Result<Vec<Held>, String> {
    let conn = open_ro(db)?;
    let failed = |e: rusqlite::Error| format!("{} can't be read: {e}", db.display());
    // The schema checks below read a failure as "no such table", so a file
    // that isn't a database is caught here (as `plan::unreadable_stores` does).
    conn.query_row("SELECT name FROM sqlite_master LIMIT 1", [], |_| Ok(()))
        .or_else(|e| if matches!(e, rusqlite::Error::QueryReturnedNoRows) { Ok(()) } else { Err(e) })
        .map_err(failed)?;
    let mut out = Vec::new();
    if has_agent_slugs(&conn) {
        let mut stmt = conn.prepare("SELECT id, COALESCE(slug, '') FROM db_agents").map_err(failed)?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))).map_err(failed)?;
        for row in rows {
            let (uid, slug) = row.map_err(failed)?;
            if !slug.trim().is_empty() {
                out.push(Held { kind: Kind::Slug, name: fold(&slug), uid });
            }
        }
    }
    if table_exists(&conn, "db_agent_former_slugs") {
        let mut stmt = conn.prepare("SELECT agent_uid, slug FROM db_agent_former_slugs").map_err(failed)?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))).map_err(failed)?;
        for row in rows {
            let (uid, slug) = row.map_err(failed)?;
            out.push(Held { kind: Kind::FormerSlug, name: fold(&slug), uid });
        }
    }
    if table_exists(&conn, "db_agent_former_names") {
        let mut stmt = conn.prepare("SELECT agent_id, name FROM db_agent_former_names").map_err(failed)?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))).map_err(failed)?;
        for row in rows {
            let (uid, name) = row.map_err(failed)?;
            if let Some(route) = route_key(&name) {
                out.push(Held { kind: Kind::FormerName, name: route, uid });
            }
        }
    }
    Ok(out)
}

/// The key a display name is addressed by, as `resolve_slug_collision` and
/// `plan::holder_of` read a former one: its route name, folded; `None` for a
/// name no message can address.
fn route_key(name: &str) -> Option<String> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let route = agentmux_common::slug::agent_route_name(name);
    agentmux_common::slug::is_agent_id(&route).then(|| route.to_lowercase())
}

/// Every holder on this host outside the shared definitions, as one read
/// found them.
pub struct HostSlugs {
    home: Home,
    installs: Vec<Install>,
    /// Folded name → what holds it, and where.
    by_name: HashMap<String, Vec<(Held, String)>>,
}

impl HostSlugs {
    /// Read every `objects.db` under `home` (each from its cache while its
    /// files are unchanged). A directory that can't be listed or a database
    /// that can't be read is an error: a slug picked without it could be one
    /// that store holds.
    pub fn read(home: &Home) -> Result<HostSlugs, String> {
        let installs = home.installs()?;
        let mut by_name: HashMap<String, Vec<(Held, String)>> = HashMap::new();
        for install in &installs {
            for version in &install.versions {
                let db = version.objects_db();
                let now = stamp(&db);
                let cached = cache().lock().unwrap_or_else(|e| e.into_inner()).get(&db).filter(|(s, _)| *s == now).map(|(_, h)| h.clone());
                let held = match cached {
                    Some(h) => h,
                    None => {
                        let h = Arc::new(read_db(&db)?);
                        cache().lock().unwrap_or_else(|e| e.into_inner()).insert(db.clone(), (now, h.clone()));
                        h
                    }
                };
                let at = place(install, version);
                for h in held.iter() {
                    by_name.entry(h.name.clone()).or_default().push((h.clone(), at.clone()));
                }
            }
        }
        Ok(HostSlugs { home: home.clone(), installs, by_name })
    }

    /// Who other than `uid` holds `name` as a slug or a former slug in some
    /// `objects.db`, described for a log line or an error.
    pub fn slug_holder(&self, name: &str, uid: &str) -> Option<String> {
        self.held(name, uid, &[Kind::Slug, Kind::FormerSlug])
    }

    /// [`Self::slug_holder`], or a former display name of another agent that
    /// routes to `name`, or a live registration of another agent's pane under
    /// it: everything a new slug must not be.
    pub fn holder(&self, name: &str, uid: &str) -> Option<String> {
        self.held(name, uid, &[Kind::Slug, Kind::FormerSlug, Kind::FormerName])
            .or_else(|| self.registered(&fold(name), uid))
    }

    fn held(&self, name: &str, uid: &str, kinds: &[Kind]) -> Option<String> {
        let (h, at) = self.by_name.get(&fold(name))?.iter().find(|(h, _)| h.uid != uid && kinds.contains(&h.kind))?;
        Some(match h.kind {
            Kind::Slug => format!("agent {} has it in {at}", h.uid),
            Kind::FormerSlug => format!("agent {} was renamed from it in {at}", h.uid),
            Kind::FormerName => format!("agent {} had a name that routes to it in {at}", h.uid),
        })
    }

    /// A live registration under `name` of a pane that isn't `uid`'s, in any
    /// version or in the cross-channel registry (`plan::holder_of`'s rule).
    fn registered(&self, name: &str, uid: &str) -> Option<String> {
        for install in &self.installs {
            for version in &install.versions {
                let Some(entry) = super::running::registration(&version.registrations(), name) else { continue };
                let owner = open_ro(&version.objects_db()).ok().and_then(|c| super::running::block_agent(&c, &entry.block_id));
                if owner.as_deref() != Some(uid) {
                    return Some(format!("a running pane is registered as it in {}", place(install, version)));
                }
            }
        }
        super::running::shared_entries(&self.home, name)
            .into_iter()
            .find(|e| e.uid != uid)
            .map(|e| format!("a running pane in channel {} is registered as it", e.channel))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::storage::store::Store;

    fn version(root: &Path, channel: &str, v: &str) -> PathBuf {
        let data = root.join("channels").join(channel).join("versions").join(v).join("data");
        std::fs::create_dir_all(data.join("db")).unwrap();
        drop(Store::open(&data.join("db").join("objects.db")).unwrap());
        data
    }

    fn exec(data: &Path, sql: &str) {
        rusqlite::Connection::open(data.join("db").join("objects.db")).unwrap().execute_batch(sql).unwrap();
    }

    #[test]
    fn every_objects_db_counts_and_the_agents_own_rows_do_not() {
        let tmp = tempfile::tempdir().unwrap();
        let home = Home::at(tmp.path());
        let a = version(tmp.path(), "ch-a", "1.0.0");
        let b = version(tmp.path(), "ch-b", "2.0.0");
        exec(&a, "INSERT INTO db_agents (id, name, provider, slug) VALUES ('uid-a', 'Claude', 'claude', 'Claude-2')");
        exec(&b, "INSERT INTO db_agents (id, name, provider, slug) VALUES ('uid-b', 'Bee', 'claude', 'bee');
                  INSERT INTO db_agent_former_slugs (agent_uid, slug, retired_at) VALUES ('uid-b', 'old-b', 1);
                  INSERT INTO db_agent_former_names (agent_id, name, recorded_at) VALUES ('uid-b', 'Was Bee', 1);");

        let host = HostSlugs::read(&home).unwrap();
        assert!(host.slug_holder("claude-2", "uid-new").unwrap().contains("uid-a"));
        assert!(host.slug_holder("claude-2", "uid-a").is_none(), "its own row");
        assert!(host.slug_holder("old-b", "uid-new").unwrap().contains("renamed"));
        assert!(host.slug_holder("was-bee", "uid-new").is_none(), "a former display name isn't a slug");
        assert!(host.holder("was-bee", "uid-new").unwrap().contains("uid-b"));
        assert!(host.holder("free", "uid-new").is_none());

        // A change to a database is seen on the next read, not the cached one.
        exec(&b, "INSERT INTO db_agents (id, name, provider, slug) VALUES ('uid-c', 'Free', 'claude', 'free')");
        assert!(HostSlugs::read(&home).unwrap().holder("free", "uid-new").is_some());
    }

    #[test]
    fn an_unreadable_objects_db_is_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let data = version(tmp.path(), "ch-a", "1.0.0");
        std::fs::write(data.join("db").join("objects.db"), b"not a database, not at all").unwrap();
        let err = HostSlugs::read(&Home::at(tmp.path())).err().expect("unreadable");
        assert!(err.contains("objects.db"), "{err}");
    }
}
