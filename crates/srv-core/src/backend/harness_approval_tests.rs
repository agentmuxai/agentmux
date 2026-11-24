// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::backend::harness_manifest::{Auth, MANIFEST_VERSION};
use std::collections::BTreeMap;

fn manifest(command: &str) -> HarnessManifest {
    HarnessManifest {
        manifest_version: MANIFEST_VERSION,
        id: "custom-demo".to_string(),
        name: "Demo".to_string(),
        icon: None,
        protocol: "acp".to_string(),
        command: command.to_string(),
        args: vec![],
        env: BTreeMap::new(),
        auth: Auth::None,
        startup_instructions: None,
        vendors: vec![],
        base_url_env_var: None,
        install: Install::Path,
    }
}

/// A folder holding a fake program and the manifest naming it by absolute path.
struct Fixture {
    dir: tempfile::TempDir,
    binary: PathBuf,
    manifest: HarnessManifest,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let binary = dir.path().join(if cfg!(windows) { "demo-acp.exe" } else { "demo-acp" });
    std::fs::write(&binary, b"program v1").unwrap();
    let manifest = manifest(&std::fs::canonicalize(&binary).unwrap().to_string_lossy());
    Fixture { dir, binary, manifest }
}

fn approve(f: &Fixture) -> Approval {
    let real = resolve_command(&f.manifest).unwrap();
    let approval = approval_for(&f.manifest, &real, "someone", 1_000).unwrap();
    write(f.dir.path(), &approval).unwrap();
    approval
}

#[test]
fn a_harness_with_no_approval_does_not_start() {
    let f = fixture();
    assert_eq!(check(&f.manifest, f.dir.path()), Err(Refusal::NotApproved));
    assert!(Refusal::NotApproved.to_string().contains("not been approved"));
}

#[test]
fn an_approved_harness_starts_and_the_approval_names_who_and_what() {
    let f = fixture();
    let approval = approve(&f);
    assert_eq!(check(&f.manifest, f.dir.path()), Ok(approval.clone()));
    assert_eq!((approval.approved_by.as_str(), approval.approved_at_ms), ("someone", 1_000));
    assert_eq!(approval.binary_sha256, sha256_file(&f.binary).unwrap());
    assert_eq!(read(f.dir.path()), Some(approval));
}

#[test]
fn a_changed_manifest_asks_again() {
    let f = fixture();
    approve(&f);
    let mut edited = f.manifest.clone();
    edited.args.push("--verbose".to_string());
    assert_eq!(check(&edited, f.dir.path()), Err(Refusal::ManifestChanged));
}

#[test]
fn a_swapped_or_updated_program_asks_again_and_names_both_hashes() {
    let f = fixture();
    let approval = approve(&f);
    std::fs::write(&f.binary, b"program v2").unwrap();
    let refusal = check(&f.manifest, f.dir.path()).unwrap_err();
    let Refusal::BinaryChanged { approved, found } = &refusal else { panic!("{refusal:?}") };
    assert_eq!(approved, &approval.binary_sha256);
    assert_ne!(found, approved);
    let text = refusal.to_string();
    assert!(text.contains(&approved[..12]) && text.contains(&found[..12]) && text.contains("approve it again"), "{text}");
    // Putting the approved bytes back is the approved program again.
    std::fs::write(&f.binary, b"program v1").unwrap();
    assert!(check(&f.manifest, f.dir.path()).is_ok());
}

#[test]
fn a_program_that_is_gone_does_not_start() {
    let f = fixture();
    approve(&f);
    std::fs::remove_file(&f.binary).unwrap();
    assert!(matches!(check(&f.manifest, f.dir.path()), Err(Refusal::BinaryMissing(_))));
}

#[test]
fn an_unreadable_or_foreign_approval_file_is_no_approval() {
    let f = fixture();
    std::fs::write(f.dir.path().join(APPROVAL_FILE), "{ not json").unwrap();
    assert_eq!(check(&f.manifest, f.dir.path()), Err(Refusal::NotApproved));
    // A field this build does not know is refused, not read past.
    std::fs::write(f.dir.path().join(APPROVAL_FILE), r#"{"manifest_hash":"a","binary_path":"b","binary_sha256":"c","approved_at_ms":1,"approved_by":"x","extra":1}"#).unwrap();
    assert_eq!(read(f.dir.path()), None);
}

#[test]
fn clearing_an_approval_withdraws_it_and_clearing_none_is_fine() {
    let f = fixture();
    approve(&f);
    clear(f.dir.path()).unwrap();
    assert_eq!(check(&f.manifest, f.dir.path()), Err(Refusal::NotApproved));
    clear(f.dir.path()).unwrap();
}

#[test]
fn writing_an_approval_leaves_no_temporary_file() {
    let f = fixture();
    approve(&f);
    let names: Vec<String> = std::fs::read_dir(f.dir.path()).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    assert!(names.iter().any(|n| n == APPROVAL_FILE) && !names.iter().any(|n| n.ends_with(".tmp")), "{names:?}");
}

#[test]
fn a_command_resolves_to_the_real_file_and_only_for_a_path_install() {
    let f = fixture();
    assert_eq!(resolve_command(&f.manifest).unwrap(), std::fs::canonicalize(&f.binary).unwrap());
    let missing = manifest(&f.dir.path().join("nope").to_string_lossy());
    assert!(resolve_command(&missing).is_err());
    let folder = manifest(&f.dir.path().to_string_lossy());
    assert!(resolve_command(&folder).unwrap_err().contains("not a file"));
    assert!(resolve_command(&manifest("no-such-program-anywhere-9f3a")).unwrap_err().contains("not on the PATH"));
    let mut npm = f.manifest.clone();
    npm.install = Install::Npm { package: "demo".to_string(), version: "1.0.0".to_string() };
    assert!(resolve_command(&npm).unwrap_err().contains("after its install"));
}

#[test]
fn a_large_file_hashes_in_blocks_to_the_same_digest() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("big");
    let bytes = vec![7u8; 200 * 1024 + 13];
    std::fs::write(&path, &bytes).unwrap();
    assert_eq!(sha256_file(&path).unwrap(), hex::encode(Sha256::digest(&bytes)));
}
