// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::path::Path;

use serde_json::{json, Value};

use super::*;
use crate::backend::storage::filestore::FileStore;

const SID: &str = "11111111-aaaa-bbbb-cccc-222222222222";
const ZONE: &str = "agent:def-prov:current";

fn line(v: Value) -> String {
    v.to_string()
}

fn stored_user(text: &str) -> String {
    line(json!({ "type": "user", "session_id": SID, "message": { "role": "user", "content": text } }))
}

fn stored_assistant(blocks: Value) -> String {
    line(json!({ "type": "assistant", "session_id": SID, "message": { "id": "m1", "role": "assistant", "content": blocks } }))
}

fn stored_result(id: &str, text: &str) -> String {
    line(json!({ "type": "user", "session_id": SID, "message": { "role": "user", "content": [{ "type": "tool_result", "tool_use_id": id, "content": text }] } }))
}

fn provider_user(text: &str, uuid: &str) -> String {
    line(json!({ "parentUuid": null, "isSidechain": false, "sessionId": SID, "type": "user", "uuid": uuid, "timestamp": "2026-11-20T10:00:00Z", "message": { "role": "user", "content": text } }))
}

fn provider_assistant(blocks: Value, uuid: &str) -> String {
    line(json!({ "parentUuid": "p", "sessionId": SID, "type": "assistant", "uuid": uuid, "requestId": "req_1", "timestamp": "2026-11-20T10:00:01Z", "message": { "id": "m1", "role": "assistant", "model": "claude-x", "content": blocks } }))
}

fn provider_result(id: &str, text: &str, uuid: &str, extra: Value) -> String {
    line(json!({ "sessionId": SID, "type": "user", "uuid": uuid, "toolUseResult": extra, "message": { "role": "user", "content": [{ "type": "tool_result", "tool_use_id": id, "content": text }] } }))
}

struct Env {
    fs: FileStore,
    dir: tempfile::TempDir,
}

impl Env {
    fn new(stored: &[String]) -> Self {
        let fs = FileStore::open_in_memory().unwrap();
        let zone = FsZone { fs: &fs, zone: ZONE };
        let mut data = String::new();
        for l in stored {
            data.push_str(l);
            data.push('\n');
        }
        zone.append("output", data.as_bytes()).unwrap();
        Self { fs, dir: tempfile::tempdir().unwrap() }
    }

    fn zone(&self) -> FsZone<'_> {
        FsZone { fs: &self.fs, zone: ZONE }
    }

    fn files(&self) -> SessionFiles {
        SessionFiles::at(self.dir.path(), SID).unwrap()
    }

    fn write(&self, rel: &str, content: &str) {
        let p = self.dir.path().join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
    }

    fn provider(&self, lines: &[String]) {
        self.write(&format!("{SID}.jsonl"), &lines.iter().map(|l| format!("{l}\n")).collect::<String>());
    }

    fn text(&self, name: &str) -> String {
        String::from_utf8(self.zone().read_all(name).unwrap()).unwrap()
    }

    fn end(&self) -> Result<Outcome, String> {
        end_session(&self.files(), &[&self.zone()], &self.zone(), &mut go, &mut |_, _| {})
    }
}

/// A `before_remove` that holds nothing and allows the removal.
fn go() -> Result<Box<dyn std::any::Any>, String> {
    Ok(Box::new(()))
}

fn tool_use() -> Value {
    json!([{ "type": "tool_use", "id": "toolu_1", "name": "Bash", "input": { "command": "echo C-marker", "description": "d" } }])
}

#[test]
fn a_session_the_store_holds_is_removed_and_its_provider_fields_are_kept_in_the_sidecar() {
    let e = Env::new(&[stored_user("A-marker"), stored_assistant(tool_use()), stored_result("toolu_1", "D-marker")]);
    e.provider(&[
        provider_user("A-marker", "uuid-H-marker"),
        provider_assistant(tool_use(), "uuid-2"),
        provider_result("toolu_1", "D-marker", "uuid-3", json!({ "stdout": "D-marker", "interrupted": false })),
    ]);
    let out = e.end().unwrap();
    assert_eq!((out.records, out.matched, out.imported_records, out.imported_files, out.removed_files), (3, 3, 0, 0, 1));
    assert!(!e.files().jsonl.exists());
    let side = e.text(SIDECAR_FILE);
    assert!(side.contains("uuid-H-marker") && side.contains("req_1"), "ids and chain are kept");
    assert!(!side.contains("stdout"), "a toolUseResult the matched result covers adds nothing");
    assert_eq!(e.text("import/11111111-aaaa-bbbb-cccc-222222222222/11111111-aaaa-bbbb-cccc-222222222222.jsonl"), "");
}

#[test]
fn a_tool_use_result_with_text_the_result_lacks_goes_to_the_sidecar() {
    let e = Env::new(&[stored_result("toolu_1", "short")]);
    e.provider(&[provider_result("toolu_1", "short", "uuid-3", json!({ "file": { "filePath": "/p/Q-marker.rs" } }))]);
    e.end().unwrap();
    assert!(e.text(SIDECAR_FILE).contains("Q-marker"));
}

#[test]
fn a_record_the_store_lacks_is_imported_as_it_is_before_the_file_goes() {
    let missing = provider_assistant(json!([{ "type": "text", "text": "only-the-provider-has-this" }]), "uuid-9");
    let e = Env::new(&[stored_user("A-marker")]);
    let snapshot = line(json!({ "type": "file-history-snapshot", "messageId": "x", "snapshot": {} }));
    e.provider(&[provider_user("A-marker", "u1"), missing.clone(), snapshot.clone()]);
    let out = e.end().unwrap();
    assert_eq!((out.matched, out.imported_records), (1, 2));
    let imported = e.text(&import_name(SID, &format!("{SID}.jsonl")));
    assert_eq!(imported, format!("{missing}\n{snapshot}\n"), "verbatim, in order");
    assert!(!e.files().jsonl.exists());
}

#[test]
fn records_out_of_the_stores_order_are_not_matched() {
    let e = Env::new(&[stored_user("first"), stored_user("second")]);
    e.provider(&[provider_user("second", "u1"), provider_user("first", "u2")]);
    let out = e.end().unwrap();
    assert_eq!((out.matched, out.imported_records), (1, 1), "the one that is out of order is kept, not lost");
}

#[test]
fn a_spill_file_and_a_subagent_file_are_imported_and_every_file_goes() {
    let e = Env::new(&[stored_user("A-marker")]);
    e.provider(&[provider_user("A-marker", "u1")]);
    e.write(&format!("{SID}/tool-results/spill.txt"), "S-marker the whole output");
    e.write(&format!("{SID}/subagents/agent-a.jsonl"), &format!("{}\n", provider_user("sub-only-marker", "su1")));
    let out = e.end().unwrap();
    assert_eq!((out.files, out.imported_files, out.removed_files), (3, 1, 3));
    assert_eq!(e.text(&import_name(SID, "session/tool-results/spill.txt")), "S-marker the whole output");
    assert!(e.text(&import_name(SID, "session/subagents/agent-a.jsonl")).contains("sub-only-marker"));
    assert!(!e.files().folder.exists() && !e.files().jsonl.exists());
}

#[test]
fn a_file_that_is_not_text_is_imported_whole() {
    let e = Env::new(&[]);
    let bytes = [0xffu8, 0xfe, 0x00, b'\n', 0x80];
    std::fs::write(e.files().jsonl, bytes).unwrap();
    let out = e.end().unwrap();
    assert_eq!(out.imported_files, 1);
    assert_eq!(e.zone().read_all(&import_name(SID, &format!("{SID}.jsonl"))).unwrap(), bytes);
}

struct Failing<'a>(FsZone<'a>);

impl Zone for Failing<'_> {
    fn size(&self, name: &str) -> Result<u64, String> {
        self.0.size(name)
    }
    fn chunk(&self, name: &str, offset: u64, len: u64) -> Result<Vec<u8>, String> {
        self.0.chunk(name, offset, len)
    }
    fn append(&self, name: &str, bytes: &[u8]) -> Result<(), String> {
        if name.starts_with("import/") {
            return Err("disk full".to_string());
        }
        self.0.append(name, bytes)
    }
    fn delete(&self, name: &str) -> Result<(), String> {
        self.0.delete(name)
    }
}

#[test]
fn a_refusal_before_the_removal_removes_nothing_and_a_hold_lasts_until_the_files_are_gone() {
    let e = Env::new(&[stored_user("A-marker")]);
    e.provider(&[provider_user("A-marker", "u1")]);
    let r = end_session(&e.files(), &[&e.zone()], &e.zone(), &mut || Err("the agent is running".to_string()), &mut |_, _| panic!("nothing may be erased"));
    assert_eq!(r, Err("the agent is running".to_string()));
    assert!(e.files().jsonl.exists());
    let held = std::rc::Rc::new(());
    let seen = std::rc::Rc::clone(&held);
    let mut count = 0;
    end_session(&e.files(), &[&e.zone()], &e.zone(), &mut || Ok(Box::new(std::rc::Rc::clone(&seen))), &mut |_, _| count = std::rc::Rc::strong_count(&held)).unwrap();
    assert_eq!(count, 3, "the hold is still kept while a file is erased");
    assert_eq!(std::rc::Rc::strong_count(&held), 2, "and let go after");
}

#[test]
fn the_newest_write_anywhere_in_the_session_is_its_time() {
    let e = Env::new(&[]);
    e.provider(&[provider_user("x", "u1")]);
    let first = newest_modified_ms(&e.files()).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(30));
    e.write(&format!("{SID}/tool-results/late.txt"), "written later");
    assert!(newest_modified_ms(&e.files()).unwrap() > first, "a child file counts");
    assert_eq!(newest_modified_ms(&SessionFiles::at(e.dir.path(), "99999999-aaaa-bbbb-cccc-000000000000").unwrap()), None);
}

#[test]
fn a_failed_import_removes_nothing() {
    let e = Env::new(&[]);
    e.provider(&[provider_user("not-in-the-store", "u1")]);
    e.write(&format!("{SID}/tool-results/spill.txt"), "S");
    let r = end_session(&e.files(), &[&e.zone()], &Failing(e.zone()), &mut go, &mut |_, _| panic!("nothing may be erased"));
    assert!(r.is_err());
    assert!(e.files().jsonl.exists() && e.files().folder.join("tool-results/spill.txt").exists());
}

#[test]
fn a_secret_the_store_holds_as_its_mark_matches_and_is_never_imported() {
    let secret = "s3cret-value-for-provider-test-9f2";
    super::super::withhold::hold("provider-test", secret, None);
    let e = Env::new(&[stored_user(&format!("token {}", super::super::withhold::MARK))]);
    let extra = provider_assistant(json!([{ "type": "text", "text": format!("also {secret}") }]), "u2");
    e.provider(&[provider_user(&format!("token {secret}"), "u1"), extra]);
    let out = e.end().unwrap();
    super::super::withhold::release("provider-test");
    assert_eq!((out.matched, out.imported_records), (1, 1));
    let imported = e.text(&import_name(SID, &format!("{SID}.jsonl")));
    assert!(!imported.contains(secret) && imported.contains(super::super::withhold::MARK));
}

#[test]
fn removal_overwrites_each_file_before_it_is_unlinked() {
    let e = Env::new(&[stored_user("A-marker")]);
    e.provider(&[provider_user("A-marker", "u1")]);
    e.write(&format!("{SID}/tool-results/spill.txt"), "S-marker bytes");
    let mut seen: Vec<(String, u64, bool)> = Vec::new();
    end_session(&e.files(), &[&e.zone()], &e.zone(), &mut go, &mut |p: &Path, size| {
        let bytes = std::fs::read(p).unwrap();
        seen.push((p.file_name().unwrap().to_string_lossy().into_owned(), size, bytes.iter().all(|b| *b == 0)));
    })
    .unwrap();
    assert_eq!(seen.len(), 2);
    assert!(seen.iter().all(|(_, size, zeroed)| *size > 0 && *zeroed), "{seen:?}");
}

#[test]
fn another_sessions_files_and_a_foreign_name_are_not_touched() {
    let e = Env::new(&[stored_user("A-marker")]);
    e.provider(&[provider_user("A-marker", "u1")]);
    e.write("22222222-aaaa-bbbb-cccc-333333333333.jsonl", "other session\n");
    e.write("22222222-aaaa-bbbb-cccc-333333333333/x.txt", "other folder");
    e.write("notes.txt", "foreign");
    e.end().unwrap();
    for keep in ["22222222-aaaa-bbbb-cccc-333333333333.jsonl", "22222222-aaaa-bbbb-cccc-333333333333/x.txt", "notes.txt"] {
        assert!(e.dir.path().join(keep).exists(), "{keep}");
    }
    assert!(SessionFiles::at(e.dir.path(), "../escape").is_none());
    assert!(SessionFiles::at(e.dir.path(), "a/b-cccccccc").is_none());
    assert!(SessionFiles::at(e.dir.path(), "short").is_none());
}

#[test]
fn importing_the_same_records_twice_adds_nothing() {
    let e = Env::new(&[]);
    let lines = vec![provider_user("x", "u1"), provider_user("y", "u2")];
    import_lines(&e.zone(), "import/f", &lines).unwrap();
    import_lines(&e.zone(), "import/f", &lines).unwrap();
    assert_eq!(e.text("import/f").lines().count(), 2);
}

#[test]
fn a_store_line_of_another_session_does_not_match() {
    let other = line(json!({ "type": "user", "session_id": "someone-else-entirely", "message": { "role": "user", "content": "A-marker" } }));
    let e = Env::new(&[other]);
    e.provider(&[provider_user("A-marker", "u1")]);
    let out = e.end().unwrap();
    assert_eq!((out.matched, out.imported_records), (0, 1));
}

#[test]
fn a_secret_in_a_spill_file_is_masked_in_the_import() {
    let secret = "spill-secret-value-for-provider-test-77c";
    super::super::withhold::hold("provider-spill", secret, None);
    let e = Env::new(&[stored_user("A-marker")]);
    e.provider(&[provider_user("A-marker", "u1")]);
    e.write(&format!("{SID}/tool-results/spill.txt"), &format!("before {secret} after"));
    let out = e.end();
    super::super::withhold::release("provider-spill");
    assert_eq!(out.unwrap().imported_files, 1);
    let imported = e.text(&import_name(SID, "session/tool-results/spill.txt"));
    assert!(!imported.contains(secret) && imported.contains(super::super::withhold::MARK), "{imported}");
}

#[test]
fn a_file_that_is_not_text_and_holds_a_secret_is_refused_and_nothing_is_removed() {
    let secret = "binary-secret-value-for-provider-test-5d1";
    super::super::withhold::hold("provider-bin", secret, None);
    let e = Env::new(&[stored_user("A-marker")]);
    e.provider(&[provider_user("A-marker", "u1")]);
    let mut bytes = vec![0xffu8, 0xfe];
    bytes.extend_from_slice(secret.as_bytes());
    e.write(&format!("{SID}/blob.bin"), "");
    std::fs::write(e.files().folder.join("blob.bin"), bytes).unwrap();
    let r = e.end();
    super::super::withhold::release("provider-bin");
    assert!(r.is_err());
    assert!(e.files().jsonl.exists() && e.files().folder.join("blob.bin").exists());
}

#[test]
fn a_file_that_changed_after_the_proof_is_not_removed() {
    let e = Env::new(&[stored_user("A-marker")]);
    e.provider(&[provider_user("A-marker", "u1")]);
    let files = e.files();
    let proved = vec![(files.jsonl.clone(), file_digest(&files.jsonl).unwrap())];
    // A record appended after the proof read the file.
    let mut f = std::fs::OpenOptions::new().append(true).open(&files.jsonl).unwrap();
    f.write_all(b"{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"late\"}}\n").unwrap();
    drop(f);
    let r = remove_proved(&files, &proved, &mut |_, _| panic!("nothing may be erased"));
    assert!(r.is_err() && files.jsonl.exists());
    // A file that was added.
    e.write(&format!("{SID}/new.txt"), "x");
    let r = remove_proved(&files, &proved, &mut |_, _| panic!("nothing may be erased"));
    assert!(r.is_err() && files.folder.join("new.txt").exists());
}

#[test]
fn nothing_is_read_while_secret_matching_is_closed() {
    assert!(require_open_matching("closed").is_err());
    assert!(require_open_matching("pattern").is_ok() && require_open_matching("none").is_ok());
}
