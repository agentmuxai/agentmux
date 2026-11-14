// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::backend::computer_use_skill::tests::{base_files, catalog, install_checked, manifest, read, signed_with_script};
use crate::backend::storage::skill_app::{ScriptApproval, ScriptOutcome};
use crate::backend::widget::signature::test_keys;
use ed25519_dalek::{Signer, SigningKey};

fn skill_row(b: &Bundle) -> Skill {
    Skill {
        id: "x".into(),
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

/// The zip the catalog would serve for `b`: the same folder layout a `.skill` file has.
fn zip_of(b: &Bundle) -> Vec<u8> {
    let c = catalog();
    c.skill_upsert_unique_global(&skill_row(b)).unwrap();
    install_checked(&c, "x", b, 1).unwrap();
    skill_file::export(&c, "x").unwrap().bytes
}

fn entry_for(b: &Bundle, zip: &[u8], seed: u8) -> CatalogEntry {
    CatalogEntry {
        id: format!("{}.{}", b.info.publisher, b.info.id),
        name: b.info.name.clone(),
        version: b.info.version.clone(),
        description: b.description.clone(),
        publisher: b.info.publisher.clone(),
        app: CatalogApp::default(),
        platforms: vec!["windows".into()],
        routes: vec!["api".into()],
        has_scripts: false,
        mcp: None,
        hash: b.content_hash.clone(),
        publisher_key: test_keys::public_key(seed),
        verification: vec![],
        zip: "https://catalog.test/acme-word-1.0.0.zip".into(),
        zip_sha256: sha256_hex(zip),
    }
}

fn signed_index(seed: u8, entries: &[CatalogEntry]) -> (Vec<u8>, String) {
    let raw = serde_json::to_vec(&serde_json::json!({"catalogVersion": 1, "generated": "x", "skills": entries})).unwrap();
    let sig = SigningKey::from_bytes(&[seed; 32]).sign(&raw);
    (raw, base64::engine::general_purpose::STANDARD.encode(sig.to_bytes()))
}

const URL: &str = "https://catalog.test/index.json";

#[test]
fn an_index_is_accepted_only_when_the_catalog_key_signed_it_and_it_names_its_own_origin() {
    let b = signed_with_script(1, "1.0.0");
    let zip = zip_of(&b);
    let good = entry_for(&b, &zip, 1);
    let (raw, sig) = signed_index(9, std::slice::from_ref(&good));
    let keys = [test_keys::public_key(9)];
    let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
    assert_eq!(verify_index(&raw, &sig, &keys, URL).unwrap(), vec![good.clone()]);

    // Changed after signing, signed by another key, or a signature that is not one.
    let mut tampered = raw.clone();
    tampered.extend_from_slice(b" ");
    assert!(verify_index(&tampered, &sig, &keys, URL).unwrap_err().contains("doesn't verify"));
    let other = [test_keys::public_key(8)];
    let other: Vec<&str> = other.iter().map(String::as_str).collect();
    assert!(verify_index(&raw, &sig, &other, URL).unwrap_err().contains("doesn't verify"));
    assert!(verify_index(&raw, "not base64", &keys, URL).unwrap_err().contains("64 bytes"));

    // An entry that points off the catalog's origin, or is not <publisher>.<id>.
    let off = CatalogEntry { zip: "https://elsewhere.test/a.zip".into(), ..good.clone() };
    let (raw, sig) = signed_index(9, &[off]);
    assert!(verify_index(&raw, &sig, &keys, URL).unwrap_err().contains("isn't served from the catalog itself"));
    let odd = CatalogEntry { id: "acme/../word".into(), ..good.clone() };
    let (raw, sig) = signed_index(9, &[odd]);
    assert!(verify_index(&raw, &sig, &keys, URL).unwrap_err().contains("isn't a <publisher>.<id> skill"));
    let mismatch = CatalogEntry { publisher: "other".into(), ..good };
    let (raw, sig) = signed_index(9, &[mismatch]);
    assert!(verify_index(&raw, &sig, &keys, URL).is_err());
    assert!(verify_index(b"{\"catalogVersion\":2,\"skills\":[]}", &sig, &keys, URL).is_err());
}

#[test]
fn the_pinned_key_is_a_real_key() {
    assert_eq!(PACK_CATALOG_KEYS.len(), 1);
    let raw = base64::engine::general_purpose::STANDARD.decode(PACK_CATALOG_KEYS[0]).unwrap();
    assert!(<[u8; 32]>::try_from(raw).ok().and_then(|k| ed25519_dalek::VerifyingKey::from_bytes(&k).ok()).is_some());
    assert_eq!(fingerprint(PACK_CATALOG_KEYS[0]), "DU3S-53XD-CITH-4F4Q");
}

#[test]
fn an_installed_skill_is_looked_up_by_its_own_executables_for_this_platform_only() {
    use crate::backend::app_version::Found;
    let b = signed_with_script(1, "1.0.0");
    let c = catalog();
    c.skill_upsert_unique_global(&skill_row(&b)).unwrap();
    install_checked(&c, "x", &b, 1).unwrap();
    let installed = c.skill_app_map().unwrap();

    let asked = std::cell::RefCell::new(Vec::new());
    let here = installed_here(&installed, "windows", |names| {
        asked.borrow_mut().push(names.to_vec());
        Some(Found { version: Some("16.0.18227".into()) })
    });
    assert_eq!(here, vec![AppSkillHere { skill_id: "x".into(), found: true, version: Some("16.0.18227".into()) }]);
    assert_eq!(*asked.borrow(), vec![vec!["WINWORD.EXE".to_string()]], "the skill's own executable names");

    assert_eq!(installed_here(&installed, "windows", |_| Some(Found { version: None }))[0], AppSkillHere { skill_id: "x".into(), found: true, version: None });
    assert_eq!(installed_here(&installed, "windows", |_| None)[0], AppSkillHere { skill_id: "x".into(), found: false, version: None });
    let never = |_: &[String]| -> Option<Found> { panic!("a skill with no executable for this platform is not looked up") };
    assert!(installed_here(&installed, "macos", never).is_empty());
    assert!(installed_here(&BTreeMap::new(), "windows", never).is_empty());
}

#[test]
fn browse_adds_the_version_of_the_app_found_here_by_the_executables_listed_for_this_platform() {
    use crate::backend::app_version::Found;
    let b = signed_with_script(1, "1.0.0");
    let zip = zip_of(&b);
    let with = |exes: &[(&str, &[&str])]| {
        let mut e = entry_for(&b, &zip, 1);
        e.app.executables = exes.iter().map(|(p, l)| (p.to_string(), l.iter().map(|x| x.to_string()).collect())).collect();
        e
    };
    let mut listed = items(
        vec![with(&[("windows", &["Live.exe"]), ("macos", &["Live"])]), with(&[("windows", &[])]), with(&[("macos", &["Live"])]), with(&[])],
        &BTreeMap::new(),
    );
    assert!(listed.iter().all(|i| !i.app_found && i.app_version.is_none()), "nothing is looked up by items()");
    let asked = std::cell::RefCell::new(Vec::new());
    add_app_versions(&mut listed, "windows", |names| {
        asked.borrow_mut().push(names.to_vec());
        Some(Found { version: Some("12.2".into()) })
    });
    assert_eq!(*asked.borrow(), vec![vec!["Live.exe".to_string()]], "only the entry that lists a Windows executable is looked up, with those names");
    assert_eq!((listed[0].app_found, listed[0].app_version.as_deref()), (true, Some("12.2")));
    assert!(listed[1..].iter().all(|i| !i.app_found && i.app_version.is_none()));

    // Found but saying no version, and not found at all.
    let mut one = items(vec![with(&[("linux", &["blender"])])], &BTreeMap::new());
    add_app_versions(&mut one, "linux", |_| Some(Found { version: None }));
    assert_eq!((one[0].app_found, one[0].app_version.clone()), (true, None));
    let mut none = items(vec![with(&[("linux", &["blender"])])], &BTreeMap::new());
    add_app_versions(&mut none, "linux", |_| None);
    assert_eq!((none[0].app_found, none[0].app_version.clone()), (false, None));
    assert!(["windows", "macos", "linux"].contains(&this_platform()));
}

#[test]
fn browse_says_what_is_installed_and_whether_the_catalog_is_ahead() {
    let b = signed_with_script(1, "1.0.0");
    let zip = zip_of(&b);
    let entry = entry_for(&b, &zip, 1);

    let none = BTreeMap::new();
    let listed = items(vec![entry.clone()], &none);
    assert_eq!((listed[0].state.as_str(), listed[0].badge.as_str()), ("not_installed", "community"));
    assert_eq!(listed[0].fingerprint, fingerprint(&entry.publisher_key));

    let c = catalog();
    c.skill_upsert_unique_global(&skill_row(&b)).unwrap();
    install_checked(&c, "x", &b, 1).unwrap();
    let here = c.skill_app_map().unwrap();
    assert_eq!(items(vec![entry.clone()], &here)[0].state, "changed", "the same files added another way are not as the catalog lists them");
    install_download(&c, &zip, &entry, 2).unwrap();
    let here = c.skill_app_map().unwrap();
    assert_eq!(items(vec![entry.clone()], &here)[0].state, "current");
    let newer = CatalogEntry { version: "1.1.0".into(), hash: "f".repeat(64), ..entry.clone() };
    let item = &items(vec![newer], &here)[0].clone();
    assert_eq!((item.state.as_str(), item.installed_version.as_deref(), item.skill_id.as_deref()), ("update", Some("1.0.0"), Some("x")));
    let older = CatalogEntry { version: "0.9.0".into(), hash: "f".repeat(64), ..entry.clone() };
    assert_eq!(items(vec![older], &here)[0].state, "newer_here");
    let changed = CatalogEntry { hash: "f".repeat(64), ..entry.clone() };
    assert_eq!(items(vec![changed], &here)[0].state, "changed");
    // "1.10.0" is ahead of "1.9.0": versions compare by number, not as text.
    assert_eq!(version_cmp("1.10.0", "1.9.0"), std::cmp::Ordering::Greater);
    // A missing number is 0, a pre-release is before its release, and a build tag changes nothing.
    use std::cmp::Ordering::{Equal, Greater, Less};
    assert_eq!(version_cmp("1.0", "1.0.0"), Equal);
    assert_eq!(version_cmp("1.0.0-rc1", "1.0.0"), Less);
    assert_eq!(version_cmp("1.0.0", "1.0.0-rc1"), Greater);
    assert_eq!(version_cmp("1.0.0-rc.2", "1.0.0-rc.10"), Less);
    assert_eq!(version_cmp("1.0.0-1", "1.0.0-alpha"), Less);
    assert_eq!(version_cmp("1.0.0-rc", "1.0.0-rc.1"), Less);
    assert_eq!(version_cmp("1.0.0+a", "1.0.0+b"), Equal);
    // So a catalog 1.0.0 is an update over an installed 1.0.0-rc1 that has other files.
    let rc = signed_with_script(1, "1.0.0-rc1");
    let c2 = catalog();
    c2.skill_upsert_unique_global(&skill_row(&rc)).unwrap();
    install_checked(&c2, "x", &rc, 1).unwrap();
    let release = CatalogEntry { version: "1.0.0".into(), hash: "f".repeat(64), ..entry_for(&rc, &zip, 1) };
    assert_eq!(items(vec![release], &c2.skill_app_map().unwrap())[0].state, "update");

    let agentmux = CatalogEntry { publisher_key: PACK_CATALOG_KEYS[0].to_string(), ..entry };
    assert_eq!(items(vec![agentmux], &none)[0].badge, "agentmux");
}

#[test]
fn a_download_is_checked_against_the_catalog_entry_three_ways() {
    let b = signed_with_script(1, "1.0.0");
    let zip = zip_of(&b);
    let entry = entry_for(&b, &zip, 1);
    let c = catalog();
    let (got, key) = check_download(&c, &zip, &entry).unwrap();
    assert_eq!((got.content_hash, key), (b.content_hash.clone(), test_keys::public_key(1)));

    // 1. The zip is not the one the index describes.
    let mut flipped = zip.clone();
    let last = flipped.len() - 1;
    flipped[last] ^= 1;
    assert!(check_download(&c, &flipped, &entry).unwrap_err().contains("doesn't match the catalog"));
    // 2. The files are not the ones whose hash the index lists.
    let wrong_hash = CatalogEntry { hash: "0".repeat(64), ..entry.clone() };
    assert!(check_download(&c, &zip, &wrong_hash).unwrap_err().contains("files don't match"));
    // 3. The author's signature is not by the key the index lists.
    let wrong_key = CatalogEntry { publisher_key: test_keys::public_key(2), ..entry.clone() };
    assert!(check_download(&c, &zip, &wrong_key).unwrap_err().contains("isn't signed by the author"));
    // The id and version the index says are the ones in the manifest.
    let wrong_version = CatalogEntry { version: "9.9.9".into(), ..entry.clone() };
    assert!(check_download(&c, &zip, &wrong_version).unwrap_err().contains("manifest doesn't match"));

    // Unsigned, and an entry that lists a key for it anyway.
    let unsigned = read(&base_files(&manifest("1.0.0", ""))).unwrap();
    let unsigned_zip = zip_of(&unsigned);
    let claims = entry_for(&unsigned, &unsigned_zip, 1);
    assert!(check_download(&c, &unsigned_zip, &claims).unwrap_err().contains("isn't signed by the author"));
}

#[test]
fn a_catalog_skill_installs_unapproved_and_an_update_replaces_it_in_place() {
    let v1 = signed_with_script(1, "1.0.0");
    let zip1 = zip_of(&v1);
    let c = catalog();
    let first = install_download(&c, &zip1, &entry_for(&v1, &zip1, 1), 10).unwrap();
    assert!(!first.updated);
    let app = c.skill_app_get(&first.skill_id).unwrap().unwrap();
    assert_eq!((app.info.version.as_str(), app.signature.state), ("1.0.0", crate::backend::widget::signature::SignatureState::SignedNew));
    assert!(app.files.iter().filter(|f| f.script).all(|f| !f.approved), "a script arrives unapproved");
    let skill = c.skill_get(&first.skill_id).unwrap().unwrap();
    assert!(skill.is_global);
    assert!(c.skill_publisher_pins().unwrap().is_empty(), "installing pins nothing: approving a script does");

    // The person approves the script; an update that leaves its text alone keeps that, and keeps the skill's id.
    let sha = app.files.iter().find(|f| f.script).unwrap().sha256.clone();
    let done = c.skill_script_set_approved(&first.skill_id, "scripts/run.py", ScriptApproval::Approve { reviewed_sha256: &sha }, 11).unwrap();
    assert_eq!(done, ScriptOutcome::Done);
    let v2 = signed_with_script(1, "1.1.0");
    let zip2 = zip_of(&v2);
    let second = install_download(&c, &zip2, &entry_for(&v2, &zip2, 1), 12).unwrap();
    assert!(second.updated);
    assert_eq!(second.skill_id, first.skill_id);
    let app = c.skill_app_get(&second.skill_id).unwrap().unwrap();
    assert_eq!(app.info.version, "1.1.0");
    assert!(app.files.iter().find(|f| f.script).unwrap().approved);
    assert_eq!(c.skill_app_map().unwrap().len(), 1);
}

#[test]
fn a_skill_is_from_the_catalog_only_while_it_is_still_what_the_catalog_listed() {
    let v1 = signed_with_script(1, "1.0.0");
    let zip1 = zip_of(&v1);
    let c = catalog();

    // A skill added another way (here, a folder import) was never from the catalog.
    c.skill_upsert_unique_global(&skill_row(&v1)).unwrap();
    install_checked(&c, "x", &v1, 1).unwrap();
    assert!(!c.skill_app_get("x").unwrap().unwrap().from_catalog);

    // Installed from the catalog it is, in the one skill and in the list.
    let first = install_download(&c, &zip1, &entry_for(&v1, &zip1, 1), 10).unwrap();
    assert!(c.skill_app_get(&first.skill_id).unwrap().unwrap().from_catalog);
    assert!(c.skill_app_map().unwrap().get(&first.skill_id).unwrap().from_catalog);

    // An edit of its text in Memory ends it: the content hash does not follow an edit, the text does.
    let mut edited = c.skill_get(&first.skill_id).unwrap().unwrap();
    edited.content.push_str("
My own note.");
    c.skill_upsert_unique_global(&edited).unwrap();
    assert!(!c.skill_app_get(&first.skill_id).unwrap().unwrap().from_catalog);
    let entry = entry_for(&v1, &zip1, 1);
    assert_eq!(items(vec![entry.clone()], &c.skill_app_map().unwrap())[0].state, "changed", "Browse offers the catalog's version back");

    // Installing the catalog's version again puts it back.
    let again = install_download(&c, &zip1, &entry_for(&v1, &zip1, 1), 11).unwrap();
    assert_eq!(again.skill_id, first.skill_id);
    assert!(c.skill_app_get(&first.skill_id).unwrap().unwrap().from_catalog);
    assert_eq!(items(vec![entry], &c.skill_app_map().unwrap())[0].state, "current");

    // Different files than the catalog listed, or another key, are not: the record holds what the catalog said.
    c.skill_catalog_source_set(&first.skill_id, &"f".repeat(64), &entry_for(&v1, &zip1, 1).publisher_key).unwrap();
    assert!(!c.skill_app_get(&first.skill_id).unwrap().unwrap().from_catalog, "other files");
    let app = c.skill_app_get(&first.skill_id).unwrap().unwrap();
    c.skill_catalog_source_set(&first.skill_id, &app.content_hash, "another-key").unwrap();
    assert!(!c.skill_app_get(&first.skill_id).unwrap().unwrap().from_catalog, "another key");

    // Deleting the skill drops the record with it.
    c.skill_catalog_forget(&first.skill_id).unwrap();
    assert!(c.skill_app_get(&first.skill_id).unwrap().is_none());
}

#[test]
fn a_skill_from_another_key_than_the_publishers_pin_is_refused() {
    let v1 = signed_with_script(1, "1.0.0");
    let zip1 = zip_of(&v1);
    let c = catalog();
    let first = install_download(&c, &zip1, &entry_for(&v1, &zip1, 1), 10).unwrap();
    let sha = c.skill_app_get(&first.skill_id).unwrap().unwrap().files.iter().find(|f| f.script).unwrap().sha256.clone();
    c.skill_script_set_approved(&first.skill_id, "scripts/run.py", ScriptApproval::Approve { reviewed_sha256: &sha }, 11).unwrap();

    let imposter = signed_with_script(2, "1.1.0");
    let zip = zip_of(&imposter);
    let err = install_download(&c, &zip, &entry_for(&imposter, &zip, 2), 12).unwrap_err();
    assert!(err.contains("different author"), "{err}");
    assert_eq!(c.skill_app_get(&first.skill_id).unwrap().unwrap().info.version, "1.0.0");
}
