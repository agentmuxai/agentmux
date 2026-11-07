// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::backend::storage::skill_app::{ScriptApproval, ScriptOutcome};

const SKILL_MD: &str = "---\nname: acme-word\ndescription: Use when driving Word (WINWORD.EXE).\n---\n\n# Word\n\nBody.\n";
const TEST_JSON: &str = "{\"skill\":\"acme.word\",\"task\":\"t\",\"steps\":[],\"verify\":[\"a\"]}\n";
const PY: &str = "print('hi')\n";

fn manifest(version: &str, extra: &str) -> String {
    format!(
        r#"{{"schema":"computer-use-skill/v1","publisher":"acme","id":"word","name":"Word","version":"{version}",
"changelog":[{{"version":"{version}","date":"2026-11-05","by":"x","change":"first"}}],
"app":{{"vendor":"Acme","executables":{{"windows":["WINWORD.EXE"]}},"titleHints":["Word"],"versions":{{"supported":">=16","tested":["16.0"]}}}},
"platforms":["windows"],"ladder":[{{"kind":"api","note":"COM"}},{{"kind":"desktop","note":"clicks"}}],"mcp":null{extra}}}"#
    )
}

/// A skill folder named `acme-word` under a fresh temp dir.
fn folder(files: &[(&str, &str)]) -> (tempfile::TempDir, std::path::PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("acme-word");
    for (path, text) in files {
        let p = dir.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    (tmp, dir)
}

fn base_files(m: &str) -> Vec<(&'static str, String)> {
    vec![("SKILL.md", SKILL_MD.to_string()), (MANIFEST, m.to_string()), ("test.json", TEST_JSON.to_string())]
}

fn read(files: &[(&str, String)]) -> Result<Bundle, String> {
    let refs: Vec<(&str, &str)> = files.iter().map(|(p, t)| (*p, t.as_str())).collect();
    let (_tmp, dir) = folder(&refs);
    read_folder(&dir)
}

#[test]
fn a_skill_folder_is_read_with_its_manifest_body_and_files() {
    let mut files = base_files(&manifest("1.0.0", ""));
    files.push(("controls.md", "# controls\n".to_string()));
    let b = read(&files).unwrap();
    assert_eq!((b.name.as_str(), b.info.publisher.as_str(), b.info.id.as_str(), b.info.version.as_str()), ("acme-word", "acme", "word", "1.0.0"));
    assert_eq!(b.description, "Use when driving Word (WINWORD.EXE).");
    assert!(b.content.starts_with("# Word"));
    assert_eq!(b.info.executables["windows"], vec!["WINWORD.EXE".to_string()]);
    assert_eq!((b.info.supported.as_deref(), b.info.tested.clone()), (Some(">=16"), vec!["16.0".to_string()]));
    assert_eq!(b.info.ladder.len(), 2);
    let paths: Vec<&str> = b.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, vec!["controls.md", "test.json"]);
    assert!(b.files.iter().all(|f| !f.script));
    assert!(b.signature.is_none() && b.verified.is_empty());
}

#[test]
fn the_content_hash_is_the_catalogs_and_ignores_the_signature_and_pass_records() {
    // The same definition as the agentmuxai/skills repo's contentHash: sha256 over "path\0sha256(file)\n", by path.
    let mut files = BTreeMap::new();
    files.insert("b.md".to_string(), b"two".to_vec());
    files.insert("a.md".to_string(), b"one".to_vec());
    let want = {
        let mut h = Sha256::new();
        h.update(format!("a.md\0{}\nb.md\0{}\n", sha256_hex(b"one"), sha256_hex(b"two")).as_bytes());
        h.finalize().iter().map(|b| format!("{b:02x}")).collect::<String>()
    };
    assert_eq!(content_hash(&files), want);
    // The value the catalog's own contentHash computes for these files plus a signature and a pass record.
    files.insert(SIG_FILE.to_string(), b"s".to_vec());
    files.insert(VERIFIED_FILE.to_string(), b"[]".to_vec());
    assert_eq!(content_hash(&files), "46f5d1f7e67701da2cc6000bda01c9b1f7e0795ca1756372c41c08d5cb850133");
    files.remove(SIG_FILE);
    files.remove(VERIFIED_FILE);
    let before = content_hash(&files);
    files.insert(SIG_FILE.to_string(), b"sig".to_vec());
    files.insert(VERIFIED_FILE.to_string(), b"[]".to_vec());
    assert_eq!(content_hash(&files), before);
    files.insert("c.md".to_string(), b"x".to_vec());
    assert_ne!(content_hash(&files), before);
}

#[test]
fn a_pass_record_and_a_signature_are_read_but_do_not_change_the_hash() {
    let plain = read(&base_files(&manifest("1.0.0", ""))).unwrap();
    let mut files = base_files(&manifest("1.0.0", ""));
    files.push((SIG_FILE, "AAAA\n".to_string()));
    files.push((
        VERIFIED_FILE,
        r#"[{"skillVersion":"1.0.0","appVersion":"16.0","platform":"windows","date":"2026-11-06","runner":"AgentMux test machine","result":"pass"}]"#.to_string(),
    ));
    let b = read(&files).unwrap();
    assert_eq!(b.content_hash, plain.content_hash);
    assert_eq!(b.signature.as_deref(), Some("AAAA"));
    assert_eq!(b.verified.len(), 1);
    assert_eq!((b.verified[0].app_version.as_str(), b.verified[0].result.as_str(), b.verified[0].route.as_str()), ("16.0", "pass", ""));
}

#[test]
fn a_script_must_be_listed_with_the_hash_it_has() {
    let mut files = base_files(&manifest("1.0.0", ""));
    files.push(("scripts/run.py", PY.to_string()));
    let unlisted = read(&files).unwrap_err();
    assert!(unlisted.contains("is a script"), "{unlisted}");

    let listed = |sha: &str| manifest("1.0.0", &format!(r#","scripts":[{{"path":"scripts/run.py","sha256":"{sha}","purpose":"test","needs":[]}}]"#));
    let mut ok = base_files(&listed(&sha256_hex(PY.as_bytes())));
    ok.push(("scripts/run.py", PY.to_string()));
    let b = read(&ok).unwrap();
    assert!(b.files.iter().any(|f| f.path == "scripts/run.py" && f.script));

    let mut wrong = base_files(&listed(&sha256_hex(b"other")));
    wrong.push(("scripts/run.py", PY.to_string()));
    assert!(read(&wrong).unwrap_err().contains("doesn't match"));

    let missing = base_files(&listed(&sha256_hex(PY.as_bytes())));
    assert!(read(&missing).unwrap_err().contains("which isn't in the folder"));
}

#[test]
fn a_folder_that_breaks_a_rule_is_refused_with_the_rule() {
    // Folder name against the manifest.
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("other-name");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("SKILL.md"), SKILL_MD).unwrap();
    std::fs::write(dir.join(MANIFEST), manifest("1.0.0", "")).unwrap();
    assert!(read_folder(&dir).unwrap_err().contains("must match the folder"));

    // A binary file.
    let mut files = base_files(&manifest("1.0.0", ""));
    files.push(("icon.png", "x".to_string()));
    assert!(read(&files).unwrap_err().contains("only text and script files"));

    // A changelog that does not end at the version.
    let bad = manifest("1.0.0", "").replace(r#"{"version":"1.0.0","date""#, r#"{"version":"0.9.0","date""#);
    assert!(read(&base_files(&bad)).unwrap_err().contains("changelog must end"));

    // A command line where a catalog id belongs.
    let cmd = manifest("1.0.0", "").replace(r#""mcp":null"#, r#""mcp":{"command":"uvx x"}"#);
    assert!(read(&base_files(&cmd)).unwrap_err().contains("never a command line"));
    let spaced = manifest("1.0.0", "").replace(r#""mcp":null"#, r#""mcp":"uvx x""#);
    assert!(read(&base_files(&spaced)).unwrap_err().contains("never a command line"));

    // A ladder rung that is not one of the four.
    let rung = manifest("1.0.0", "").replace(r#""kind":"api""#, r#""kind":"magic""#);
    assert!(read(&base_files(&rung)).unwrap_err().contains("ladder"));
}

#[test]
fn an_mcp_pointer_may_be_a_bare_id_or_an_object_with_a_catalog_id() {
    let id = manifest("1.0.0", "").replace(r#""mcp":null"#, r#""mcp":"ableton-live""#);
    assert_eq!(info_from_manifest(&id).unwrap().mcp.as_deref(), Some("ableton-live"));
    let obj = manifest("1.0.0", "").replace(r#""mcp":null"#, r#""mcp":{"catalogId":"ableton-live","note":"n"}"#);
    assert_eq!(info_from_manifest(&obj).unwrap().mcp.as_deref(), Some("ableton-live"));
    // The Word and Excel skills say there is no catalog entry yet.
    let none_yet = manifest("1.0.0", "").replace(r#""mcp":null"#, r#""mcp":{"catalogId":null,"note":"none yet"}"#);
    assert_eq!(info_from_manifest(&none_yet).unwrap().mcp, None);
    let number = manifest("1.0.0", "").replace(r#""mcp":null"#, r#""mcp":{"catalogId":5}"#);
    assert!(info_from_manifest(&number).unwrap_err().contains("string or null"));
}

#[test]
fn relative_paths_that_could_leave_the_skill_folder_are_not_valid() {
    for ok in ["controls.md", "scripts/run.py", "a/b/c/d.md"] {
        assert!(valid_rel_path(ok), "{ok}");
    }
    for bad in ["", "/etc/x", "../x", "a/../x", "a//b", "a\\b", "C:/x", "a/./b", "a/b/c/d/e.md", "x.", "SKILL.md:stream"] {
        assert!(!valid_rel_path(bad), "{bad:?}");
    }
}

#[test]
fn an_executable_matches_by_file_name_on_its_platform_only() {
    let info = info_from_manifest(&manifest("1.0.0", "")).unwrap();
    assert!(matches_executable(&info, "windows", "winword.exe"));
    assert!(matches_executable(&info, "windows", "C:\\Program Files\\Office\\WINWORD.EXE"));
    assert!(!matches_executable(&info, "windows", "notepad.exe"));
    assert!(!matches_executable(&info, "macos", "WINWORD.EXE"));
    assert!(!matches_executable(&info, "windows", ""));
}

/// A PNG of the given size: the signature, an IHDR, an IDAT, an optional extra chunk, an IEND. `check_png` does not
/// check CRCs, so they are left zero.
fn png(w: u32, h: u32, extra: Option<&str>) -> Vec<u8> {
    let chunk = |kind: &str, data: &[u8]| {
        let mut c = (data.len() as u32).to_be_bytes().to_vec();
        c.extend(kind.as_bytes());
        c.extend(data);
        c.extend([0u8; 4]);
        c
    };
    let mut ihdr = w.to_be_bytes().to_vec();
    ihdr.extend(h.to_be_bytes());
    ihdr.extend([8, 2, 0, 0, 0]);
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    out.extend(chunk("IHDR", &ihdr));
    out.extend(chunk("IDAT", &[1, 2, 3]));
    if let Some(kind) = extra {
        out.extend(chunk(kind, b"Author=someone"));
    }
    out.extend(chunk("IEND", &[]));
    out
}

#[test]
fn a_png_crop_is_checked_for_type_size_pixels_and_metadata() {
    assert!(check_png("references/a.png", &png(400, 300, None)).is_ok());
    assert!(check_png("references/a.png", &png(801, 300, None)).unwrap_err().contains("at most 800x600"));
    assert!(check_png("references/a.png", &png(400, 601, None)).is_err());
    assert!(check_png("references/a.png", &png(0, 10, None)).is_err());
    assert!(check_png("references/a.png", &png(10, 10, Some("tEXt"))).unwrap_err().contains("tEXt chunk"));
    assert!(check_png("references/a.png", &png(10, 10, Some("eXIf"))).is_err());
    assert!(check_png("references/a.png", b"GIF89a....").unwrap_err().contains("not a PNG"));
    let mut truncated = png(10, 10, None);
    truncated.truncate(truncated.len() - 6);
    assert!(check_png("references/a.png", &truncated).is_err());
    let mut trailing = png(10, 10, None);
    trailing.extend(b"junk");
    assert!(check_png("references/a.png", &trailing).unwrap_err().contains("not a complete PNG"));
    let mut big = png(10, 10, None);
    big.extend(vec![0u8; MAX_REFERENCE_BYTES]);
    assert!(check_png("references/a.png", &big).unwrap_err().contains("KB"));
}

fn reference_entry(path: &str, sha: &str) -> String {
    format!(
        r#","references":[{{"path":"{path}","label":"master fader","shows":"the fader cap","appVersion":"16.0","platform":"windows","theme":"dark","scale":100,"capturedBy":"AgentMux test machine","date":"2026-11-06","sha256":"{sha}"}}]"#
    )
}

fn with_crop(bytes: &[u8], listed_sha: &str) -> Result<Bundle, String> {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("acme-word");
    std::fs::create_dir_all(dir.join("references")).unwrap();
    std::fs::write(dir.join("SKILL.md"), SKILL_MD).unwrap();
    std::fs::write(dir.join(MANIFEST), manifest("1.0.0", &reference_entry("references/fader.png", listed_sha))).unwrap();
    std::fs::write(dir.join("references/fader.png"), bytes).unwrap();
    read_folder(&dir)
}

#[test]
fn a_listed_crop_is_read_as_an_image_with_its_tags_and_counts_in_the_hash() {
    let bytes = png(200, 100, None);
    let b = with_crop(&bytes, &sha256_hex(&bytes)).unwrap();
    let f = b.files.iter().find(|f| f.path == "references/fader.png").unwrap();
    assert_eq!((f.image.as_deref(), f.content.as_str(), f.script), (Some(bytes.as_slice()), "", false));
    let r = &b.info.references[0];
    assert_eq!(
        (r.label.as_str(), r.app_version.as_str(), r.theme.as_str(), r.scale, r.captured_by.as_str()),
        ("master fader", "16.0", "dark", 100, "AgentMux test machine")
    );
    let other = png(200, 101, None);
    let b2 = with_crop(&other, &sha256_hex(&other)).unwrap();
    assert_ne!(b.content_hash, b2.content_hash, "the crop is part of what is signed");
}

#[test]
fn a_crop_without_tags_a_wrong_hash_or_outside_references_is_refused() {
    let bytes = png(200, 100, None);
    assert!(with_crop(&bytes, &"0".repeat(64)).unwrap_err().contains("doesn't match"));
    assert!(with_crop(&bytes, "short").unwrap_err().contains("sha256"));
    let texty = png(10, 10, Some("tEXt"));
    assert!(with_crop(&texty, &sha256_hex(&texty)).unwrap_err().contains("tEXt"));

    // Unlisted crop.
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("acme-word");
    std::fs::create_dir_all(dir.join("references")).unwrap();
    std::fs::write(dir.join("SKILL.md"), SKILL_MD).unwrap();
    std::fs::write(dir.join(MANIFEST), manifest("1.0.0", "")).unwrap();
    std::fs::write(dir.join("references/fader.png"), &bytes).unwrap();
    assert!(read_folder(&dir).unwrap_err().contains("list it in"));

    // A PNG outside references/.
    std::fs::write(dir.join("icon.png"), &bytes).unwrap();
    std::fs::remove_file(dir.join("references/fader.png")).unwrap();
    assert!(read_folder(&dir).unwrap_err().contains("only text and script files"));

    // A listed crop that is not in the folder.
    std::fs::remove_file(dir.join("icon.png")).unwrap();
    std::fs::write(dir.join(MANIFEST), manifest("1.0.0", &reference_entry("references/fader.png", &sha256_hex(&bytes)))).unwrap();
    assert!(read_folder(&dir).unwrap_err().contains("isn't in the folder"));

    // A tag missing.
    let no_version = manifest("1.0.0", &reference_entry("references/fader.png", &sha256_hex(&bytes))).replace("\"appVersion\":\"16.0\",", "");
    assert!(info_from_manifest(&no_version).unwrap_err().contains("the app version"));
}

// ── install, import, launch files and the Desktop pointer ───────────────────────────────────

fn catalog() -> Store {
    let s = Store::open_in_memory().unwrap();
    s.apply_identity_schema_for_tests().unwrap();
    s
}

fn catalog_skill(id: &str, name: &str) -> crate::backend::storage::Skill {
    crate::backend::storage::Skill {
        id: id.into(),
        name: name.into(),
        trigger: String::new(),
        skill_type: "agent-skill".into(),
        description: "d".into(),
        content: "body".into(),
        is_global: true,
        created_at: 1,
        updated_at: 1,
    }
}

fn agent_skill(id: &str, name: &str) -> AgentSkill {
    AgentSkill {
        id: id.into(),
        agent_id: "a1".into(),
        name: name.into(),
        trigger: String::new(),
        skill_type: "agent-skill".into(),
        description: "d".into(),
        content: "body".into(),
        created_at: 1,
    }
}

/// A scanned folder installed into a catalog: the skill row, its manifest, files and the pass record it carried.
fn installed() -> (Store, Bundle) {
    let listed = |sha: &str| manifest("1.0.0", &format!(r#","scripts":[{{"path":"scripts/run.py","sha256":"{sha}","purpose":"test","needs":[]}}]"#));
    let mut files = base_files(&listed(&sha256_hex(PY.as_bytes())));
    files.push(("scripts/run.py", PY.to_string()));
    files.push(("controls.md", "# controls\n".to_string()));
    files.push((VERIFIED_FILE, r#"[{"skillVersion":"1.0.0","appVersion":"16.0","platform":"windows","date":"2026-11-06","runner":"AgentMux test machine","result":"pass"}]"#.to_string()));
    let b = read(&files).unwrap();
    let c = catalog();
    c.skill_upsert_unique_global(&catalog_skill("s1", &b.name)).unwrap();
    install_checked(&c, "s1", &b, 5).unwrap();
    (c, b)
}

#[test]
fn installing_a_bundle_stores_the_manifest_files_and_the_pass_records_it_carried() {
    let (c, b) = installed();
    let app = c.skill_app_get("s1").unwrap().unwrap();
    assert_eq!((app.info.version.as_str(), app.content_hash.as_str()), ("1.0.0", b.content_hash.as_str()));
    let files: Vec<(&str, bool, bool)> = app.files.iter().map(|f| (f.path.as_str(), f.script, f.approved)).collect();
    assert_eq!(files, vec![("controls.md", false, false), ("scripts/run.py", true, false), ("test.json", false, false)]);
    assert_eq!(app.verified.len(), 1);
    // Installed again (an update to the same folder): nothing doubles.
    install_checked(&c, "s1", &b, 6).unwrap();
    let again = c.skill_app_get("s1").unwrap().unwrap();
    assert_eq!((again.files.len(), again.verified.len()), (3, 1));
}

#[test]
fn a_launch_writes_the_files_beside_skill_md_but_not_an_unapproved_script_or_a_crop() {
    let (c, _) = installed();
    let skills = vec![agent_skill("s1", "acme-word")];
    let paths = |c: &Store| sibling_files(&skills, c).into_iter().map(|f| f.filename).collect::<Vec<_>>();
    assert_eq!(paths(&c), vec![".claude/skills/acme-word/controls.md", ".claude/skills/acme-word/test.json"]);

    // Once a person approves the script (stage 4 does this; the row is set directly here), it is written too.
    c.conn().lock().unwrap().execute("UPDATE db_skill_file SET approved = 1 WHERE script = 1", []).unwrap();
    assert!(paths(&c).contains(&".claude/skills/acme-word/scripts/run.py".to_string()));

    // Raw text, not expanded: a script may hold {{ of its own.
    let body = sibling_files(&skills, &c).into_iter().find(|f| f.filename.ends_with("run.py")).unwrap().content;
    assert_eq!(body, PY);
}

#[test]
fn the_files_follow_the_same_folder_name_as_skill_md_when_two_skills_share_a_name() {
    let (c, b) = installed();
    c.skill_upsert_unique_global(&catalog_skill("s2", "another-name")).unwrap();
    install_checked(&c, "s2", &b, 5).unwrap();
    // Two names that make the same folder name: the second is made unique, and its files follow it.
    let skills = vec![agent_skill("s1", "acme-word"), agent_skill("s2", "Acme Word")];
    let written = crate::backend::agent_config::build_config_files(&Default::default(), &skills, "A", "a1", "a", "w", "claude");
    let skill_md: Vec<&str> = written.iter().filter(|f| f.filename.ends_with("/SKILL.md")).map(|f| f.filename.as_str()).collect();
    assert_eq!(skill_md, vec![".claude/skills/acme-word/SKILL.md", ".claude/skills/acme-word-2/SKILL.md"]);
    let sib: Vec<String> = sibling_files(&skills, &c).into_iter().map(|f| f.filename).filter(|n| n.ends_with("controls.md")).collect();
    assert_eq!(sib, vec![".claude/skills/acme-word/controls.md", ".claude/skills/acme-word-2/controls.md"]);
}

#[test]
fn the_only_files_a_prompt_skill_or_an_empty_one_gets_are_none() {
    let (c, _) = installed();
    let mut prompt = agent_skill("s1", "acme-word");
    prompt.skill_type = "prompt".into();
    let mut empty = agent_skill("s1", "acme-word");
    empty.content = String::new();
    assert!(sibling_files(&[prompt], &c).is_empty());
    assert!(sibling_files(&[empty], &c).is_empty());
}

#[test]
fn desktop_apps_points_at_the_skill_for_a_window_and_says_whether_it_was_tested_here() {
    let (c, _) = installed();
    let p = skill_for_executable(&c, "windows", "C:\\Program Files\\Office\\winword.exe").unwrap();
    assert_eq!((p.id.as_str(), p.name.as_str(), p.version.as_str(), p.verified_here), ("s1", "acme-word", "1.0.0", true));
    assert_eq!(skill_for_executable(&c, "windows", "WINWORD").map(|p| p.id), Some("s1".to_string()), "with or without .exe");
    assert_eq!(skill_for_executable(&c, "windows", "notepad.exe"), None);
    assert_eq!(skill_for_executable(&c, "macos", "WINWORD.EXE"), None);
    // No pass on this platform: the pointer stays, it says not tested.
    c.conn().lock().unwrap().execute("DELETE FROM db_skill_verified", []).unwrap();
    assert!(!skill_for_executable(&c, "windows", "WINWORD.EXE").unwrap().verified_here);
}

#[test]
fn a_preview_is_kept_by_hash_for_the_commit_and_a_newer_one_replaces_it() {
    let (_, b) = installed();
    assert_eq!(stashed(&b.content_hash), None);
    stash(&b);
    assert_eq!(stashed(&b.content_hash).map(|k| k.name), Some("acme-word".to_string()));
    assert!(stashed("not-a-hash").is_none());
    stash(&b);
    assert_eq!(STASH.lock().unwrap().iter().filter(|(_, k)| k.content_hash == b.content_hash).count(), 1);
}

#[test]
fn a_scan_reads_a_computer_use_folder_whole_and_refuses_a_broken_one_with_the_reason() {
    use crate::backend::skill_import::scan_dir;
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("skills");
    for (name, ok) in [("acme-word", true), ("acme-broken", false)] {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("SKILL.md"), SKILL_MD.replace("acme-word", name)).unwrap();
        // The broken one's manifest names a different folder.
        std::fs::write(dir.join(MANIFEST), if ok { manifest("1.0.0", "") } else { manifest("1.0.0", "").replace("\"id\":\"word\"", "\"id\":\"other\"") }).unwrap();
    }
    // A plain skill beside them still imports as a plain skill.
    std::fs::create_dir_all(root.join("plain")).unwrap();
    std::fs::write(root.join("plain/SKILL.md"), "---\nname: plain\ndescription: p\n---\nbody\n").unwrap();

    let scan = scan_dir(&root).unwrap();
    let by_name: std::collections::BTreeMap<&str, bool> = scan.candidates.iter().map(|c| (c.name.as_str(), c.app.is_some())).collect();
    assert_eq!(by_name, [("acme-word", true), ("plain", false)].into_iter().collect());
    assert_eq!(scan.warnings.len(), 1);
    assert!(scan.warnings[0].contains("acme-broken") && scan.warnings[0].contains("must be acme-other"), "{:?}", scan.warnings);
}

#[test]
fn a_pass_on_an_older_version_does_not_say_tested_here() {
    let (c, _) = installed();
    let v2 = read(&base_files(&manifest("1.1.0", ""))).unwrap();
    install_checked(&c, "s1", &v2, 7).unwrap();
    let p = skill_for_executable(&c, "windows", "WINWORD.EXE").unwrap();
    assert_eq!((p.version.as_str(), p.verified_here), ("1.1.0", false), "the 1.0.0 pass is kept but does not speak for 1.1.0");
}

#[test]
fn a_name_the_folder_uses_is_reserved_in_any_letters_and_never_written_beside_skill_md() {
    // Not read from a disk: on a case-insensitive one `skill.md` and `SKILL.md` are the same file, which is the point.
    for name in ["SKILL.md", "skill.md", "Skill.MD", "Computer-Use.json", "VERIFIED.json", "Skill.Sig"] {
        assert!(is_reserved_name(name), "{name}");
    }
    assert!(!is_reserved_name("controls.md") && !is_reserved_name("references/skill.md"));

    let (c, _) = installed();
    c.conn().lock().unwrap().execute("UPDATE db_skill_file SET path = 'skill.md' WHERE path = 'controls.md'", []).unwrap();
    let skills = vec![agent_skill("s1", "acme-word")];
    let paths: Vec<String> = sibling_files(&skills, &c).into_iter().map(|f| f.filename).collect();
    assert_eq!(paths, vec![".claude/skills/acme-word/test.json"]);
}

fn signed_with_script(seed: u8, version: &str) -> Bundle {
    use crate::backend::widget::signature::{test_keys, COMPUTER_USE_SKILL};
    let listed = manifest(version, &format!(r#","scripts":[{{"path":"scripts/run.py","sha256":"{}","purpose":"test","needs":["python"]}}]"#, sha256_hex(PY.as_bytes())));
    let mut files = base_files(&listed);
    files.push(("scripts/run.py", PY.to_string()));
    let unsigned = read(&files).unwrap();
    files.push(("skill.sig", test_keys::sig_file_in(&COMPUTER_USE_SKILL, seed, "acme.word", version, &unsigned.content_hash)));
    read(&files).unwrap()
}

#[test]
fn a_signature_is_checked_and_the_publisher_is_pinned_by_the_first_script_a_person_approves() {
    use crate::backend::widget::signature::{test_keys, SignatureState};
    let (b1, c) = (signed_with_script(1, "1.0.0"), catalog());
    c.skill_upsert_unique_global(&catalog_skill("s1", &b1.name)).unwrap();
    assert_eq!(check_signature(&c, &b1).unwrap(), Some(test_keys::public_key(1)));
    install_checked(&c, "s1", &b1, 1).unwrap();
    let app = c.skill_app_get("s1").unwrap().unwrap();
    assert_eq!((app.signature.state, app.signature.publisher.as_str()), (SignatureState::SignedNew, "acme"));
    assert!(app.signature.fingerprint.is_some() && app.signature.pinned.is_none());
    assert_eq!((app.scripts[0].purpose.as_str(), app.scripts[0].needs.clone()), ("test", vec!["python".to_string()]));
    let script = app.files.iter().find(|f| f.script).unwrap();
    assert_eq!(script.sha256, sha256_hex(PY.as_bytes()));

    // A signature that does not match its files is refused.
    let tampered = Bundle { content_hash: "0".repeat(64), ..b1.clone() };
    assert!(check_signature(&c, &tampered).unwrap_err().contains("doesn't match"));

    // Approving a script is what pins the publisher to the key; approving a file that is not a script does nothing.
    assert!(c.skill_publisher_pins().unwrap().is_empty());
    let sha = sha256_hex(PY.as_bytes());
    let approve = |id: &str, path: &str, sha: &str| c.skill_script_set_approved(id, path, ScriptApproval::Approve { reviewed_sha256: sha }, 5).unwrap();
    assert_eq!(approve("s1", "test.json", &sha), ScriptOutcome::NotAScript);
    assert_eq!(approve("s1", "scripts/run.py", &"0".repeat(64)), ScriptOutcome::Changed, "only the reviewed text is approved");
    assert!(c.skill_publisher_pins().unwrap().is_empty());
    assert_eq!(approve("s1", "scripts/run.py", &sha), ScriptOutcome::Done);
    assert_eq!(c.skill_publisher_pins().unwrap().get("acme"), Some(&test_keys::public_key(1)));
    assert_eq!(c.skill_app_get("s1").unwrap().unwrap().signature.state, SignatureState::Signed);

    // After that: another key, or none, is a different author, not an update.
    let other = signed_with_script(2, "1.1.0");
    assert!(check_signature(&c, &other).unwrap_err().contains("different author"));
    let unsigned = read(&base_files(&manifest("1.1.0", ""))).unwrap();
    let e = check_signature(&c, &unsigned).unwrap_err();
    assert!(e.contains("not signed") && e.contains("pinned"), "{e}");
    // The same key updating is fine, and its script starts unapproved again because the text changed.
    assert!(check_signature(&c, &signed_with_script(1, "1.1.0")).is_ok());
}

#[test]
fn only_an_approved_script_is_written_beside_skill_md_and_revoking_takes_it_back() {
    let (b, c) = (signed_with_script(1, "1.0.0"), catalog());
    c.skill_upsert_unique_global(&catalog_skill("s1", &b.name)).unwrap();
    install_checked(&c, "s1", &b, 1).unwrap();
    let skills = vec![agent_skill("s1", "acme-word")];
    let has_script = |c: &Store| sibling_files(&skills, c).iter().any(|f| f.filename.ends_with("scripts/run.py"));
    assert!(!has_script(&c));
    let sha = sha256_hex(PY.as_bytes());
    c.skill_script_set_approved("s1", "scripts/run.py", ScriptApproval::Approve { reviewed_sha256: &sha }, 2).unwrap();
    assert!(has_script(&c));
    c.skill_script_set_approved("s1", "scripts/run.py", ScriptApproval::Revoke, 3).unwrap();
    assert!(!has_script(&c));
}

#[test]
fn a_script_of_a_skill_signed_by_another_key_than_the_pin_cannot_be_approved() {
    // Two skills of one publisher, imported before any approval, so no pin existed to stop either.
    let (first, second, c) = (signed_with_script(1, "1.0.0"), signed_with_script(2, "1.0.0"), catalog());
    c.skill_upsert_unique_global(&catalog_skill("s1", "acme-word")).unwrap();
    c.skill_upsert_unique_global(&catalog_skill("s2", "acme-word-two")).unwrap();
    install_checked(&c, "s1", &first, 1).unwrap();
    install_checked(&c, "s2", &second, 1).unwrap();
    let sha = sha256_hex(PY.as_bytes());
    let approve = |id: &str| c.skill_script_set_approved(id, "scripts/run.py", ScriptApproval::Approve { reviewed_sha256: &sha }, 2).unwrap();
    assert_eq!(approve("s1"), ScriptOutcome::Done);
    assert_eq!(approve("s2"), ScriptOutcome::OtherAuthor);
    assert!(!c.skill_files("s2").unwrap().iter().any(|f| f.approved));
    assert_eq!(c.skill_publisher_pins().unwrap().get("acme"), Some(&crate::backend::widget::signature::test_keys::public_key(1)));
}

/// What the importer does: check the signature, then install with the key it returned.
fn install_checked(c: &Store, skill_id: &str, b: &Bundle, now_ms: i64) -> Result<(), crate::backend::storage::error::StoreError> {
    let signer = check_signature(c, b).expect("the bundle passes the signature check");
    install(c, skill_id, b, signer.as_deref(), now_ms)
}
