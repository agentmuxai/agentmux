// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::backend::computer_use_skill::read_folder;
use crate::backend::computer_use_skill::tests::{base_files, catalog, install_checked, manifest, read, signed_with_script};
use crate::backend::computer_use_skill::Bundle;

/// The catalog row a bundle's `SKILL.md` makes.
fn catalog_skill(b: &Bundle) -> Skill {
    Skill {
        id: "s1".into(),
        name: b.name.clone(),
        trigger: String::new(),
        skill_type: "agent-skill".into(),
        description: b.description.clone(),
        content: b.content.clone(),
        is_global: true,
        created_at: 1,
        updated_at: 1,
    }
}

fn installed(b: &Bundle) -> Store {
    let c = catalog();
    c.skill_upsert_unique_global(&catalog_skill(b)).unwrap();
    install_checked(&c, "s1", b, 1).unwrap();
    c
}

/// Unpack `bytes` as a `.skill` and read the folder it holds.
fn import(bytes: &[u8]) -> Result<Bundle, String> {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("in.skill");
    std::fs::write(&file, bytes).unwrap();
    let out = tmp.path().join("out");
    std::fs::create_dir_all(&out).unwrap();
    extract(&file, &out)?;
    let folder = std::fs::read_dir(&out).unwrap().flatten().next().unwrap().path();
    read_folder(&folder)
}

/// A zip made by hand, to hold what the exporter never writes.
fn raw_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    for (name, bytes) in entries {
        zip.start_file(*name, options).unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

fn refused(bytes: &[u8]) -> String {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("in.skill");
    std::fs::write(&file, bytes).unwrap();
    let out = tmp.path().join("out");
    std::fs::create_dir_all(&out).unwrap();
    let err = extract(&file, &out).unwrap_err();
    let written = std::fs::read_dir(&out).unwrap().count();
    assert_eq!(written, 0, "a refused file leaves nothing behind that the check should have stopped: {err}");
    err
}

#[test]
fn a_version_with_path_characters_is_neither_accepted_nor_put_in_a_file_name() {
    use crate::backend::computer_use_skill::tests::{base_files, manifest, read};
    for bad in ["1.0.0-x/../../name", "1.0.0+a\\b", "1.0.0-rc 1", "1.0.0-é"] {
        let err = read(&base_files(&manifest(bad, ""))).unwrap_err();
        assert!(err.contains("version"), "{bad}: {err}");
    }
    assert!(read(&base_files(&manifest("1.0.0-rc.1+build.5", ""))).is_ok());
    assert_eq!(file_name("acme-word", "1.0.0-x/../../name"), "acme-word-1.0.0-x.name.skill");
}

#[test]
fn a_signed_skill_keeps_its_signature_through_export_and_import() {
    let original = signed_with_script(1, "1.0.0");
    let exported = export(&installed(&original), "s1").unwrap();
    assert!(exported.signed);
    assert_eq!(exported.filename, "acme-word-1.0.0.skill");

    let back = import(&exported.bytes).unwrap();
    assert_eq!(back.content_hash, original.content_hash);
    assert_eq!(back.signature, original.signature);
    assert_eq!(back.skill_md, original.skill_md);
    let script = back.files.iter().find(|f| f.path == "scripts/run.py").unwrap();
    assert!(script.script, "a script is still a script, and arrives unapproved: nothing in the file says approved");
    // The importing machine checks the signature the same way it checks a folder's.
    let receiver = catalog();
    assert_eq!(
        crate::backend::computer_use_skill::check_signature(&receiver, &back).unwrap(),
        Some(crate::backend::widget::signature::test_keys::public_key(1))
    );
}

#[test]
fn a_skill_the_person_edited_is_exported_without_the_signature_it_no_longer_matches() {
    let original = signed_with_script(1, "1.0.0");
    let c = installed(&original);
    let mut edited = catalog_skill(&original);
    edited.content.push_str("\nOne more step.\n");
    c.skill_upsert_unique_global(&edited).unwrap();

    let exported = export(&c, "s1").unwrap();
    assert!(!exported.signed);
    let back = import(&exported.bytes).unwrap();
    assert!(back.signature.is_none());
    assert!(back.content.contains("One more step."));
    assert_ne!(back.content_hash, original.content_hash);
    assert_eq!(back.info.version, "1.0.0");
}

#[test]
fn a_description_with_quotes_colons_and_lines_survives_the_rebuilt_skill_md() {
    let b = read(&base_files(&manifest("1.0.0", ""))).unwrap();
    let c = installed(&b);
    let mut edited = catalog_skill(&b);
    edited.description = "Use when: \"quoting\" matters\nand lines do too".into();
    c.skill_upsert_unique_global(&edited).unwrap();
    let back = import(&export(&c, "s1").unwrap().bytes).unwrap();
    assert_eq!(back.description, edited.description);
}

#[test]
fn an_unsigned_skill_and_its_pass_records_travel_too() {
    let b = read(&base_files(&manifest("1.0.0", ""))).unwrap();
    let c = installed(&b);
    let record = crate::backend::storage::skill_app::VerifiedRecord {
        skill_version: "1.0.0".into(),
        app_version: "16.0".into(),
        platform: "windows".into(),
        date: "2026-11-06".into(),
        runner: "tester".into(),
        route: "api".into(),
        result: "pass".into(),
        notes: String::new(),
        evidence_sha256: "ab".repeat(32),
    };
    c.skill_verified_append("s1", &record, "local", 2).unwrap();
    let exported = export(&c, "s1").unwrap();
    assert!(!exported.signed);
    let back = import(&exported.bytes).unwrap();
    assert!(back.signature.is_none());
    assert_eq!(back.verified, vec![record]);
    assert_eq!(back.content_hash, b.content_hash, "the pass records are not part of the hash");
}

#[test]
fn a_skill_installed_before_its_original_was_kept_exports_unsigned() {
    let original = signed_with_script(1, "1.0.0");
    let c = catalog();
    c.skill_upsert_unique_global(&catalog_skill(&original)).unwrap();
    // The way an older build installed it: the files and the manifest, with no kept `SKILL.md` or signature.
    c.skill_app_set("s1", &original.info, &original.manifest_json, &original.content_hash, &original.files, 1).unwrap();
    let exported = export(&c, "s1").unwrap();
    assert!(!exported.signed);
    let back = import(&exported.bytes).unwrap();
    assert_eq!((back.description, back.content), (original.description, original.content));
}

#[test]
fn only_one_folder_of_safe_names_is_unpacked() {
    let md = b"---\nname: acme-word\ndescription: d\n---\nbody\n";
    assert!(refused(&raw_zip(&[("acme-word/../escape.md", md)])).contains("not a path inside"));
    assert!(refused(&raw_zip(&[("/abs/SKILL.md", md)])).contains("not a path inside"));
    assert!(refused(&raw_zip(&[("acme-word/SKILL.md", md), ("other/SKILL.md", md)])).contains("holds one folder"));
    assert!(refused(&raw_zip(&[("SKILL.md", md)])).contains("outside the skill's folder"));
    assert!(refused(&raw_zip(&[("acme-word/SKILL.md", md), ("acme-word/skill.md", md)])).contains("twice"));
    assert!(refused(b"not a zip at all").contains("not a .skill file"));
    assert!(refused(&raw_zip(&[])).contains("empty"));
}

#[test]
fn a_symbolic_link_entry_is_refused() {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    zip.add_symlink("acme-word/SKILL.md", "/etc/passwd", zip::write::SimpleFileOptions::default()).unwrap();
    assert!(refused(&zip.finish().unwrap().into_inner()).contains("symbolic link"));
}

#[test]
fn sizes_are_counted_as_the_files_are_read() {
    let big = vec![b'x'; (MAX_FILE_BYTES + 1) as usize];
    assert!(refused(&raw_zip(&[("acme-word/big.md", &big)])).contains("over 1 MB"));

    let chunk = vec![b'y'; MAX_FILE_BYTES as usize];
    let names: Vec<String> = (0..6).map(|i| format!("acme-word/f{i}.md")).collect();
    let entries: Vec<(&str, &[u8])> = names.iter().map(|n| (n.as_str(), chunk.as_slice())).collect();
    assert!(refused(&raw_zip(&entries)).contains("add up to over 5 MB"));

    let many: Vec<String> = (0..=MAX_ENTRIES).map(|i| format!("acme-word/f{i}.md")).collect();
    let entries: Vec<(&str, &[u8])> = many.iter().map(|n| (n.as_str(), b"x".as_slice())).collect();
    assert!(refused(&raw_zip(&entries)).contains("entries"));
}

#[test]
fn a_folder_that_breaks_a_skill_rule_is_refused_after_unpacking_like_any_folder() {
    let md = b"---\nname: acme-word\ndescription: d\n---\nbody\n";
    let bytes = raw_zip(&[("acme-word/SKILL.md", md), ("acme-word/computer-use.json", b"{\"schema\":\"computer-use-skill/v9\"}")]);
    assert!(import(&bytes).is_err());
}
