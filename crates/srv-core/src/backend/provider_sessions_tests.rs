// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use serde_json::json;

use super::*;
use crate::backend::continuity_segments::{record_end, record_session, record_start, EndReason, Rung, Start};

const UID: &str = "uid-prov";
const DEF: &str = "def-prov";
const SID1: &str = "aaaaaaaa-1111-2222-3333-444444444444";
const SID2: &str = "bbbbbbbb-1111-2222-3333-444444444444";

fn now() -> i64 {
    agentmux_common::time::now_ms()
}

struct Env {
    dir: tempfile::TempDir,
    fs: FileStore,
    roots: Roots,
    config: String,
    cwd: String,
}

impl Env {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let shared = dir.path().join("shared");
        let config = shared.join("providers").join("claude");
        std::fs::create_dir_all(&config).unwrap();
        let cwd = dir.path().join("work").to_string_lossy().into_owned();
        let roots = Roots { shared: Some(shared), agentmux_home: Some(dir.path().to_path_buf()) };
        Self { dir, fs: FileStore::open_in_memory().unwrap(), roots, config: config.to_string_lossy().into_owned(), cwd }
    }

    fn start(&self, at: i64, sid: &str, config: &str) -> String {
        let s = Start {
            segment_id: String::new(),
            agent_uid: UID.to_string(),
            definition_id: Some(DEF.to_string()),
            provider: "claude".to_string(),
            config_dir: Some(config.to_string()),
            provider_session_id: None,
            cwd: self.cwd.clone(),
            channel: "c".to_string(),
            agentmux_version: "v".to_string(),
            block_id: "b".to_string(),
            zone: Some(format!("agent:{DEF}:current")),
            byte_start: None,
            started_at_ms: at,
            continuity_rung: Rung::Native,
            predecessor_segment_id: None,
            lease_epoch: None,
            identity_key: None,
            forked_from: None,
        };
        let id = record_start(&self.fs, s).unwrap();
        record_session(&self.fs, UID, &id, sid, at).unwrap();
        id
    }

    fn end(&self, seg: &str, at: i64) {
        record_end(&self.fs, UID, seg, EndReason::Stopped, None, at).unwrap();
    }

    fn file(&self, sid: &str) -> std::path::PathBuf {
        crate::backend::session_backfill::session_file_path(&self.config, &self.cwd, sid).unwrap()
    }

    fn put(&self, sid: &str, text: &str) {
        let p = self.file(sid);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        let line = json!({ "type": "user", "sessionId": sid, "uuid": "u", "message": { "role": "user", "content": text } });
        std::fs::write(p, format!("{line}\n")).unwrap();
    }

    fn store(&self, zone: &str, sid: &str, text: &str) {
        let line = json!({ "type": "user", "session_id": sid, "message": { "role": "user", "content": text } });
        FsZone { fs: &self.fs, zone }.append("output", format!("{line}\n").as_bytes()).unwrap();
    }

    fn ended(&self, policy: &Policy, at: i64) -> Vec<Ended> {
        ended_sessions(UID, &segs::segments(&self.fs, UID), &self.roots, policy, at)
    }
}

fn on(days: u32) -> Policy {
    Policy { enabled: true, idle_days: days }
}

fn later() -> i64 {
    now() + 40 * DAY_MS
}

#[test]
fn a_session_the_agent_left_ends_and_the_one_it_is_in_stays() {
    let e = Env::new();
    let a = e.start(1_000, SID1, &e.config);
    e.end(&a, 2_000);
    e.start(3_000, SID2, &e.config);
    e.put(SID1, "old");
    e.put(SID2, "current");
    let ended = e.ended(&on(7), later());
    assert_eq!(ended.len(), 1);
    assert_eq!((ended[0].sid.as_str(), ended[0].reason), (SID1, Reason::Replaced));
}

#[test]
fn the_newest_session_ends_only_when_nothing_holds_it_and_it_has_been_idle_long_enough() {
    let e = Env::new();
    let a = e.start(1_000, SID1, &e.config);
    e.put(SID1, "idle");
    assert!(e.ended(&on(7), later()).is_empty(), "a running process holds it");
    e.end(&a, 2_000);
    assert_eq!(e.ended(&on(7), later()).iter().map(|x| x.reason).collect::<Vec<_>>(), [Reason::Idle]);
    assert!(e.ended(&on(30), now() + 10 * DAY_MS).is_empty(), "not idle long enough for 30 days");
}

#[test]
fn a_file_written_in_the_last_hour_is_never_touched() {
    let e = Env::new();
    let a = e.start(1_000, SID1, &e.config);
    e.end(&a, 2_000);
    let b = e.start(3_000, SID2, &e.config);
    e.end(&b, 4_000);
    e.put(SID1, "just now");
    assert!(e.ended(&on(7), now()).is_empty());
}

#[test]
fn a_session_in_a_home_agentmux_does_not_own_is_not_looked_at() {
    let e = Env::new();
    let own = e.dir.path().join("home").join(".claude").to_string_lossy().into_owned();
    let a = e.start(1_000, SID1, &own);
    e.end(&a, 2_000);
    e.start(3_000, SID2, &e.config);
    let p = crate::backend::session_backfill::session_file_path(&own, &e.cwd, SID1).unwrap();
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, "a person's own session\n").unwrap();
    assert!(e.ended(&on(7), later()).is_empty());
    assert!(p.exists());
}

#[test]
fn the_policy_reads_its_settings_with_the_limits() {
    let m = |k: &str, v: serde_json::Value| HashMap::from([(k.to_string(), v)]);
    assert_eq!(Policy::from_extra(&HashMap::new()), Policy { enabled: false, idle_days: 7 });
    assert_eq!(Policy::from_extra(&m(KEY_IDLE_DAYS, json!(0))).idle_days, 1);
    assert_eq!(Policy::from_extra(&m(KEY_IDLE_DAYS, json!(99))).idle_days, 30);
    assert_eq!(Policy::from_extra(&m(KEY_IDLE_DAYS, json!(3.9))).idle_days, 3);
    assert_eq!(Policy::from_extra(&m(KEY_IDLE_DAYS, json!("soon"))).idle_days, 7);
    assert!(Policy::from_extra(&m(KEY_SWEEP, json!(true))).enabled);
}

#[test]
fn a_sweep_that_is_not_enabled_removes_nothing() {
    let e = Env::new();
    let a = e.start(1_000, SID1, &e.config);
    e.end(&a, 2_000);
    e.start(3_000, SID2, &e.config);
    e.put(SID1, "old");
    let off = Policy { enabled: false, idle_days: 7 };
    assert_eq!(sweep(&e.fs, &e.roots, &off, later()), Report::default());
    assert!(e.file(SID1).exists());
}

#[test]
fn a_sweep_reads_only_the_store_it_is_given_and_none_is_swept_without_a_global_store() {
    let e = Env::new();
    let a = e.start(1_000, SID1, &e.config);
    e.end(&a, 2_000);
    e.start(3_000, SID2, &e.config);
    e.put(SID1, "old");
    // The per-channel store holds none of the segment logs: a sweep over it finds nothing to end.
    let channel = FileStore::open_in_memory().unwrap();
    assert_eq!(sweep(&channel, &e.roots, &on(7), later()), Report { agents: 0, ..Report::default() });
    assert!(e.file(SID1).exists());
    // No global store is installed in a unit test: the production entry point sweeps nothing.
    assert_eq!(sweep_global(&e.roots, &on(7), later()), None);
}

#[test]
fn a_sweep_removes_the_session_the_store_holds_and_keeps_the_current_one() {
    let e = Env::new();
    let a = e.start(1_000, SID1, &e.config);
    e.end(&a, 2_000);
    e.start(3_000, SID2, &e.config);
    e.put(SID1, "kept in the store");
    e.put(SID2, "current");
    e.store(&format!("agent:{DEF}:current"), SID1, "kept in the store");
    let r = sweep(&e.fs, &e.roots, &on(7), later());
    assert_eq!((r.agents, r.ended, r.removed_files, r.imported_records, r.failed), (1, 1, 1, 0, 0));
    assert!(!e.file(SID1).exists() && e.file(SID2).exists());
}

#[test]
fn a_conversation_that_was_archived_still_proves_the_session() {
    let e = Env::new();
    let a = e.start(1_000, SID1, &e.config);
    e.end(&a, 2_000);
    e.start(3_000, SID2, &e.config);
    e.put(SID1, "said before the archive");
    e.store(&format!("agent:{DEF}:archive:100"), SID1, "said before the archive");
    let r = sweep(&e.fs, &e.roots, &on(7), later());
    assert_eq!((r.removed_files, r.imported_records), (1, 0));
}

#[test]
fn what_the_store_lacks_is_imported_into_the_agents_own_zone_before_the_file_goes() {
    let e = Env::new();
    let a = e.start(1_000, SID1, &e.config);
    e.end(&a, 2_000);
    e.start(3_000, SID2, &e.config);
    e.put(SID1, "only the provider has this");
    let r = sweep(&e.fs, &e.roots, &on(7), later());
    assert_eq!((r.removed_files, r.imported_records), (1, 1));
    let zone = provider_zone(UID).unwrap();
    let imported = FsZone { fs: &e.fs, zone: &zone }.read_all(&format!("import/{SID1}/{SID1}.jsonl")).unwrap();
    assert!(String::from_utf8(imported).unwrap().contains("only the provider has this"));
}
