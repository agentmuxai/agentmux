// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::backend::harness_manifest::{DownloadAsset, MANIFEST_VERSION};
use crate::backend::providers::{get_provider, ControllerType, Tier};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn manifest(id: &str) -> HarnessManifest {
    HarnessManifest {
        manifest_version: MANIFEST_VERSION,
        id: id.to_string(),
        name: "Demo".to_string(),
        icon: None,
        protocol: "acp".to_string(),
        command: "demo-acp".to_string(),
        args: vec!["--stdio".to_string()],
        env: BTreeMap::from([("DEMO_FLAG".to_string(), "1".to_string())]),
        auth: Auth::ConfigDir { env_var: "DEMO_HOME".to_string() },
        startup_instructions: Some("AGENTS.md".to_string()),
        vendors: vec!["openai".to_string()],
        base_url_env_var: Some("DEMO_URL".to_string()),
        install: Install::Npm { package: "demo-acp".to_string(), version: "1.2.3".to_string() },
    }
}

#[test]
fn a_manifest_becomes_an_acp_config_with_the_generic_defaults() {
    let mut layer = CustomLayer::new(4, 8);
    let (c, changed) = layer.register(&manifest("custom-demo")).unwrap();
    assert!(changed);
    assert_eq!((c.id, c.tier, c.controller_type), ("custom-demo", Tier::Custom, ControllerType::Acp));
    assert_eq!((c.cli_command, c.launch_args), ("demo-acp", &["--stdio"][..]));
    assert_eq!(c.auth_config_dir_env_var, "DEMO_HOME");
    assert_eq!((c.auth_dir_name, c.npm_package, c.pinned_version), ("custom-demo", "demo-acp", "1.2.3"));
    assert_eq!(c.auth_extra_env, &[("DEMO_FLAG", "1")]);
    assert_eq!(c.startup_instructions_filename, Some("AGENTS.md"));
    assert_eq!(c.native_instruction_sources, &["AGENTS.md"]);
    assert_eq!((c.base_url_env_var, c.supported_vendors), (Some("DEMO_URL"), &["openai"][..]));
    assert!(c.history_native_subdir.is_none() && c.resume_flag.is_none() && c.app_server.is_none());
    assert_eq!((c.session_id_field, c.styled_output_format), ("sessionId", "acp"));
}

#[test]
fn a_key_login_does_not_name_a_config_dir_and_a_download_has_no_npm_package() {
    let mut m = manifest("custom-key");
    m.auth = Auth::Key { env_var: "DEMO_KEY".to_string() };
    m.install = Install::Download {
        urls: BTreeMap::from([(
            "linux-x86_64".to_string(),
            DownloadAsset { url: "https://example.com/d".to_string(), sha256: "ab".repeat(32) },
        )]),
    };
    let (c, _) = CustomLayer::new(4, 8).register(&m).unwrap();
    assert_eq!((c.auth_config_dir_env_var, c.npm_package, c.pinned_version), ("", "", ""));
}

#[test]
fn reading_the_same_manifest_again_leaks_nothing_and_changes_nothing() {
    let mut layer = CustomLayer::new(4, 8);
    let (first, _) = layer.register(&manifest("custom-demo")).unwrap();
    let (second, changed) = layer.register(&manifest("custom-demo")).unwrap();
    assert!(std::ptr::eq(first, second), "the same content reuses the same config");
    assert!(!changed);
    assert_eq!(layer.leaked.len(), 1);
}

#[test]
fn an_edited_manifest_replaces_the_harness_and_reports_the_change() {
    let mut layer = CustomLayer::new(4, 8);
    let (first, _) = layer.register(&manifest("custom-demo")).unwrap();
    let hash_first = layer.hash_of("custom-demo").unwrap().to_string();
    let mut edited = manifest("custom-demo");
    edited.args.push("--verbose".to_string());
    let (second, changed) = layer.register(&edited).unwrap();
    assert!(changed && !std::ptr::eq(first, second));
    assert_eq!(second.launch_args, &["--stdio", "--verbose"]);
    assert_ne!(layer.hash_of("custom-demo").unwrap(), hash_first);
    assert!(std::ptr::eq(layer.get("custom-demo").unwrap(), second));
    assert_eq!(layer.all().len(), 1);
}

#[test]
fn at_most_the_live_cap_are_registered_and_a_replacement_is_not_a_new_one() {
    let mut layer = CustomLayer::new(2, 8);
    layer.register(&manifest("custom-a")).unwrap();
    layer.register(&manifest("custom-b")).unwrap();
    assert_eq!(layer.register(&manifest("custom-c")).unwrap_err(), RegisterError::TooManyLive);
    let mut edited = manifest("custom-b");
    edited.args.clear();
    assert!(layer.register(&edited).is_ok(), "replacing a live id needs no free slot");
    assert!(layer.remove("custom-a"));
    assert!(layer.register(&manifest("custom-c")).is_ok(), "a removal frees a slot");
}

#[test]
fn the_leak_cap_bounds_distinct_manifests_not_rereads() {
    let mut layer = CustomLayer::new(8, 2);
    let mut m = manifest("custom-demo");
    layer.register(&m).unwrap();
    m.args = vec!["a".to_string()];
    layer.register(&m).unwrap();
    layer.register(&m).expect("a reread of known content leaks nothing");
    m.args = vec!["b".to_string()];
    assert_eq!(layer.register(&m).unwrap_err(), RegisterError::TooManyRevisions);
    assert!(layer.get("custom-demo").is_some(), "a refused registration leaves the live one alone");
}

#[test]
fn a_removed_harness_cannot_be_looked_up_and_a_broken_manifest_is_never_registered() {
    let mut layer = CustomLayer::new(4, 8);
    layer.register(&manifest("custom-demo")).unwrap();
    assert!(layer.remove("custom-demo") && !layer.remove("custom-demo"));
    assert!(layer.get("custom-demo").is_none());
    let mut broken = manifest("claude");
    assert!(matches!(layer.register(&broken), Err(RegisterError::Manifest(e)) if e.field == "id"));
    broken.id = "custom-ok".to_string();
    broken.protocol = "msp".to_string();
    assert!(matches!(layer.register(&broken), Err(RegisterError::Manifest(e)) if e.field == "protocol"));
    assert!(layer.all().is_empty());
}

#[test]
fn a_custom_id_never_shadows_a_built_in_in_the_registry() {
    let id = "custom-registry-lookup";
    assert!(get_provider(id).is_none());
    register_custom(&manifest(id)).unwrap();
    assert_eq!(get_provider(id).map(|p| p.id), Some(id));
    // Built-ins and aliases still answer as before.
    assert_eq!(get_provider("claude").map(|p| p.id), Some("claude"));
    assert_eq!(get_provider("claude-code").map(|p| p.id), Some("claude"));
    assert!(all_custom_providers().iter().any(|p| p.id == id));
    assert!(crate::backend::providers::all_providers().all(|p| p.id != id), "the built-in list stays the built-ins");
    assert!(remove_custom(id));
    assert!(get_provider(id).is_none());
    // A name without the prefix is never answered by the custom layer.
    assert!(get_custom_provider("claude").is_none());
}

#[test]
fn a_directory_of_harnesses_loads_what_is_valid_and_reports_the_rest() {
    let dir = tempfile::tempdir().unwrap();
    let write = |folder: &str, id: &str| {
        let path = dir.path().join(folder);
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join(MANIFEST_FILE), serde_json::to_string(&manifest(id)).unwrap()).unwrap();
        path
    };
    write("custom-good", "custom-good");
    write("custom-misnamed", "custom-other");
    let big = write("custom-big", "custom-big");
    std::fs::write(big.join("pad.bin"), vec![0u8; MAX_FOLDER_BYTES as usize + 1]).unwrap();
    let broken = dir.path().join("custom-broken");
    std::fs::create_dir_all(&broken).unwrap();
    std::fs::write(broken.join(MANIFEST_FILE), "{ nope").unwrap();
    std::fs::create_dir_all(dir.path().join("custom-empty")).unwrap();
    let icon = write("custom-icon", "custom-icon");
    let mut with_icon = manifest("custom-icon");
    with_icon.icon = Some("logo.png".to_string());
    std::fs::write(icon.join(MANIFEST_FILE), serde_json::to_string(&with_icon).unwrap()).unwrap();
    std::fs::write(icon.join("logo.png"), vec![0u8; MAX_ICON_BYTES as usize + 1]).unwrap();

    let mut layer = CustomLayer::new(8, 8);
    let report = layer.load_dir(dir.path());
    assert_eq!(report.loaded, ["custom-good"]);
    let refused: Vec<&str> = report.refused.iter().map(|(f, _)| f.as_str()).collect();
    assert_eq!(refused, ["custom-big", "custom-broken", "custom-empty", "custom-icon", "custom-misnamed"]);
    let reason = |folder: &str| report.refused.iter().find(|(f, _)| f == folder).unwrap().1.clone();
    assert!(reason("custom-misnamed").contains("custom-other"), "{}", reason("custom-misnamed"));
    assert!(reason("custom-icon").contains("logo.png"), "{}", reason("custom-icon"));
    assert!(layer.get("custom-good").is_some() && layer.get("custom-misnamed").is_none());
    assert!(CustomLayer::new(1, 1).load_dir(&dir.path().join("missing")).loaded.is_empty());
}

/// A harness folder read from disk whose manifest names a real file as its program.
fn on_disk(id: &str) -> (tempfile::TempDir, CustomLayer, PathBuf, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let binary = root.path().join(if cfg!(windows) { "program.exe" } else { "program" });
    std::fs::write(&binary, b"program v1").unwrap();
    let folder = root.path().join("harnesses").join(id);
    std::fs::create_dir_all(&folder).unwrap();
    let mut m = manifest(id);
    m.command = std::fs::canonicalize(&binary).unwrap().to_string_lossy().into_owned();
    m.install = Install::Path;
    std::fs::write(folder.join(MANIFEST_FILE), serde_json::to_string(&m).unwrap()).unwrap();
    let mut layer = CustomLayer::new(4, 8);
    assert_eq!(layer.load_dir(&root.path().join("harnesses")).loaded, [id]);
    (root, layer, folder, binary)
}

fn approve_on_disk(layer: &CustomLayer, folder: &Path, id: &str) -> super::super::harness_approval::Approval {
    use super::super::harness_approval as approval;
    let entry = &layer.live[id];
    let real = approval::resolve_command(&entry.manifest).unwrap();
    let a = approval::approval_for(&entry.manifest, &real, "someone", 1).unwrap();
    approval::write(folder, &a).unwrap();
    a
}

#[test]
fn a_start_runs_only_the_approved_program_and_only_while_the_approval_holds() {
    let (_root, layer, folder, binary) = on_disk("custom-gate");
    // Not a custom harness here: nothing to refuse, the block's own command runs.
    assert_eq!(layer.approved_command("claude"), Ok(None));
    // Registered and not approved: refused.
    assert!(layer.approved_command("custom-gate").unwrap_err().contains("not been approved"));
    // Approved: the approved program, as the absolute path it was approved at.
    let approval = approve_on_disk(&layer, &folder, "custom-gate");
    assert_eq!(layer.approved_command("custom-gate"), Ok(Some(approval.binary_path.clone())));
    // The program swapped: refused again, until it is approved again.
    std::fs::write(&binary, b"program v2").unwrap();
    assert!(layer.approved_command("custom-gate").unwrap_err().contains("changed since it was approved"));
    approve_on_disk(&layer, &folder, "custom-gate");
    assert!(layer.approved_command("custom-gate").unwrap().is_some());
}

#[test]
fn a_harness_registered_in_memory_has_no_folder_and_so_no_approval() {
    let mut layer = CustomLayer::new(4, 8);
    layer.register(&manifest("custom-mem")).unwrap();
    assert!(layer.approved_command("custom-mem").unwrap_err().contains("not been approved"));
}

#[test]
fn registering_a_harness_again_keeps_the_folder_its_approval_lives_in() {
    let (_root, mut layer, folder, _binary) = on_disk("custom-again");
    approve_on_disk(&layer, &folder, "custom-again");
    let same = layer.live["custom-again"].manifest.clone();
    layer.register(&same).unwrap();
    assert!(layer.approved_command("custom-again").unwrap().is_some());
    // An edited manifest is a different manifest: its approval no longer matches.
    let mut edited = same;
    edited.args.push("--changed".to_string());
    layer.register(&edited).unwrap();
    assert!(layer.approved_command("custom-again").unwrap_err().contains("manifest changed"));
}
