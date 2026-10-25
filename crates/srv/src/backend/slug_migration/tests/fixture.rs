// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A fixture AgentMux root: channels with versions, each with a real
//! `objects.db` (the current schema), the shared definitions, `wan.db`,
//! the host stores, and every kind of thing the old slug is stored in.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{json, Value};

use super::super::home::Home;
use crate::backend::storage::store::Store;
use crate::registry::DefinitionStore;

pub const LOAP: &str = "uid-loap";
pub const LOAP2: &str = "uid-loap-2";
pub const LOAP3: &str = "uid-loap-3";
pub const NAKI: &str = "uid-naki";
pub const NAKI2: &str = "uid-naki-2";

pub struct Fx {
    pub _tmp: tempfile::TempDir,
    pub home: Home,
    pub defs: Arc<DefinitionStore>,
}

impl Fx {
    pub fn empty() -> Fx {
        let tmp = tempfile::tempdir().unwrap();
        let home = Home::at(tmp.path());
        let defs = Arc::new(DefinitionStore::open(home.definitions()).unwrap());
        Fx { _tmp: tmp, home, defs }
    }

    pub fn root(&self) -> &Path {
        &self.home.root
    }

    /// `channels/<channel>/versions/<version>/data`, with its `objects.db`.
    pub fn version(&self, channel: &str, version: &str) -> PathBuf {
        let data = self.root().join("channels").join(channel).join("versions").join(version).join("data");
        std::fs::create_dir_all(data.join("db")).unwrap();
        drop(Store::open(&data.join("db").join("objects.db")).unwrap());
        data
    }

    pub fn store(&self, data: &Path) -> Store {
        Store::open(&data.join("db").join("objects.db")).unwrap()
    }

    /// An agent row in `data`'s `objects.db`; `mirror` also writes its shared
    /// definition, as the first channel to see an agent does.
    #[allow(clippy::too_many_arguments)]
    pub fn agent(&self, data: &Path, uid: &str, name: &str, slug: &str, parent: &str, label: &str, mirror: bool) {
        let store = self.store(data);
        if mirror {
            store.set_def_registry(self.defs.clone());
        }
        let mut def = crate::backend::storage::agents::test_agent_def(uid, name, "claude", "host", 1_000, "");
        def.slug = slug.to_string();
        def.parent_id = parent.to_string();
        def.branch_label = label.to_string();
        store.agent_def_insert(&mut def).unwrap();
        assert_eq!(store.agent_def_get(uid).unwrap().unwrap().slug, slug, "the fixture's slug is kept");
    }

    /// The three real chains of the spec's §1, and their roots, in `data`.
    pub fn chains(&self, data: &Path, mirror: bool) {
        self.agent(data, LOAP, "Loap", "loap", "", "", mirror);
        self.agent(data, LOAP2, "Loap #2", "loap-loap-2", LOAP, "Loap #2", mirror);
        self.agent(data, LOAP3, "Loap #3", "loap-loap-2-loap-3", LOAP2, "Loap #3", mirror);
        self.agent(data, NAKI, "Naki", "naki", "", "", mirror);
        self.agent(data, NAKI2, "Naki #2", "naki-naki-2", NAKI, "Naki #2", mirror);
    }

    pub fn exec(&self, db: &Path, sql: &str) {
        rusqlite::Connection::open(db).unwrap().execute_batch(sql).unwrap();
    }

    /// Name-keyed key rows under `name` in `data`'s `objects.db`.
    pub fn keys(&self, data: &Path, name: &str) {
        self.exec(
            &data.join("db").join("objects.db"),
            &format!(
                "INSERT INTO db_agent_jekt_keys (agent_id, hmac_key, created_at) VALUES ('{name}', 'hmac-{name}', 1);
                 INSERT INTO db_agent_lan_keys (agent_id, public_key, private_key, created_at) VALUES ('{name}', 'lpub', 'lpriv', 1);
                 INSERT INTO db_agent_wan_keys (agent_id, public_key, private_key, key_version, created_at) VALUES ('{name}', 'wpub', 'wpriv', 1, 1);"
            ),
        );
    }

    /// A WAN key under `name` in `channel`'s `wan.db`, published once.
    pub fn wan_key(&self, channel: &str, name: &str) {
        let path = self.root().join("channels").join(channel).join("wan-identity").join("wan.db");
        let wan = crate::backend::storage::wan_identity::WanIdentityStore::open(&path).unwrap();
        let key = wan.agent_key_ensure(name, None).unwrap();
        assert!(wan.agent_key_mark_published(name, &key.public_key, "relay|account").unwrap());
    }

    /// A pane of `uid` whose persisted `cmd:env` names `slug`, working in
    /// `cwd`, and a layout snapshot holding the same.
    pub fn pane(&self, data: &Path, uid: &str, slug: &str, oid: &str, cwd: &Path) {
        let store = self.store(data);
        let meta = json!({
            "view": "agent",
            "agentId": uid,
            "cmd:cwd": cwd.display().to_string(),
            "cmd:env": {
                "AGENTMUX_AGENT_ID": slug,
                "AGENTMUX_AGENT_SLUG": slug,
                "GIT_AUTHOR_EMAIL": format!("{slug}@agentmux.local"),
                "GIT_AUTHOR_NAME": slug,
                "API_TOKEN": "keep-me",
            },
        });
        let mut block = crate::backend::obj::Block {
            oid: oid.to_string(),
            meta: meta.as_object().unwrap().clone().into_iter().collect(),
            ..Default::default()
        };
        store.insert(&mut block).unwrap();
        let client = json!({
            "oid": format!("client-{oid}"),
            "otype": "client",
            "meta": { "session:last_topology": { "windows": [{ "tabs": [{ "blocks": [{ "oid": oid, "meta": meta }] }] }] } },
        });
        let conn = store.conn().lock().unwrap();
        conn.execute(
            "INSERT INTO db_client (oid, version, data) VALUES (?1, 1, ?2)",
            rusqlite::params![format!("client-{oid}"), serde_json::to_vec(&client).unwrap()],
        )
        .unwrap();
    }

    /// A workspace whose `.mcp.json` signs as `slug`, with a key and a server
    /// of the user's own.
    pub fn workspace(&self, name: &str, slug: &str) -> PathBuf {
        let dir = self.root().join("ws").join(name);
        std::fs::create_dir_all(&dir).unwrap();
        let mcp = json!({
            "mcpServers": {
                "agentmux": { "command": "agentmux-mcp", "env": { "AGENTMUX_AGENT_ID": slug, "AGENTMUX_JEKT_KEY": "secret" } },
                "mine": { "command": "my-server" },
            }
        });
        std::fs::write(dir.join(".mcp.json"), serde_json::to_vec_pretty(&mcp).unwrap()).unwrap();
        dir
    }

    /// A registration left behind by an srv that is gone, per version and in
    /// the cross-channel registry.
    pub fn stale_registration(&self, data: &Path, channel: &str, name: &str) {
        let entry = json!({ "agent_id": name, "local_url": "http://127.0.0.1:1", "block_id": "b-gone", "pid": u32::MAX, "updated_at": 1, "channel": channel });
        let bytes = serde_json::to_vec(&entry).unwrap();
        std::fs::create_dir_all(data.join("agents")).unwrap();
        std::fs::write(data.join("agents").join(format!("{name}.json")), &bytes).unwrap();
        let shared = self.home.reactive().join(name);
        std::fs::create_dir_all(&shared).unwrap();
        std::fs::write(shared.join(format!("{channel}.json")), &bytes).unwrap();
    }

    /// The host stores, with a cron job, a job, a queue item and a MuxBus credential
    /// that name `slug` and carry no uid.
    pub fn host_rows(&self, slug: &str) {
        let shared = &self.home.shared;
        drop(Store::open_shared(&shared.join("store.db")).unwrap());
        drop(Store::open_identity_store(&shared.join("identity-store.db")).unwrap());
        self.exec(
            &shared.join("store.db"),
            &format!(
                "INSERT INTO db_cron_jobs (id, name, expression, prompt, target, target_uid) VALUES ('cron-1', 'nightly', '0 0 * * *', 'hi', '{slug}', '');
                 INSERT INTO db_cron_jobs (id, name, expression, prompt, target, target_uid) VALUES ('cron-2', 'other', '0 0 * * *', 'hi', 'naki', '');
                 INSERT INTO db_jobs (id, name, expression, prompt, target, target_uid) VALUES ('job-1', 'nightly', '0 0 * * *', 'hi', '{slug}', '');
                 INSERT INTO db_jobs (id, name, expression, prompt, target, target_uid) VALUES ('job-2', 'other', '0 0 * * *', 'hi', 'naki', '');
                 INSERT INTO db_agent_credentials (agent_id, client_id) VALUES ('{slug}', 'cid');"
            ),
        );
        self.exec(
            &shared.join("identity-store.db"),
            &format!("INSERT INTO db_work_queue (id, title, payload, target_agent, state) VALUES ('w-1', 't', 'p', '{slug}', 'open');"),
        );
    }

    /// The full fixture of the spec's §6: two channels with two versions
    /// each, the three chains, and every location type for `loap-loap-2`.
    pub fn standard() -> Fx {
        let fx = Fx::empty();
        let stable_a = fx.version("stable", "0.59.1");
        let stable_b = fx.version("stable", "0.59.2");
        let local_a = fx.version("local-x", "0.60.0");
        let local_b = fx.version("local-x", "0.60.1");
        fx.chains(&stable_a, true);
        for data in [&stable_b, &local_a, &local_b] {
            fx.chains(data, false);
        }
        for data in [&stable_a, &local_b] {
            fx.keys(data, "loap-loap-2");
            fx.keys(data, "naki");
        }
        fx.wan_key("stable", "loap-loap-2");
        fx.wan_key("local-x", "naki");
        let ws = fx.workspace("loap-2", "loap-loap-2");
        fx.pane(&stable_b, LOAP2, "loap-loap-2", "block-loap2", &ws);
        fx.exec(
            &stable_a.join("db").join("objects.db"),
            "INSERT INTO db_conversation_trust_grants (agent_id, granted_peer_agent_id, tier, granted_at) VALUES ('naki', 'loap-loap-2', 'host', 5);",
        );
        fx.host_rows("loap-loap-2");
        let gh = fx.root().join("channels").join("stable").join("config").join("gh-loap-loap-2");
        std::fs::create_dir_all(&gh).unwrap();
        std::fs::write(gh.join("config.yml"), "git_protocol: https\n").unwrap();
        std::fs::create_dir_all(&fx.home.bashwrap_dir).unwrap();
        std::fs::write(fx.home.bashwrap_dir.join("loap-loap-2.cwd"), "/work\n").unwrap();
        fx.stale_registration(&stable_a, "stable", "loap-loap-2");
        fx
    }

    /// Every file and folder under the root, each with a digest: a
    /// database's rows (`backup::db_digest`), a file's bytes. The run's own
    /// backups and locks are left out.
    pub fn snapshot(&self) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        walk(self.root(), self.root(), &mut out);
        out
    }
}

fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, String>) {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.path()).collect();
    entries.sort();
    for path in entries {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let rel = path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
        if path.is_dir() {
            if matches!(name.as_str(), "backups" | "locks") {
                continue;
            }
            out.insert(rel, "dir".into());
            walk(root, &path, out);
        } else if name.ends_with(".db") {
            out.insert(rel, super::super::backup::db_digest(&path).unwrap());
        } else if !(name.ends_with("-wal") || name.ends_with("-shm") || name.ends_with(".lock")) {
            out.insert(rel, super::super::backup::file_sha256(&path).unwrap());
        }
    }
}

/// The JSON at `path`.
pub fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

/// One value from `db`.
pub fn query<T: rusqlite::types::FromSql>(db: &Path, sql: &str) -> Option<T> {
    use rusqlite::OptionalExtension;
    rusqlite::Connection::open(db).unwrap().query_row(sql, [], |r| r.get(0)).optional().unwrap()
}
