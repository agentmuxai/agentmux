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
    segment_as(gfs, sid, at, None);
}

/// [`segment`], recorded under the identity `identity_key`.
fn segment_as(gfs: &FileStore, sid: &str, at: i64, identity_key: Option<&str>) {
    segment_in(gfs, sid, at, identity_key, None);
}

/// [`segment_as`], recorded as run under `config_dir`.
fn segment_in(gfs: &FileStore, sid: &str, at: i64, identity_key: Option<&str>, config_dir: Option<&str>) {
    let id = segs::record_start(
        gfs,
        segs::Start {
            segment_id: String::new(),
            agent_uid: UID.into(),
            definition_id: None,
            provider: "claude".into(),
            config_dir: config_dir.map(str::to_string),
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
            identity_key: identity_key.map(str::to_string),
            forked_from: None,
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
    c.apply_resume_gate_with(&f.config, Some(&f.gfs), None);
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
    c.apply_resume_gate_with(&f.config, Some(&f.gfs), None);
    assert_eq!(held(&c), None);
}

#[test]
fn the_chain_head_is_left_alone() {
    let f = fixture(&["s1", "s2"]);
    segment(&f.gfs, "s1", 1_000);
    segment(&f.gfs, "s2", 2_000);
    let c = controller_holding(Some("s2"));
    c.apply_resume_gate_with(&f.config, Some(&f.gfs), None);
    assert_eq!(held(&c).as_deref(), Some("s2"));
}

#[test]
fn an_agent_without_a_chain_resumes_as_before() {
    let f = fixture(&["s1"]);
    let c = controller_holding(Some("s1"));
    c.apply_resume_gate_with(&f.config, Some(&f.gfs), None);
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
    c.apply_resume_gate_with(&f.config, Some(&f.gfs), None);
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
        c.apply_resume_gate_with(config, Some(&f.gfs), None);
        assert_eq!(held(&c).as_deref(), Some("s1"));
    }
    let c = controller_holding(Some("s1"));
    c.apply_resume_gate_with(&f.config, None, None);
    assert_eq!(held(&c).as_deref(), Some("s1"), "no global store");
}

/// Signs the fixture's config dir in as `account`/`org`, and returns that
/// identity's key as the spawn will compute it.
fn sign_in(f: &Fixture, account: &str, org: &str) -> String {
    let dir = f.config.env_vars["CLAUDE_CONFIG_DIR"].clone();
    std::fs::write(
        std::path::Path::new(&dir).join(".claude.json"),
        format!(r#"{{"oauthAccount":{{"accountUuid":"{account}","organizationUuid":"{org}"}}}}"#),
    )
    .unwrap();
    crate::identity::account_email::identity_key_from_oauth_dir("claude", &dir).unwrap()
}

/// SPEC §3 I2: the config dir was re-logged-into as another identity. The
/// head's file is right here, but resuming it would cross identities.
#[test]
fn the_head_recorded_under_another_identity_is_not_resumed() {
    let f = fixture(&["s2"]);
    let _now = sign_in(&f, "acc-new", "org-new");
    segment_as(&f.gfs, "s2", 2_000, Some("0000000000000000"));
    let c = controller_holding(Some("s2"));
    c.apply_resume_gate_with(&f.config, Some(&f.gfs), None);
    assert_eq!(held(&c), None);
}

#[test]
fn the_head_recorded_under_the_same_identity_is_resumed() {
    let f = fixture(&["s1", "s2"]);
    let key = sign_in(&f, "acc-1", "org-1");
    segment_as(&f.gfs, "s2", 2_000, Some(&key));
    let c = controller_holding(Some("s1"));
    c.apply_resume_gate_with(&f.config, Some(&f.gfs), None);
    assert_eq!(held(&c).as_deref(), Some("s2"));
}

#[test]
fn a_spawn_holding_no_session_is_untouched() {
    let f = fixture(&["s2"]);
    segment(&f.gfs, "s2", 2_000);
    let c = controller_holding(None);
    c.apply_resume_gate_with(&f.config, Some(&f.gfs), None);
    assert_eq!(held(&c), None, "the gate never invents a resume");
}

// ── Relocation (spec §4.3): the head under another login of the same identity ──

/// A second login's config dir, signed in as `account`/`org`, holding the
/// complete session `sid` for [`WORK`].
fn other_login(account: &str, org: &str, sid: &str) -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join(".claude.json"),
        format!(r#"{{"oauthAccount":{{"accountUuid":"{account}","organizationUuid":"{org}"}}}}"#),
    )
    .unwrap();
    let dir = tmp.path().join("projects").join(crate::backend::claude_layout::project_dir_name(WORK));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(format!("{sid}.jsonl")), b"{\"type\":\"user\"}\n{\"type\":\"assistant\"}\n").unwrap();
    tmp
}

fn here(f: &Fixture, sid: &str) -> std::path::PathBuf {
    crate::backend::session_backfill::session_file_path(&f.config.env_vars["CLAUDE_CONFIG_DIR"], WORK, sid).unwrap()
}

/// The operator's rebuild: a new login, same person. The pane holds the
/// head; its file is only under the old login.
#[test]
fn the_head_under_another_login_of_the_same_identity_is_relocated_to_fork_from() {
    let f = fixture(&[]);
    let key = sign_in(&f, "acc-1", "org-1");
    let old = other_login("acc-1", "org-1", "s2");
    segment_in(&f.gfs, "s2", 2_000, Some(&key), Some(&old.path().to_string_lossy()));
    let c = controller_holding(Some("s2"));
    c.apply_resume_gate_with(&f.config, Some(&f.gfs), None);

    assert_eq!(held(&c).as_deref(), Some("s2"));
    let copy = c.inner.lock().unwrap().fork_copy.clone().expect("a copy to fork from").copy;
    assert_eq!(copy, here(&f, "s2"));
    assert_eq!(std::fs::read(&copy).unwrap(), std::fs::read(old.path().join("projects").join(crate::backend::claude_layout::project_dir_name(WORK)).join("s2.jsonl")).unwrap());
}

/// A new pane holds no id; the history it renders is the head, under the old
/// login. The first spawn relocates it rather than starting blank.
#[test]
fn a_first_spawn_relocates_the_history_it_renders() {
    let f = fixture(&[]);
    let key = sign_in(&f, "acc-1", "org-1");
    let old = other_login("acc-1", "org-1", "s2");
    segment_in(&f.gfs, "s2", 2_000, Some(&key), Some(&old.path().to_string_lossy()));
    let c = controller_holding(None);
    c.apply_resume_gate_with(&f.config, Some(&f.gfs), Some("s2".into()));
    assert_eq!(held(&c).as_deref(), Some("s2"));
    assert!(c.inner.lock().unwrap().fork_copy.is_some());
}

/// The history candidate is for relocation only: a head the spawn could only
/// redirect to, or refuse, leaves a first spawn holding no id, as before.
#[test]
fn a_first_spawn_takes_nothing_but_a_relocation_from_its_history() {
    let f = fixture(&["s2"]);
    segment(&f.gfs, "s2", 2_000);
    let c = controller_holding(None);
    c.apply_resume_gate_with(&f.config, Some(&f.gfs), Some("s1".into()));
    assert_eq!(held(&c), None);
}

#[test]
fn another_identity_under_another_login_is_never_relocated() {
    let f = fixture(&[]);
    let _now = sign_in(&f, "acc-1", "org-team");
    let old = other_login("acc-1", "org-personal", "s2");
    let old_key = crate::identity::account_email::identity_key_from_oauth_dir("claude", &old.path().to_string_lossy()).unwrap();
    segment_in(&f.gfs, "s2", 2_000, Some(&old_key), Some(&old.path().to_string_lossy()));
    let c = controller_holding(Some("s2"));
    c.apply_resume_gate_with(&f.config, Some(&f.gfs), None);
    assert_eq!(held(&c), None);
    assert!(!here(&f, "s2").exists(), "nothing copied");
}

/// A copy a dead spawn left behind is swept before the gate looks, so it is
/// relocated afresh (and forked) instead of resumed in place.
#[test]
fn a_leftover_copy_is_swept_and_relocated_afresh() {
    let f = fixture(&[]);
    let key = sign_in(&f, "acc-1", "org-1");
    let old = other_login("acc-1", "org-1", "s2");
    segment_in(&f.gfs, "s2", 2_000, Some(&key), Some(&old.path().to_string_lossy()));
    let first = controller_holding(Some("s2"));
    first.apply_resume_gate_with(&f.config, Some(&f.gfs), None);
    assert!(first.inner.lock().unwrap().fork_copy.is_some());

    // That spawn died before its fork reported an id; the copy is still there.
    let second = controller_holding(Some("s2"));
    second.apply_resume_gate_with(&f.config, Some(&f.gfs), None);
    assert!(second.inner.lock().unwrap().fork_copy.is_some(), "relocated again, so it forks again");
}

/// The 0.57.7 move: the relocated fork's segment is recorded, then the pane
/// restarts it before it reports an id. That segment must not become the
/// head, or the next spawn finds the head "here" (the copy, about to be
/// swept) and resumes a file that is gone instead of relocating again.
#[test]
fn a_relocated_fork_killed_before_its_id_is_relocated_again() {
    let f = fixture(&[]);
    let key = sign_in(&f, "acc-1", "org-1");
    let old = other_login("acc-1", "org-1", "s2");
    segment_in(&f.gfs, "s2", 2_000, Some(&key), Some(&old.path().to_string_lossy()));
    let first = controller_holding(Some("s2"));
    first.apply_resume_gate_with(&f.config, Some(&f.gfs), None);
    assert!(first.inner.lock().unwrap().fork_copy.is_some());

    // The spawn records its segment, then dies before the fork reports an id.
    let mut start = first.segment_start(UID, &f.config, Some("s2"), Some("s2"), true, false, None);
    start.provider = "claude".into();
    segs::record_start(&f.gfs, start).unwrap();

    let second = controller_holding(Some("s2"));
    second.apply_resume_gate_with(&f.config, Some(&f.gfs), None);
    assert_eq!(held(&second).as_deref(), Some("s2"));
    assert!(second.inner.lock().unwrap().fork_copy.is_some(), "relocated again from the old login");
}

#[test]
fn a_fork_reporting_its_own_id_is_the_resume_succeeding() {
    use persistent_resume::SessionOutcome::{Fresh, Resumed};
    use super::super::segments::forked_outcome;
    assert_eq!(forked_outcome(Fresh, Some("s2"), "s2", Some("s3")), Resumed);
    assert_eq!(forked_outcome(Fresh, None, "s2", Some("s3")), Fresh, "not a relocated spawn");
    assert_eq!(forked_outcome(Fresh, Some("s2"), "s2", None), Fresh, "no new id");
    assert_eq!(forked_outcome(Fresh, Some("s9"), "s2", Some("s3")), Fresh, "another attempt");
    assert_eq!(forked_outcome(Resumed, Some("s2"), "s2", None), Resumed);
}

/// Codex P1 on #3907: capture never adopts an id the controller already
/// holds, so a relocated resume the CLI kept in place is settled apart.
#[test]
fn a_relocated_resume_kept_in_place_is_settled_once_a_result_proves_it() {
    use super::super::segments::kept_relocated_id;
    assert!(kept_relocated_id(false, true, true, true, Some("s2"), "s2"));
    assert!(!kept_relocated_id(false, false, true, true, Some("s2"), "s2"), "an init frame echoes the attempted id before a failure");
    assert!(!kept_relocated_id(true, true, true, true, Some("s2"), "s2"), "adopted: the capture path settles it");
    assert!(!kept_relocated_id(false, true, false, true, Some("s2"), "s2"), "a superseded generation");
    assert!(!kept_relocated_id(false, true, true, false, Some("s2"), "s2"), "already settled, or a same-dir fork with no copy");
    assert!(!kept_relocated_id(false, true, true, true, Some("s2"), "s3"), "a fork: its new id is adopted");
    assert!(!kept_relocated_id(false, true, true, true, None, "s2"), "not a fork");
}

/// Codex P1 on #3907 (round 2): only the current generation, still holding
/// the id, may settle a kept-in-place resume.
#[test]
fn only_the_current_generation_holding_the_id_holds_the_session() {
    let c = controller_holding(Some("s2"));
    let mut inner = c.inner.lock().unwrap();
    let now = inner.spawn_generation;
    assert!(inner.holds_current_session("s2", now));
    assert!(!inner.holds_current_session("s2", now.wrapping_sub(1)), "superseded by a replacement spawn");
    assert!(!inner.holds_current_session("s3", now), "the controller holds another id");
    inner.resume_poisoned = Some("s2".into());
    assert!(!inner.holds_current_session("s2", now), "known dead");
}

#[test]
fn the_copy_goes_once_the_fork_has_its_own_id() {
    use super::super::segments::settle_relocated_copy;
    let f = fixture(&[]);
    let old = other_login("acc-1", "org-1", "s2");
    let src = old.path().join("projects").join(crate::backend::claude_layout::project_dir_name(WORK)).join("s2.jsonl");
    let dest_dir = f.config.env_vars["CLAUDE_CONFIG_DIR"].clone();

    let r = crate::backend::continuity_relocate::relocate(&src, &dest_dir, WORK, "s2", UID).unwrap();
    assert!(settle_relocated_copy("block", &r, Some("s2"), "s3"));
    assert!(!r.copy.exists(), "forked: the copy goes");

    // The CLI resumed the copy in place (no fork): it is the live session now.
    let r = crate::backend::continuity_relocate::relocate(&src, &dest_dir, WORK, "s2", UID).unwrap();
    assert!(settle_relocated_copy("block", &r, Some("s2"), "s2"));
    assert!(r.copy.exists());
    assert_eq!(crate::backend::continuity_relocate::sweep(&dest_dir, WORK, UID), 0, "no longer marked as a copy");
    assert!(src.exists(), "the original is never touched");
}

/// Codex P1 on #3907 (round 3): a superseded spawn's reader settles late,
/// after a replacement swept its copy and relocated its own to the same
/// path. It must neither unmark nor remove the replacement's copy.
#[test]
fn a_late_settle_leaves_a_replacements_copy_alone() {
    use super::super::segments::settle_relocated_copy;
    let f = fixture(&[]);
    let old = other_login("acc-1", "org-1", "s2");
    let src = old.path().join("projects").join(crate::backend::claude_layout::project_dir_name(WORK)).join("s2.jsonl");
    let dest_dir = f.config.env_vars["CLAUDE_CONFIG_DIR"].clone();

    let stale = crate::backend::continuity_relocate::relocate(&src, &dest_dir, WORK, "s2", UID).unwrap();
    crate::backend::continuity_relocate::sweep(&dest_dir, WORK, UID);
    let _replacement = crate::backend::continuity_relocate::relocate(&src, &dest_dir, WORK, "s2", UID).unwrap();

    assert!(!settle_relocated_copy("block", &stale, Some("s2"), "s2"), "kept in place: not its copy any more");
    assert!(!settle_relocated_copy("block", &stale, Some("s2"), "s3"), "forked: not its copy any more");
    assert!(stale.copy.exists());
    assert_eq!(crate::backend::continuity_relocate::sweep(&dest_dir, WORK, UID), 1, "the replacement's copy is still marked");
}


// ── Continued outside AgentMux (spec §4.4) ──

/// Record a segment for `sid` that ended with the provider file at `bytes`.
fn ended_segment(gfs: &FileStore, sid: &str, at: i64, bytes: i64) {
    segment(gfs, sid, at);
    let id = segs::segments(gfs, UID).last().unwrap().start.segment_id.clone();
    segs::record_end_with_provider_bytes(gfs, UID, &id, segs::EndReason::Exited, None, Some(bytes), at + 10).unwrap();
}

/// The fixture writes each session as 3 bytes (`{}` + newline).
#[test]
fn a_session_that_grew_after_its_segment_ended_is_forked() {
    let f = fixture(&["s2"]);
    ended_segment(&f.gfs, "s2", 2_000, 1);
    let c = controller_holding(Some("s2"));
    c.apply_resume_gate_with(&f.config, Some(&f.gfs), None);
    assert_eq!(held(&c).as_deref(), Some("s2"));
    let inner = c.inner.lock().unwrap();
    assert!(inner.fork_next, "resumed as a fork");
    assert!(inner.fork_copy.is_none(), "nothing to copy: it is here");
}

#[test]
fn a_session_as_agentmux_left_it_is_resumed_in_place() {
    let f = fixture(&["s2"]);
    ended_segment(&f.gfs, "s2", 2_000, 3);
    let c = controller_holding(Some("s2"));
    c.apply_resume_gate_with(&f.config, Some(&f.gfs), None);
    assert_eq!(held(&c).as_deref(), Some("s2"));
    assert!(!c.inner.lock().unwrap().fork_next);
}

/// An agent's first upgrade from a build before identity keys (#3839): its
/// head segment has no key, so the key comes from the old login's own
/// `.claude.json`, and the same person's new login still relocates.
#[test]
fn a_head_recorded_before_identity_keys_still_relocates_for_the_same_person() {
    let f = fixture(&[]);
    let _key = sign_in(&f, "acc-1", "org-1");
    let old = other_login("acc-1", "org-1", "s2");
    segment_in(&f.gfs, "s2", 2_000, None, Some(&old.path().to_string_lossy()));
    let c = controller_holding(Some("s2"));
    c.apply_resume_gate_with(&f.config, Some(&f.gfs), None);
    assert_eq!(held(&c).as_deref(), Some("s2"));
    assert!(c.inner.lock().unwrap().fork_copy.is_some(), "relocated to fork from");
}

#[test]
fn a_head_recorded_before_identity_keys_under_another_person_is_not_relocated() {
    let f = fixture(&[]);
    let _key = sign_in(&f, "acc-1", "org-team");
    let old = other_login("acc-1", "org-personal", "s2");
    segment_in(&f.gfs, "s2", 2_000, None, Some(&old.path().to_string_lossy()));
    let c = controller_holding(Some("s2"));
    c.apply_resume_gate_with(&f.config, Some(&f.gfs), None);
    assert_eq!(held(&c), None);
    assert!(!here(&f, "s2").exists(), "nothing copied");
}


// ── The gate runs under the agent's lease ──

/// A lease store in its own temp registry, so the fixture's [`UID`] never
/// meets another test's lease.
fn lease_registry() -> (tempfile::TempDir, Arc<crate::registry::Registry>) {
    let tmp = tempfile::tempdir().unwrap();
    let registry = Arc::new(crate::registry::Registry::open(tmp.path().to_path_buf()).unwrap());
    (tmp, registry)
}

fn leased(sid: Option<&str>, registry: &Arc<crate::registry::Registry>, boot: &str) -> PersistentSubprocessController {
    controller_holding(sid).with_agent_lease_store(Some(Arc::clone(registry)), Arc::from(boot))
}

/// The fixture's head `s2`, only under an old login of the same identity:
/// a spawn holding it relocates it.
fn relocatable(f: &Fixture) -> tempfile::TempDir {
    let key = sign_in(f, "acc-1", "org-1");
    let old = other_login("acc-1", "org-1", "s2");
    segment_in(&f.gfs, "s2", 2_000, Some(&key), Some(&old.path().to_string_lossy()));
    old
}

/// Another instance runs the agent: the spawn is refused before the gate
/// sweeps, relocates or copies anything.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_spawn_the_lease_refuses_never_reaches_the_gate() {
    let f = fixture(&[]);
    let _old = relocatable(&f);
    let (_reg, registry) = lease_registry();
    let store = crate::backend::agent_admission::lease_store_for(Some(Arc::clone(&registry))).unwrap();
    let _other = crate::backend::agent_admission::acquire(&store, UID, "AgentA", &Arc::from("other-srv"), "other-block", None, |_| {}).unwrap();

    let c = leased(Some("s2"), &registry, "this-srv");
    let err = c.lease_then_gate(&f.config, Some(&f.gfs), None).err().expect("refused");

    assert!(err.contains("already running in another AgentMux instance"), "got: {err}");
    assert!(c.inner.lock().unwrap().fork_copy.is_none(), "the gate must not have run");
    assert!(!here(&f, "s2").exists(), "nothing relocated in");
}

/// The race #3924 kept narrowing: a second controller of the agent spawning
/// while the first holds it is refused at the lease, so it never sweeps the
/// first one's copy.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_second_controller_cannot_sweep_the_holders_relocation() {
    let f = fixture(&[]);
    let _old = relocatable(&f);
    let (_reg, registry) = lease_registry();

    let first = leased(Some("s2"), &registry, "this-srv");
    let lease = first.lease_then_gate(&f.config, Some(&f.gfs), None).unwrap();
    assert!(lease.is_some(), "the first spawn holds the agent");
    let copy = first.inner.lock().unwrap().fork_copy.clone().expect("relocated under the lease").copy;
    assert_eq!(copy, here(&f, "s2"));

    let second = leased(Some("s2"), &registry, "other-srv");
    assert!(second.lease_then_gate(&f.config, Some(&f.gfs), None).is_err());
    assert!(second.inner.lock().unwrap().fork_copy.is_none());
    assert!(copy.exists(), "the holder's copy is untouched");
    drop(lease);
}

/// The agent's other pane in this process holds the lease with this very
/// conversation open: that is the held-elsewhere refusal, which callers
/// never answer with a fresh session.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_other_pane_holding_the_conversation_is_reported_as_held_elsewhere() {
    let f = fixture(&[]);
    let sid = format!("sid-{}", uuid::Uuid::new_v4());
    let (_reg, registry) = lease_registry();
    let store = crate::backend::agent_admission::lease_store_for(Some(Arc::clone(&registry))).unwrap();
    let live_block = format!("live-{}", uuid::Uuid::new_v4());
    let _held = crate::backend::agent_admission::acquire(&store, UID, "AgentA", &Arc::from("this-srv"), &live_block, None, |_| {}).unwrap();
    let live = Arc::new(PersistentSubprocessController::new("tab".into(), live_block.clone(), None, None, None, None));
    {
        let mut g = live.inner.lock().unwrap();
        g.session_id = Some(sid.clone());
        g.current_pid = Some(4242);
    }
    crate::backend::blockcontroller::register_controller(&live_block, live.clone());

    let c = leased(Some(&sid), &registry, "this-srv");
    let err = c.lease_then_gate(&f.config, Some(&f.gfs), None).err().expect("refused");
    crate::backend::blockcontroller::delete_controller(&live_block);

    assert!(super::super::is_held_elsewhere_error(&err), "got: {err}");
}

/// Camper on #3935: this pane holds a stale id, and the agent's live pane in
/// this process — on another session — holds the lease. Still "open in
/// another pane" (with that pane's block), not "another instance", so the
/// callers settle queued prompts instead of falling back to a fresh session.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stale_pane_refused_by_the_agents_live_pane_is_held_elsewhere() {
    let f = fixture(&[]);
    let (_reg, registry) = lease_registry();
    let store = crate::backend::agent_admission::lease_store_for(Some(Arc::clone(&registry))).unwrap();
    let live_block = format!("live-{}", uuid::Uuid::new_v4());
    let held = crate::backend::agent_admission::acquire(&store, UID, "AgentA", &Arc::from("this-srv"), &live_block, None, |_| {}).unwrap();
    let live = Arc::new(PersistentSubprocessController::new("tab".into(), live_block.clone(), None, None, None, None));
    {
        let mut g = live.inner.lock().unwrap();
        g.session_id = Some(format!("head-{}", uuid::Uuid::new_v4()));
        g.current_pid = Some(4242);
        g.agent_lease = Some(Arc::new(held));
    }
    crate::backend::blockcontroller::register_controller(&live_block, live.clone());

    let stale = leased(Some(&format!("stale-{}", uuid::Uuid::new_v4())), &registry, "this-srv");
    let err = stale.lease_then_gate(&f.config, Some(&f.gfs), None).err().expect("refused");
    crate::backend::blockcontroller::delete_controller(&live_block);

    assert!(super::super::is_held_elsewhere_error(&err), "got: {err}");
    assert!(err.contains(&live_block), "names the live pane: {err}");
}
