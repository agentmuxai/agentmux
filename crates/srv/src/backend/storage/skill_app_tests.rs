// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;

fn catalog() -> Store {
    let s = Store::open_in_memory().unwrap();
    s.apply_identity_schema_for_tests().unwrap();
    s
}

fn info(version: &str) -> SkillAppInfo {
    SkillAppInfo {
        publisher: "acme".into(),
        id: "word".into(),
        name: "Word".into(),
        version: version.into(),
        vendor: "Acme".into(),
        executables: BTreeMap::from([("windows".to_string(), vec!["WINWORD.EXE".to_string()])]),
        title_hints: vec![],
        platforms: vec!["windows".into()],
        ladder: vec![LadderStep { kind: "api".into(), note: "COM".into() }],
        mcp: None,
        supported: None,
        tested: vec![],
        references: vec![],
    }
}

fn manifest(version: &str) -> String {
    format!(
        r#"{{"schema":"computer-use-skill/v1","publisher":"acme","id":"word","name":"Word","version":"{version}",
"changelog":[{{"version":"{version}"}}],"app":{{"vendor":"Acme","executables":{{"windows":["WINWORD.EXE"]}}}},
"platforms":["windows"],"ladder":[{{"kind":"api","note":"COM"}}]}}"#
    )
}

fn file(path: &str, content: &str, script: bool) -> SkillFileInput {
    SkillFileInput { path: path.into(), content: content.into(), image: None, script }
}

fn record(skill_version: &str, result: &str) -> VerifiedRecord {
    VerifiedRecord {
        skill_version: skill_version.into(),
        app_version: "16.0".into(),
        platform: "windows".into(),
        date: "2026-11-06".into(),
        runner: "AgentMux test machine".into(),
        route: "api".into(),
        result: result.into(),
        notes: String::new(),
        evidence_sha256: "ab".repeat(32),
    }
}

#[test]
fn a_skill_with_no_manifest_is_not_a_computer_use_skill() {
    let c = catalog();
    assert_eq!(c.skill_app_get("s1").unwrap(), None);
    assert!(c.skill_app_map().unwrap().is_empty());
    assert!(c.skill_files("s1").unwrap().is_empty());
}

#[test]
fn a_manifest_and_its_files_are_stored_listed_and_read() {
    let c = catalog();
    let files = [file("controls.md", "# controls\n", false), file("scripts/run.py", "print(1)\n", true)];
    c.skill_app_set("s1", &info("1.0.0"), &manifest("1.0.0"), "hash1", &files, 1).unwrap();
    let app = c.skill_app_get("s1").unwrap().unwrap();
    assert_eq!((app.info.version.as_str(), app.content_hash.as_str()), ("1.0.0", "hash1"));
    assert_eq!(
        app.files,
        vec![
            SkillFileInfo { path: "controls.md".into(), bytes: 11, script: false, approved: false, image: false },
            SkillFileInfo { path: "scripts/run.py".into(), bytes: 9, script: true, approved: false, image: false },
        ]
    );
    assert_eq!(c.skill_file_get("s1", "controls.md").unwrap().unwrap().content, "# controls\n");
    assert_eq!(c.skill_file_get("s1", "missing.md").unwrap(), None);
    assert_eq!(c.skill_app_map().unwrap().len(), 1);
}

#[test]
fn a_new_version_replaces_the_files_and_a_script_stays_approved_only_with_the_same_text() {
    let c = catalog();
    let v1 = [file("controls.md", "old", false), file("scripts/a.py", "A", true), file("scripts/b.py", "B", true)];
    c.skill_app_set("s1", &info("1.0.0"), &manifest("1.0.0"), "h1", &v1, 1).unwrap();
    // A person approved both scripts (stage 4 does this; here the row is set directly).
    c.conn.lock().unwrap().execute("UPDATE db_skill_file SET approved = 1 WHERE script = 1", []).unwrap();
    let v2 = [file("controls.md", "new", false), file("scripts/a.py", "A", true), file("scripts/b.py", "B changed", true)];
    c.skill_app_set("s1", &info("1.1.0"), &manifest("1.1.0"), "h2", &v2, 2).unwrap();
    let app = c.skill_app_get("s1").unwrap().unwrap();
    assert_eq!(app.info.version, "1.1.0");
    let state: Vec<(&str, bool)> = app.files.iter().filter(|f| f.script).map(|f| (f.path.as_str(), f.approved)).collect();
    assert_eq!(state, vec![("scripts/a.py", true), ("scripts/b.py", false)]);
    assert_eq!(c.skill_file_get("s1", "controls.md").unwrap().unwrap().content, "new");
}

#[test]
fn pass_records_are_added_once_and_survive_a_new_version() {
    let c = catalog();
    c.skill_app_set("s1", &info("1.0.0"), &manifest("1.0.0"), "h1", &[], 1).unwrap();
    assert!(c.skill_verified_append("s1", &record("1.0.0", "pass"), "agent-a", 2).unwrap());
    assert!(!c.skill_verified_append("s1", &record("1.0.0", "pass"), "agent-b", 3).unwrap(), "the same record twice is kept once");
    assert!(c.skill_verified_append("s1", &record("1.0.0", "fail"), "agent-a", 4).unwrap());
    c.skill_app_set("s1", &info("1.1.0"), &manifest("1.1.0"), "h2", &[], 5).unwrap();
    let app = c.skill_app_get("s1").unwrap().unwrap();
    assert_eq!(app.verified.iter().map(|r| (r.skill_version.as_str(), r.result.as_str())).collect::<Vec<_>>(), vec![("1.0.0", "pass"), ("1.0.0", "fail")]);
}

#[test]
fn forgetting_a_skill_drops_its_manifest_files_and_records_and_no_other() {
    let c = catalog();
    for id in ["s1", "s2"] {
        c.skill_app_set(id, &info("1.0.0"), &manifest("1.0.0"), "h", &[file("a.md", "x", false)], 1).unwrap();
        c.skill_verified_append(id, &record("1.0.0", "pass"), "a", 1).unwrap();
    }
    c.skill_app_forget("s1").unwrap();
    assert_eq!(c.skill_app_get("s1").unwrap(), None);
    assert!(c.skill_files("s1").unwrap().is_empty());
    let kept = c.skill_app_get("s2").unwrap().unwrap();
    assert_eq!((kept.files.len(), kept.verified.len()), (1, 1));
}

#[test]
fn a_manifest_this_build_cannot_read_is_left_out_of_the_list() {
    let c = catalog();
    c.skill_app_set("s1", &info("1.0.0"), "{\"schema\":\"computer-use-skill/v9\"}", "h", &[], 1).unwrap();
    c.skill_app_set("s2", &info("1.0.0"), &manifest("1.0.0"), "h", &[], 1).unwrap();
    assert_eq!(c.skill_app_map().unwrap().keys().collect::<Vec<_>>(), vec!["s2"]);
}

#[test]
fn a_reference_crop_is_stored_as_bytes_and_listed_as_an_image() {
    let c = catalog();
    let png = vec![0x89, b'P', b'N', b'G', 0, 1, 2, 255, 254];
    let files = [SkillFileInput { path: "references/fader.png".into(), content: String::new(), image: Some(png.clone()), script: false }];
    c.skill_app_set("s1", &info("1.0.0"), &manifest("1.0.0"), "h", &files, 1).unwrap();
    let app = c.skill_app_get("s1").unwrap().unwrap();
    assert_eq!(app.files, vec![SkillFileInfo { path: "references/fader.png".into(), bytes: 9, script: false, approved: false, image: true }]);
    let f = c.skill_file_get("s1", "references/fader.png").unwrap().unwrap();
    assert_eq!((f.image, f.content.as_str()), (Some(png), ""));
}
