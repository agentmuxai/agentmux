// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The steps of an agent turn, called without a controller.

use super::*;
use crate::backend::obj::MetaMapType;
use crate::server::tests::test_state;
use serde_json::json;

fn block_with(meta: &[(&str, serde_json::Value)]) -> Block {
    let mut block = Block { oid: uuid::Uuid::new_v4().to_string(), ..Default::default() };
    let mut m = MetaMapType::new();
    for (k, v) in meta {
        m.insert(k.to_string(), v.clone());
    }
    block.meta = m;
    block
}

#[test]
fn a_pane_with_no_command_runs_claude_with_the_stream_json_args() {
    let base = spawn_base(&block_with(&[]));
    assert_eq!(base.cli_command, "claude");
    assert_eq!(base.working_dir, "");
    assert_eq!(base.cli_args, ["-p", "--input-format", "stream-json", "--output-format", "stream-json"]);
}

#[test]
fn a_pane_runs_the_command_args_and_directory_its_meta_names() {
    let block = block_with(&[("cmd", json!("/bin/agent")), ("cmd:args", json!(["--one", "--two"])), ("cmd:cwd", json!("/work/x"))]);
    let base = spawn_base(&block);
    assert_eq!(base.cli_command, "/bin/agent");
    assert_eq!(base.cli_args, ["--one", "--two"]);
    assert_eq!(base.working_dir, "/work/x");
}

#[tokio::test]
async fn a_refusal_before_any_process_is_recorded_as_the_blocks_last_failure() {
    let state = test_state();
    let deps = AgentTurnDeps::from_state(&state);
    let mut block = block_with(&[]);
    state.mstore.insert(&mut block).unwrap();

    surface_refusal(&deps, &block.oid, "Attachments aren't supported for container agents yet.");

    let stored: Block = state.mstore.must_get(&block.oid).unwrap();
    let failure = stored.meta.get(crate::backend::blockcontroller::core::META_LAST_FAILURE).expect("a last failure");
    assert!(failure.to_string().contains("Attachments aren't supported"), "{failure}");
}

fn resolved(number: usize, name: &str, path: std::path::PathBuf) -> crate::backend::attachments::prompt::Resolved {
    crate::backend::attachments::prompt::Resolved { number, name: name.to_string(), path, mime: "image/png".into(), page_count: None, note: None }
}

fn resolved_with_note(number: usize, name: &str, path: std::path::PathBuf, note: &str) -> crate::backend::attachments::prompt::Resolved {
    crate::backend::attachments::prompt::Resolved { note: Some(note.to_string()), ..resolved(number, name, path) }
}

const HASH_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const HASH_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

#[test]
fn an_images_send_copy_keeps_its_id_and_send_suffix_so_reload_and_the_image_counter_still_recognize_it() {
    let host = tempfile::tempdir().unwrap();
    let send_copy = host.path().join(format!("{HASH_A}.v1-e2000.send.png"));
    std::fs::write(&send_copy, b"png bytes").unwrap();
    let cwd = tempfile::tempdir().unwrap();

    let (found, missing) = rehome_for_container(vec![resolved(1, "shot.png", send_copy)], Vec::new(), cwd.path().to_str().unwrap());

    assert!(missing.is_empty(), "{missing:?}");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].path, std::path::PathBuf::from(format!("/workspace/{CONTAINER_ATTACHMENTS_SUBDIR}/{HASH_A}.v1-e2000.send.png")));
    assert_eq!(std::fs::read(cwd.path().join(CONTAINER_ATTACHMENTS_SUBDIR).join(format!("{HASH_A}.v1-e2000.send.png"))).unwrap(), b"png bytes");
}

#[cfg(unix)]
#[test]
fn a_cwd_reached_through_a_symlinked_ancestor_is_not_refused() {
    // The symlink-safety walk must trust host_cwd and everything above it (a symlinked /tmp on macOS, a
    // symlinked home directory): a container agent cannot replace any of that, only the
    // .agentmux-attachments subtree it can actually write to (#7434).
    let host = tempfile::tempdir().unwrap();
    let send_copy = host.path().join(format!("{HASH_A}.send.png"));
    std::fs::write(&send_copy, b"png bytes").unwrap();

    let real_cwd = tempfile::tempdir().unwrap();
    let link_parent = tempfile::tempdir().unwrap();
    let symlinked_cwd = link_parent.path().join("cwd-via-symlink");
    std::os::unix::fs::symlink(real_cwd.path(), &symlinked_cwd).unwrap();

    let (found, missing) = rehome_for_container(vec![resolved(1, "shot.png", send_copy)], Vec::new(), symlinked_cwd.to_str().unwrap());

    assert!(missing.is_empty(), "{missing:?}");
    assert_eq!(found.len(), 1);
    assert_eq!(std::fs::read(real_cwd.path().join(CONTAINER_ATTACHMENTS_SUBDIR).join(format!("{HASH_A}.send.png"))).unwrap(), b"png bytes");
}

#[test]
fn a_named_copy_keeps_its_named_hash_realname_shape() {
    let host = tempfile::tempdir().unwrap();
    let named_dir = host.path().join("named").join(HASH_B);
    std::fs::create_dir_all(&named_dir).unwrap();
    let send_copy = named_dir.join("spec.pdf");
    std::fs::write(&send_copy, b"pdf bytes").unwrap();
    let cwd = tempfile::tempdir().unwrap();

    let (found, missing) = rehome_for_container(vec![resolved(1, "spec.pdf", send_copy)], Vec::new(), cwd.path().to_str().unwrap());

    assert!(missing.is_empty(), "{missing:?}");
    assert_eq!(found[0].path, std::path::PathBuf::from(format!("/workspace/{CONTAINER_ATTACHMENTS_SUBDIR}/named/{HASH_B}/spec.pdf")));
    assert_eq!(std::fs::read(cwd.path().join(CONTAINER_ATTACHMENTS_SUBDIR).join("named").join(HASH_B).join("spec.pdf")).unwrap(), b"pdf bytes");
}

#[test]
fn a_documents_extracted_text_companion_is_rehomed_and_the_note_rewritten() {
    let host = tempfile::tempdir().unwrap();
    let named_dir = host.path().join("named").join(HASH_B);
    std::fs::create_dir_all(&named_dir).unwrap();
    let send_copy = named_dir.join("spec.pdf");
    std::fs::write(&send_copy, b"pdf bytes").unwrap();
    let text_copy = host.path().join(format!("{HASH_B}.v1.text.txt"));
    std::fs::write(&text_copy, b"extracted text").unwrap();
    let cwd = tempfile::tempdir().unwrap();
    let note = format!("PDF, 12 pages; text version: {}", text_copy.display());

    let (found, missing) = rehome_for_container(vec![resolved_with_note(1, "spec.pdf", send_copy, &note)], Vec::new(), cwd.path().to_str().unwrap());

    assert!(missing.is_empty(), "{missing:?}");
    assert_eq!(found.len(), 1);
    assert_eq!(
        found[0].note.as_deref(),
        Some(format!("PDF, 12 pages; text version: /workspace/{CONTAINER_ATTACHMENTS_SUBDIR}/{HASH_B}.v1.text.txt").as_str())
    );
    assert_eq!(std::fs::read(cwd.path().join(CONTAINER_ATTACHMENTS_SUBDIR).join(format!("{HASH_B}.v1.text.txt"))).unwrap(), b"extracted text");
}

#[test]
fn a_missing_text_companion_drops_just_that_part_of_the_note() {
    let host = tempfile::tempdir().unwrap();
    let send_copy = host.path().join(format!("{HASH_A}.send.png"));
    std::fs::write(&send_copy, b"doc bytes").unwrap();
    let cwd = tempfile::tempdir().unwrap();
    let gone_text = std::env::temp_dir().join(format!("{HASH_B}.gone.text.txt"));
    let note = format!("Word document; text version: {}; contains macros", gone_text.display());

    let (found, missing) = rehome_for_container(vec![resolved_with_note(1, "report.docx", send_copy, &note)], Vec::new(), cwd.path().to_str().unwrap());

    assert!(missing.is_empty(), "the main attachment still succeeds: {missing:?}");
    assert_eq!(found[0].note.as_deref(), Some("Word document; contains macros"));
}

#[test]
fn two_attachments_with_different_ids_land_at_their_own_distinct_paths() {
    let host = tempfile::tempdir().unwrap();
    let cwd = tempfile::tempdir().unwrap();
    let a = host.path().join(format!("{HASH_A}.send.png"));
    let b = host.path().join(format!("{HASH_B}.send.png"));
    std::fs::write(&a, b"first").unwrap();
    std::fs::write(&b, b"second").unwrap();

    let (found, missing) = rehome_for_container(
        vec![resolved(1, "notes.png", a), resolved(2, "notes.png", b)],
        Vec::new(),
        cwd.path().to_str().unwrap(),
    );

    assert!(missing.is_empty(), "{missing:?}");
    assert_eq!(found.len(), 2);
    assert_ne!(found[0].path, found[1].path, "each attachment's own id keeps it distinct: {found:?}");
}

#[test]
fn a_rehomed_image_still_counts_as_one_in_the_transcripts_image_count() {
    // Round-trips rehome_for_container's own output through the real list-builder and the real
    // transcript parser (crate::backend::transcript::images::images_in_line, #7434), not just the path
    // shape, so a change to either side that stops recognizing a container path fails here.
    let host = tempfile::tempdir().unwrap();
    let send_copy = host.path().join(format!("{HASH_A}.v1.send.png"));
    std::fs::write(&send_copy, b"png bytes").unwrap();
    let cwd = tempfile::tempdir().unwrap();

    let (found, missing) = rehome_for_container(vec![resolved(1, "shot.png", send_copy)], Vec::new(), cwd.path().to_str().unwrap());
    assert!(missing.is_empty(), "{missing:?}");

    let list = crate::backend::attachments::prompt::list_block(&found, &missing, &[]);
    let line = json!({ "type": "user", "message": { "role": "user", "content": format!("look\n\n{list}") } }).to_string();
    assert_eq!(crate::backend::transcript::images::images_in_line(line.as_bytes()), 1, "{list}");
}

#[test]
fn a_send_copy_that_is_gone_joins_missing_instead_of_failing_the_whole_turn() {
    let cwd = tempfile::tempdir().unwrap();
    let gone = std::env::temp_dir().join(format!("{HASH_A}.send.png"));

    let (found, missing) = rehome_for_container(vec![resolved(1, "shot.png", gone)], vec!["already-missing.png".to_string()], cwd.path().to_str().unwrap());

    assert!(found.is_empty());
    assert_eq!(missing, vec!["already-missing.png".to_string(), "shot.png".to_string()]);
}

#[test]
fn a_path_with_no_recognizable_id_joins_missing_rather_than_being_sent_to_the_container_unidentified() {
    let cwd = tempfile::tempdir().unwrap();
    let plain = std::env::temp_dir().join("plain-name-no-hash.png");

    let (found, missing) = rehome_for_container(vec![resolved(1, "shot.png", plain)], Vec::new(), cwd.path().to_str().unwrap());

    assert!(found.is_empty());
    assert_eq!(missing, vec!["shot.png".to_string()]);
}

#[test]
fn id_bearing_suffix_reads_a_flat_derived_file_a_named_copy_and_rejects_neither_shape() {
    assert_eq!(id_bearing_suffix(std::path::Path::new(&format!("/s/{HASH_A}.v1.send.png"))), Some(std::path::PathBuf::from(format!("{HASH_A}.v1.send.png"))));
    assert_eq!(
        id_bearing_suffix(std::path::Path::new(&format!("/s/named/{HASH_B}/report.pdf"))),
        Some(std::path::PathBuf::from("named").join(HASH_B).join("report.pdf"))
    );
    assert_eq!(id_bearing_suffix(std::path::Path::new("/s/plain.png")), None);
    // A 64-char run that isn't hex (e.g. has a 'g') is not mistaken for an id.
    assert_eq!(id_bearing_suffix(std::path::Path::new(&format!("/s/{}.png", "g".repeat(64)))), None);
}

#[test]
fn attachment_destination_is_none_for_a_host_block_some_for_a_container_with_a_cwd_and_an_err_otherwise() {
    assert_eq!(attachment_destination(&block_with(&[])), Ok(None), "a host block has nothing to rehome");
    assert_eq!(
        attachment_destination(&block_with(&[("agentMode", json!("container")), ("cmd:cwd", json!("/work/agent"))])),
        Ok(Some("/work/agent".to_string()))
    );
    assert_eq!(
        attachment_destination(&block_with(&[("agentMode", json!("container"))])),
        Err("Attachments need a working folder, and this agent's pane has none yet.")
    );
    assert_eq!(
        attachment_destination(&block_with(&[("agentMode", json!("container")), ("cmd:cwd", json!("   "))])),
        Err("Attachments need a working folder, and this agent's pane has none yet."),
        "whitespace alone is no working folder either"
    );
}

#[test]
fn attachment_destination_expands_a_tilde_cwd_the_same_way_the_container_mount_does() {
    let Some(home) = dirs::home_dir() else { return };
    let got = attachment_destination(&block_with(&[("agentMode", json!("container")), ("cmd:cwd", json!("~/.agentmux/agents/x-0930a"))]));
    assert_eq!(got, Ok(Some(home.join(".agentmux/agents/x-0930a").to_string_lossy().into_owned())));
}

#[tokio::test]
async fn a_turn_for_a_block_with_no_controller_is_refused_before_anything_is_read() {
    let state = test_state();
    let deps = AgentTurnDeps::from_state(&state);
    let origin = crate::backend::message_origin::MessageOrigin::broadcast();
    let err = super::super::input::run_agent_turn_joining(
        &deps,
        "blk-turn-no-controller".to_string(),
        "hi".to_string(),
        None,
        super::super::input::TurnRegistration::Skip,
        origin,
        Vec::new(),
        None,
    )
    .await
    .unwrap_err();
    assert_eq!(err, "no controller for block blk-turn-no-controller");
}
