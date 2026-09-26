// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The spawn's resume gate
//! (SPEC_RESUME_GATE_AND_SAME_IDENTITY_CONTINUATION_2026_09_25.md §4.2): only
//! the head of the agent's segment chain is resumed natively.

use super::super::*;
use crate::backend::continuity_segments as segs;
use crate::backend::storage::filestore::FileStore;

const UID: &str = "gate-uid-1";
const WORK: &str = "/agents/agenta";

struct Fixture {
    _tmp: tempfile::TempDir,
    config: PersistentSpawnConfig,
    gfs: FileStore,
}

/// A config dir holding `<sid>.jsonl` for each of `on_disk`, and an empty
/// chain for [`UID`].
fn fixture(on_disk: &[&str]) -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("projects").join(crate::backend::claude_layout::project_dir_name(WORK));
    std::fs::create_dir_all(&dir).unwrap();
    for sid in on_disk {
        std::fs::write(dir.join(format!("{sid}.jsonl")), b"{}\n").unwrap();
    }
    let mut env_vars = HashMap::new();
    env_vars.insert("CLAUDE_CONFIG_DIR".to_string(), tmp.path().to_string_lossy().to_string());
    env_vars.insert("AGENTMUX_AGENT_UID".to_string(), UID.to_string());
    let config = PersistentSpawnConfig {
        cli_command: "claude".to_string(),
        cli_args: vec![],
        working_dir: WORK.to_string(),
        env_vars,
        session_id_field: "session_id".to_string(),
        resume_flag: "--resume".to_string(),
        session_id: String::new(),
        message_id: None,
    };
    Fixture { _tmp: tmp, config, gfs: FileStore::open_in_memory().unwrap() }
}

/// Append a segment whose process reported `sid`, `at` ms in.
fn segment(gfs: &FileStore, sid: &str, at: i64) {
    let id = segs::record_start(
        gfs,
        segs::Start {
            segment_id: String::new(),
            agent_uid: UID.into(),
            definition_id: None,
            provider: "claude".into(),
            config_dir: None,
            provider_session_id: None,
            cwd: WORK.into(),
            channel: "local-test".into(),
            agentmux_version: "0.0.0".into(),
            block_id: "elsewhere".into(),
            zone: None,
            byte_start: None,
            started_at_ms: at,
            continuity_rung: segs::Rung::Fresh,
            predecessor_segment_id: None,
            lease_epoch: None,
        },
    )
    .unwrap();
    segs::record_session(gfs, UID, &id, sid, at + 1).unwrap();
}

fn controller_holding(sid: Option<&str>) -> PersistentSubprocessController {
    let c = PersistentSubprocessController::new("tab".into(), "block".into(), None, None, None, None);
    c.inner.lock().unwrap().session_id = sid.map(str::to_string);
    c
}

fn held(c: &PersistentSubprocessController) -> Option<String> {
    c.inner.lock().unwrap().session_id.clone()
}

/// The operator's case: the pane holds S1, the conversation moved on to S2
/// (another channel or build), and both files are here.
#[test]
fn a_stale_held_session_is_redirected_to_the_chain_head() {
    let f = fixture(&["s1", "s2"]);
    segment(&f.gfs, "s1", 1_000);
    segment(&f.gfs, "s2", 2_000);
    let c = controller_holding(Some("s1"));
    c.apply_resume_gate_with(&f.config, Some(&f.gfs));
    assert_eq!(held(&c).as_deref(), Some("s2"));
}

/// The head lives under another login's config dir: resuming the stale id
/// would pick up an old branch, so nothing is resumed.
#[test]
fn a_stale_held_session_is_cleared_when_the_head_is_out_of_reach() {
    let f = fixture(&["s1"]);
    segment(&f.gfs, "s1", 1_000);
    segment(&f.gfs, "s2", 2_000);
    let c = controller_holding(Some("s1"));
    c.apply_resume_gate_with(&f.config, Some(&f.gfs));
    assert_eq!(held(&c), None);
}

#[test]
fn the_chain_head_is_left_alone() {
    let f = fixture(&["s1", "s2"]);
    segment(&f.gfs, "s1", 1_000);
    segment(&f.gfs, "s2", 2_000);
    let c = controller_holding(Some("s2"));
    c.apply_resume_gate_with(&f.config, Some(&f.gfs));
    assert_eq!(held(&c).as_deref(), Some("s2"));
}

#[test]
fn an_agent_without_a_chain_resumes_as_before() {
    let f = fixture(&["s1"]);
    let c = controller_holding(Some("s1"));
    c.apply_resume_gate_with(&f.config, Some(&f.gfs));
    assert_eq!(held(&c).as_deref(), Some("s1"));
}

/// A head that already failed to resume in this controller is no better
/// than the held id.
#[test]
fn a_poisoned_head_does_not_displace_the_held_session() {
    let f = fixture(&["s1", "s2"]);
    segment(&f.gfs, "s2", 2_000);
    let c = controller_holding(Some("s1"));
    c.inner.lock().unwrap().resume_poisoned = Some("s2".into());
    c.apply_resume_gate_with(&f.config, Some(&f.gfs));
    assert_eq!(held(&c).as_deref(), Some("s1"));
}

#[test]
fn nothing_is_gated_without_what_the_gate_needs() {
    let f = fixture(&["s1", "s2"]);
    segment(&f.gfs, "s2", 2_000);

    let mut no_uid = f.config.clone();
    no_uid.env_vars.remove("AGENTMUX_AGENT_UID");
    let mut no_config_dir = f.config.clone();
    no_config_dir.env_vars.remove("CLAUDE_CONFIG_DIR");
    let mut no_resume = f.config.clone();
    no_resume.resume_flag = String::new();

    for config in [&no_uid, &no_config_dir, &no_resume] {
        let c = controller_holding(Some("s1"));
        c.apply_resume_gate_with(config, Some(&f.gfs));
        assert_eq!(held(&c).as_deref(), Some("s1"));
    }
    let c = controller_holding(Some("s1"));
    c.apply_resume_gate_with(&f.config, None);
    assert_eq!(held(&c).as_deref(), Some("s1"), "no global store");
}

#[test]
fn a_spawn_holding_no_session_is_untouched() {
    let f = fixture(&["s2"]);
    segment(&f.gfs, "s2", 2_000);
    let c = controller_holding(None);
    c.apply_resume_gate_with(&f.config, Some(&f.gfs));
    assert_eq!(held(&c), None, "the gate never invents a resume");
}
