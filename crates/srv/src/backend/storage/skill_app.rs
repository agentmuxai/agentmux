// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What makes a catalog skill a computer-use skill (docs/specs/SPEC_PACKS_2026_11_05.md §3, §7): its
//! manifest (`computer-use.json`), the files that sit beside its `SKILL.md`, and the records of tests
//! that passed. They live in `db_skill_app`, `db_skill_file` and `db_skill_verified`, in the identity
//! store beside `db_skills`, and are created on first use without a schema bump (like
//! `db_skill_source`). A skill written by hand has no `db_skill_app` row.
//!
//! The skill's `SKILL.md` body stays in `db_skills.content`, so every existing path that lists,
//! binds, reaches or exports a skill keeps working. The pass records are not part of what a signature
//! covers, so a new pass never breaks one (spec §7).

use std::collections::BTreeMap;

use rusqlite::{params, OptionalExtension};

use super::error::StoreError;
use super::store::Store;
use crate::backend::memory::record::sha256_hex;

const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS db_skill_app (
        skill_id     TEXT PRIMARY KEY,
        publisher    TEXT NOT NULL,
        app_id       TEXT NOT NULL,
        version      TEXT NOT NULL,
        manifest     TEXT NOT NULL,
        content_hash TEXT NOT NULL DEFAULT '',
        installed_at INTEGER NOT NULL DEFAULT 0
    );
    CREATE TABLE IF NOT EXISTS db_skill_file (
        skill_id TEXT NOT NULL,
        path     TEXT NOT NULL,
        content  TEXT NOT NULL,
        image    BLOB,
        sha256   TEXT NOT NULL,
        script   INTEGER NOT NULL DEFAULT 0,
        approved INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY (skill_id, path)
    );
    CREATE TABLE IF NOT EXISTS db_skill_signature (
        skill_id   TEXT PRIMARY KEY,
        signer_key TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS db_skill_origin (
        skill_id  TEXT PRIMARY KEY,
        skill_md  TEXT NOT NULL,
        signature TEXT
    );
    CREATE TABLE IF NOT EXISTS db_skill_publisher_pin (
        publisher  TEXT PRIMARY KEY,
        public_key TEXT NOT NULL,
        pinned_at  INTEGER NOT NULL DEFAULT 0
    );
    CREATE TABLE IF NOT EXISTS db_skill_verified (
        skill_id       TEXT NOT NULL,
        skill_version  TEXT NOT NULL,
        app_version    TEXT NOT NULL,
        platform       TEXT NOT NULL,
        date           TEXT NOT NULL,
        runner         TEXT NOT NULL,
        route          TEXT NOT NULL DEFAULT '',
        result         TEXT NOT NULL,
        notes          TEXT NOT NULL DEFAULT '',
        evidence_sha256 TEXT NOT NULL DEFAULT '',
        added_by       TEXT NOT NULL DEFAULT '',
        added_at       INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY (skill_id, skill_version, app_version, platform, date, runner, result, evidence_sha256)
    );";

/// One rung of a skill's ladder, best first.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct LadderStep {
    /// `api`, `mcp`, `shortcuts` or `desktop`.
    pub kind: String,
    pub note: String,
}

/// A test that ran against an app, as the skill's `verification` record states it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct VerifiedRecord {
    pub skill_version: String,
    pub app_version: String,
    /// `windows`, `macos` or `linux`.
    pub platform: String,
    /// `YYYY-MM-DD`.
    pub date: String,
    /// `AgentMux test machine` or `community:<handle>`; never a host or user name.
    pub runner: String,
    /// Which rung of the ladder the test used; empty when it did not say.
    #[serde(default)]
    pub route: String,
    /// `pass` or `fail`.
    pub result: String,
    #[serde(default)]
    pub notes: String,
    /// SHA-256 of the output file or screenshot; the evidence itself is never kept.
    #[serde(default)]
    pub evidence_sha256: String,
}

/// A file beside `SKILL.md`, as the card lists it (its text is read with `Skills get`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct SkillFileInfo {
    pub path: String,
    /// SHA-256 of the file as stored: what an approval is for.
    pub sha256: String,
    #[ts(type = "number")]
    pub bytes: u64,
    /// A script the agent's shell would run: not written to an agent's folder until a person approves it.
    pub script: bool,
    pub approved: bool,
    /// A reference crop (a PNG under `references/`), read as an image.
    pub image: bool,
}

/// One reference crop of the app's UI (SPEC_PACKS §3.1), as the manifest tags it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct ReferenceInfo {
    pub path: String,
    /// What the crop is of: "master fader".
    pub label: String,
    /// One line on what to look for.
    #[serde(default)]
    pub shows: String,
    pub app_version: String,
    pub platform: String,
    /// `light`, `dark`, or the app's own skin name.
    pub theme: String,
    /// Display scaling in percent.
    pub scale: u32,
    /// `AgentMux test machine`; never a host or user name.
    pub captured_by: String,
    pub date: String,
    pub sha256: String,
}

/// The manifest's own fields, in the form Memory and `Desktop` read them.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct SkillAppInfo {
    pub publisher: String,
    pub id: String,
    pub name: String,
    pub version: String,
    pub vendor: String,
    /// Executable names per platform (`windows`, `macos`, `linux`).
    pub executables: BTreeMap<String, Vec<String>>,
    pub title_hints: Vec<String>,
    pub platforms: Vec<String>,
    pub ladder: Vec<LadderStep>,
    /// The id of the MCP catalog entry the skill points at, never a command.
    pub mcp: Option<String>,
    /// A semver range of app versions the skill claims; `None` while unknown.
    pub supported: Option<String>,
    /// App versions a test passed on, as the manifest says.
    pub tested: Vec<String>,
    /// Reference crops, each tagged with the app version it was taken at.
    pub references: Vec<ReferenceInfo>,
}

/// What a person asks of a script: revoke it, or approve the text with this hash.
#[derive(Debug, Clone, Copy)]
pub enum ScriptApproval<'a> {
    Revoke,
    Approve { reviewed_sha256: &'a str },
}

/// What `skill_script_set_approved` did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptOutcome {
    Done,
    NotAScript,
    /// The script is not the text that was reviewed: it was replaced since.
    Changed,
    /// The publisher is pinned to another key than this skill's signer.
    OtherAuthor,
}

/// What a script's manifest entry says about it, for the prompt that asks for its approval.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct ScriptInfo {
    pub path: String,
    /// What it is for, in the publisher's words.
    pub purpose: String,
    /// What it needs to run ("python 3", "network").
    pub needs: Vec<String>,
}

/// A computer-use skill as the catalog holds it: the manifest, the files beside `SKILL.md`, and the pass records.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct SkillApp {
    #[serde(flatten)]
    pub info: SkillAppInfo,
    /// The manifest's scripts, with what each is for and needs.
    pub scripts: Vec<ScriptInfo>,
    /// Who signed it, and whether that is the key its publisher is pinned to.
    pub signature: crate::backend::widget::signature::WidgetSignatureInfo,
    pub files: Vec<SkillFileInfo>,
    pub verified: Vec<VerifiedRecord>,
    /// The SHA-256 the publisher's signature covers (empty for a skill that came without one).
    pub content_hash: String,
}

/// A file ready to store: its text and whether it is a script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillFileInput {
    pub path: String,
    /// The text; empty for an image.
    pub content: String,
    /// The PNG bytes of a reference crop; `None` for a text file.
    pub image: Option<Vec<u8>>,
    pub script: bool,
}

/// A file of a skill with its text, for the launch step and `Skills get`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillFile {
    pub path: String,
    pub content: String,
    pub image: Option<Vec<u8>>,
    pub script: bool,
    pub approved: bool,
}

fn pins_of(conn: &rusqlite::Connection) -> Result<crate::backend::widget::signature::Pins, StoreError> {
    let mut stmt = conn.prepare("SELECT publisher, public_key FROM db_skill_publisher_pin")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

impl Store {
    /// Make `skill_id` a computer-use skill, replacing any earlier manifest and files in one step.
    /// A script that was approved with the same text stays approved; a changed or new one needs a new
    /// approval. Call on the catalog store.
    pub fn skill_app_set(
        &self,
        skill_id: &str,
        info: &SkillAppInfo,
        manifest_json: &str,
        content_hash: &str,
        files: &[SkillFileInput],
        now_ms: i64,
    ) -> Result<(), StoreError> {
        let mut conn = self.conn.lock().unwrap();
        conn.execute_batch(SCHEMA)?;
        let tx = conn.transaction()?;
        // (path, sha256) of scripts already approved: kept only when the text is the same.
        let approved: std::collections::HashSet<(String, String)> = {
            let mut stmt = tx.prepare("SELECT path, sha256 FROM db_skill_file WHERE skill_id = ?1 AND script = 1 AND approved = 1")?;
            let rows = stmt.query_map(params![skill_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            rows.collect::<Result<_, _>>()?
        };
        tx.execute(
            "INSERT INTO db_skill_app (skill_id, publisher, app_id, version, manifest, content_hash, installed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(skill_id) DO UPDATE SET publisher = excluded.publisher, app_id = excluded.app_id,
                version = excluded.version, manifest = excluded.manifest, content_hash = excluded.content_hash,
                installed_at = excluded.installed_at",
            params![skill_id, info.publisher, info.id, info.version, manifest_json, content_hash, now_ms],
        )?;
        tx.execute("DELETE FROM db_skill_file WHERE skill_id = ?1", params![skill_id])?;
        for f in files {
            let sha = sha256_hex(f.image.as_deref().unwrap_or(f.content.as_bytes()));
            let keep = f.script && approved.contains(&(f.path.clone(), sha.clone()));
            tx.execute(
                "INSERT INTO db_skill_file (skill_id, path, content, image, sha256, script, approved) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![skill_id, f.path, f.content, f.image, sha, f.script as i64, keep as i64],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// The computer-use fields of one skill, if it is one.
    pub fn skill_app_get(&self, skill_id: &str) -> Result<Option<SkillApp>, StoreError> {
        let mut map = self.skill_app_query(Some(skill_id))?;
        Ok(map.remove(skill_id))
    }

    /// Every computer-use skill, by skill id.
    pub fn skill_app_map(&self) -> Result<BTreeMap<String, SkillApp>, StoreError> {
        self.skill_app_query(None)
    }

    fn skill_app_query(&self, only: Option<&str>) -> Result<BTreeMap<String, SkillApp>, StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(SCHEMA)?;
        let mut out = BTreeMap::new();
        let pins = pins_of(&conn)?;
        {
            let mut stmt = conn.prepare("SELECT skill_id, manifest, content_hash FROM db_skill_app WHERE (?1 IS NULL OR skill_id = ?1)")?;
            let rows = stmt.query_map(params![only], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;
            for row in rows {
                let (id, manifest, content_hash) = row?;
                // A manifest this build can't read is left out rather than failing the whole list.
                if let Ok(info) = crate::backend::computer_use_skill::info_from_manifest(&manifest) {
                    let scripts = crate::backend::computer_use_skill::script_infos(&manifest);
                    let signature = crate::backend::widget::signature::describe(&format!("{}.{}", info.publisher, info.id), None, &pins);
                    out.insert(id, SkillApp { info, scripts, signature, files: Vec::new(), verified: Vec::new(), content_hash });
                }
            }
        }
        {
            let mut stmt = conn.prepare("SELECT skill_id, signer_key FROM db_skill_signature WHERE (?1 IS NULL OR skill_id = ?1)")?;
            let rows = stmt.query_map(params![only], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            for row in rows {
                let (id, key) = row?;
                if let Some(app) = out.get_mut(&id) {
                    app.signature = crate::backend::widget::signature::describe(&format!("{}.{}", app.info.publisher, app.info.id), Some(&key), &pins);
                }
            }
        }
        {
            let mut stmt = conn.prepare(
                "SELECT skill_id, path, length(CAST(content AS BLOB)) + COALESCE(length(image), 0), script, approved, image IS NOT NULL, sha256 FROM db_skill_file
                 WHERE (?1 IS NULL OR skill_id = ?1) ORDER BY skill_id, path",
            )?;
            let rows = stmt.query_map(params![only], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    SkillFileInfo {
                        path: r.get(1)?,
                        bytes: r.get::<_, i64>(2)? as u64,
                        script: r.get::<_, i64>(3)? != 0,
                        approved: r.get::<_, i64>(4)? != 0,
                        image: r.get::<_, i64>(5)? != 0,
                        sha256: r.get(6)?,
                    },
                ))
            })?;
            for row in rows {
                let (id, file) = row?;
                if let Some(app) = out.get_mut(&id) {
                    app.files.push(file);
                }
            }
        }
        {
            let mut stmt = conn.prepare(
                "SELECT skill_id, skill_version, app_version, platform, date, runner, route, result, notes, evidence_sha256
                 FROM db_skill_verified WHERE (?1 IS NULL OR skill_id = ?1) ORDER BY skill_id, date, rowid",
            )?;
            let rows = stmt.query_map(params![only], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    VerifiedRecord {
                        skill_version: r.get(1)?,
                        app_version: r.get(2)?,
                        platform: r.get(3)?,
                        date: r.get(4)?,
                        runner: r.get(5)?,
                        route: r.get(6)?,
                        result: r.get(7)?,
                        notes: r.get(8)?,
                        evidence_sha256: r.get(9)?,
                    },
                ))
            })?;
            for row in rows {
                let (id, rec) = row?;
                if let Some(app) = out.get_mut(&id) {
                    app.verified.push(rec);
                }
            }
        }
        Ok(out)
    }

    /// The files beside `SKILL.md`, with their text.
    pub fn skill_files(&self, skill_id: &str) -> Result<Vec<SkillFile>, StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(SCHEMA)?;
        let mut stmt = conn.prepare("SELECT path, content, image, script, approved FROM db_skill_file WHERE skill_id = ?1 ORDER BY path")?;
        let rows = stmt.query_map(params![skill_id], |r| {
            Ok(SkillFile { path: r.get(0)?, content: r.get(1)?, image: r.get(2)?, script: r.get::<_, i64>(3)? != 0, approved: r.get::<_, i64>(4)? != 0 })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// One file beside `SKILL.md`.
    pub fn skill_file_get(&self, skill_id: &str, path: &str) -> Result<Option<SkillFile>, StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(SCHEMA)?;
        Ok(conn
            .query_row(
                "SELECT path, content, image, script, approved FROM db_skill_file WHERE skill_id = ?1 AND path = ?2",
                params![skill_id, path],
                |r| Ok(SkillFile { path: r.get(0)?, content: r.get(1)?, image: r.get(2)?, script: r.get::<_, i64>(3)? != 0, approved: r.get::<_, i64>(4)? != 0 }),
            )
            .optional()?)
    }

    /// Add a test record. The same record twice is kept once. Records are only ever added.
    /// Returns whether a new one was stored. `added_by` is the agent or window that filed it.
    pub fn skill_verified_append(&self, skill_id: &str, rec: &VerifiedRecord, added_by: &str, now_ms: i64) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(SCHEMA)?;
        let n = conn.execute(
            "INSERT OR IGNORE INTO db_skill_verified
                (skill_id, skill_version, app_version, platform, date, runner, route, result, notes, evidence_sha256, added_by, added_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                skill_id,
                rec.skill_version,
                rec.app_version,
                rec.platform,
                rec.date,
                rec.runner,
                rec.route,
                rec.result,
                rec.notes,
                rec.evidence_sha256,
                added_by,
                now_ms
            ],
        )?;
        Ok(n > 0)
    }

    /// Everything the catalog keeps beside a deleted skill's row: reach, source, computer-use files and records, opt-outs.
    pub fn skill_catalog_forget(&self, skill_id: &str) -> Result<(), StoreError> {
        self.skill_reach_forget(skill_id)?;
        self.skill_source_forget(skill_id)?;
        self.skill_app_forget(skill_id)?;
        self.skill_optout_forget(skill_id)
    }

    /// Record which key verified a skill's signature at install; `None` for a skill that came unsigned.
    pub fn skill_signer_set(&self, skill_id: &str, signer: Option<&str>) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(SCHEMA)?;
        match signer {
            Some(key) => conn.execute(
                "INSERT INTO db_skill_signature (skill_id, signer_key) VALUES (?1, ?2) ON CONFLICT(skill_id) DO UPDATE SET signer_key = excluded.signer_key",
                params![skill_id, key],
            )?,
            None => conn.execute("DELETE FROM db_skill_signature WHERE skill_id = ?1", params![skill_id])?,
        };
        Ok(())
    }

    /// The manifest as the skill was installed with it.
    pub fn skill_manifest_get(&self, skill_id: &str) -> Result<Option<String>, StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(SCHEMA)?;
        Ok(conn.query_row("SELECT manifest FROM db_skill_app WHERE skill_id = ?1", params![skill_id], |r| r.get(0)).optional()?)
    }

    /// Keep the `SKILL.md` and `skill.sig` a skill was installed from, as written. The content hash covers `SKILL.md`, so
    /// an export can only carry the publisher's signature if it writes those exact bytes back.
    pub fn skill_origin_set(&self, skill_id: &str, skill_md: &str, signature: Option<&str>) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(SCHEMA)?;
        conn.execute(
            "INSERT INTO db_skill_origin (skill_id, skill_md, signature) VALUES (?1, ?2, ?3)
             ON CONFLICT(skill_id) DO UPDATE SET skill_md = excluded.skill_md, signature = excluded.signature",
            params![skill_id, skill_md, signature],
        )?;
        Ok(())
    }

    /// The `SKILL.md` and `skill.sig` the skill was installed from; `None` for one installed before they were kept.
    pub fn skill_origin_get(&self, skill_id: &str) -> Result<Option<(String, Option<String>)>, StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(SCHEMA)?;
        Ok(conn
            .query_row("SELECT skill_md, signature FROM db_skill_origin WHERE skill_id = ?1", params![skill_id], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?)
    }

    /// Publisher → the key it is pinned to (the first key a person approved a script of its for).
    pub fn skill_publisher_pins(&self) -> Result<crate::backend::widget::signature::Pins, StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(SCHEMA)?;
        pins_of(&conn)
    }

    /// Approve or revoke one script. Approving needs the SHA-256 the person reviewed and applies only if the stored
    /// script still has it, and not at all when the skill's publisher is pinned to another key than the one that
    /// signed this skill (or it is unsigned): that is a different author, not an update. A first approval of a
    /// signed skill pins its publisher to the signer's key.
    pub fn skill_script_set_approved(&self, skill_id: &str, path: &str, approval: ScriptApproval<'_>, now_ms: i64) -> Result<ScriptOutcome, StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(SCHEMA)?;
        let stored: Option<String> = conn
            .query_row("SELECT sha256 FROM db_skill_file WHERE skill_id = ?1 AND path = ?2 AND script = 1", params![skill_id, path], |r| r.get(0))
            .optional()?;
        let Some(stored) = stored else { return Ok(ScriptOutcome::NotAScript) };
        let ScriptApproval::Approve { reviewed_sha256 } = approval else {
            conn.execute("UPDATE db_skill_file SET approved = 0 WHERE skill_id = ?1 AND path = ?2", params![skill_id, path])?;
            return Ok(ScriptOutcome::Done);
        };
        if stored != reviewed_sha256 {
            return Ok(ScriptOutcome::Changed);
        }
        let (publisher, signer): (Option<String>, Option<String>) = conn
            .query_row(
                "SELECT a.publisher, (SELECT signer_key FROM db_skill_signature WHERE skill_id = a.skill_id)
                 FROM db_skill_app a WHERE a.skill_id = ?1",
                params![skill_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .unwrap_or((None, None));
        if let Some(publisher) = &publisher {
            let pinned: Option<String> = conn.query_row("SELECT public_key FROM db_skill_publisher_pin WHERE publisher = ?1", params![publisher], |r| r.get(0)).optional()?;
            if pinned.is_some() && pinned != signer {
                return Ok(ScriptOutcome::OtherAuthor);
            }
        }
        conn.execute("UPDATE db_skill_file SET approved = 1 WHERE skill_id = ?1 AND path = ?2", params![skill_id, path])?;
        conn.execute(
            "INSERT OR IGNORE INTO db_skill_publisher_pin (publisher, public_key, pinned_at)
             SELECT a.publisher, s.signer_key, ?2 FROM db_skill_app a JOIN db_skill_signature s ON s.skill_id = a.skill_id WHERE a.skill_id = ?1",
            params![skill_id, now_ms],
        )?;
        Ok(ScriptOutcome::Done)
    }

    /// Drop a deleted skill's manifest, files and records. Call on the catalog store.
    pub fn skill_app_forget(&self, skill_id: &str) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(SCHEMA)?;
        conn.execute("DELETE FROM db_skill_signature WHERE skill_id = ?1", params![skill_id])?;
        conn.execute("DELETE FROM db_skill_origin WHERE skill_id = ?1", params![skill_id])?;
        conn.execute("DELETE FROM db_skill_app WHERE skill_id = ?1", params![skill_id])?;
        conn.execute("DELETE FROM db_skill_file WHERE skill_id = ?1", params![skill_id])?;
        conn.execute("DELETE FROM db_skill_verified WHERE skill_id = ?1", params![skill_id])?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "skill_app_tests.rs"]
mod tests;
