// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What one container turn does, pinned before `spawn_container_turn` is split into steps: the calls it makes
//! on the container and their order, the prompt it uploads, the status it passes through, the exit code it
//! records for each way a turn can end, the interrupt, the message queued behind it, and what it leaves
//! behind when the container cannot take it. A fake Docker daemon (`container_fake_docker.rs`) stands in for
//! the runtime, so the turn runs end to end on a machine with none.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::{SubprocessController, SubprocessSpawnConfig};
use crate::backend::blockcontroller::{STATUS_DONE, STATUS_RUNNING};
use crate::backend::mps::Broker;

#[path = "container_fake_docker.rs"]
mod fake;
use fake::{FakeDocker, Kind, Turn};

const INIT: &str = r#"{"type":"system","subtype":"init","session_id":"sid-one"}"#;
const ANSWER: &str = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"pong"}]}}"#;
const RESULT_OK: &str = r#"{"type":"result","subtype":"success","is_error":false,"result":"pong"}"#;

fn controller(block: &str, broker: Option<Arc<Broker>>) -> Arc<SubprocessController> {
    let ctrl = Arc::new(SubprocessController::new("tab-1".into(), block.into(), broker, None, None, None, None, Arc::from("test-boot")));
    ctrl.set_self_ref();
    ctrl
}

fn config(message: &str) -> SubprocessSpawnConfig {
    SubprocessSpawnConfig {
        message: message.into(),
        resume_flag: "--resume".into(),
        resume_strategy: "flag".into(),
        session_id_field: "session_id".into(),
        env_vars: [("FOO".to_string(), "bar".to_string())].into_iter().collect(),
        instance_id: "inst-1".into(),
        ..Default::default()
    }
}

fn cli() -> Vec<String> {
    vec!["claude".to_string(), "-p".to_string()]
}

fn unique(tag: &str) -> String {
    format!("agentmux-test-ct-{tag}-{}", uuid::Uuid::new_v4().simple())
}

async fn wait_for(what: &str, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !ready() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn wait_done(ctrl: &SubprocessController) -> (String, i32) {
    wait_for("the turn to finish", || {
        ctrl.get_status_snapshot().shellprocstatus == STATUS_DONE && !ctrl.run_lock.load(Ordering::SeqCst)
    })
    .await;
    let status = ctrl.get_status_snapshot();
    (status.shellprocstatus, status.shellprocexitcode)
}

async fn run_one(fake: &FakeDocker, script: Turn, message: &str) -> (Arc<SubprocessController>, (String, i32)) {
    fake.script(vec![script]);
    let ctrl = controller(&unique("blk"), None);
    ctrl.spawn_container_turn(fake.manager(), unique("c"), cli(), config(message)).expect("the turn is accepted");
    let ended = wait_done(&ctrl).await;
    (ctrl, ended)
}

#[tokio::test]
async fn a_turn_uploads_the_prompt_runs_the_cli_from_it_captures_the_session_and_ends_done() {
    let fake = FakeDocker::start().await;
    fake.script(vec![Turn::says(&[INIT, ANSWER, RESULT_OK], 0)]);
    let ctrl = controller(&unique("blk"), None);
    let name = unique("c");
    ctrl.spawn_container_turn(fake.manager(), name, cli(), config("hello there")).unwrap();
    // The run lock is taken before the call returns; the status goes to running from the turn's own task.
    assert!(ctrl.run_lock.load(Ordering::SeqCst));

    assert_eq!(wait_done(&ctrl).await, (STATUS_DONE.to_string(), 0));
    assert_eq!(ctrl.session_id().as_deref(), Some("sid-one"), "the init event's session id is captured");
    assert!(!ctrl.get_status_snapshot().turn_active);

    // The memory counter is read first (nothing readable here, so it is not read again after the turn); the
    // uid probe and the upload come before the exec, and the exit code is inspected at the end.
    assert_eq!(fake.labels(), ["exec:oom", "exec:idprobe", "upload", "exec:turn", "inspect"]);

    let uploads = fake.uploads();
    assert_eq!(uploads.len(), 1);
    let (file, mode, uid, text) = &uploads[0];
    assert!(file.starts_with("agentmux-turn-"), "{file}");
    assert_eq!((*mode, *uid, text.as_str()), (0o600, 1000, "hello there"), "owned by the exec user, readable by it alone");

    let turns = fake.execs(Kind::Turn);
    let (cmd, env) = &turns[0];
    assert_eq!(&cmd[0..2], ["bash", "-c"]);
    assert_eq!(cmd[3], "agentmux-turn");
    assert_eq!(cmd[4], format!("/tmp/{file}"), "the CLI reads the uploaded file");
    assert_eq!(&cmd[5..], ["claude", "-p"], "no resume on a first turn");
    assert!(cmd.iter().all(|a| !a.contains("hello there")), "the message never reaches argv: {cmd:?}");
    assert!(env.iter().any(|e| e == "FOO=bar"), "the turn's own env travels as exec env: {env:?}");
}

#[tokio::test]
async fn the_next_turn_resumes_the_captured_session_and_reuses_the_probed_user() {
    let fake = FakeDocker::start().await;
    fake.script(vec![Turn::says(&[INIT, RESULT_OK], 0), Turn::says(&[RESULT_OK], 0)]);
    let ctrl = controller(&unique("blk"), None);
    let (cm, name) = (fake.manager(), unique("c"));
    ctrl.spawn_container_turn(cm.clone(), name.clone(), cli(), config("one")).unwrap();
    wait_done(&ctrl).await;
    ctrl.spawn_container_turn(cm, name, cli(), config("two")).unwrap();
    wait_done(&ctrl).await;

    let turns = fake.execs(Kind::Turn);
    assert_eq!(turns.len(), 2);
    assert_eq!(&turns[0].0[5..], ["claude", "-p"]);
    assert_eq!(&turns[1].0[5..], ["claude", "-p", "--resume", "sid-one"]);
    assert_eq!(fake.execs(Kind::IdProbe).len(), 1, "the exec user is probed once per container");
    let texts: Vec<String> = fake.uploads().into_iter().map(|u| u.3).collect();
    assert_eq!(texts, ["one", "two"]);
}

#[tokio::test]
async fn the_exit_code_comes_from_inspecting_the_exec() {
    let fake = FakeDocker::start().await;
    let (_, ended) = run_one(&fake, Turn::says(&[INIT], 3), "hi").await;
    assert_eq!(ended, (STATUS_DONE.to_string(), 3));
}

#[tokio::test]
async fn an_exec_with_no_exit_code_or_still_running_counts_as_failed() {
    let fake = FakeDocker::start().await;
    let none = Turn { exit: None, ..Turn::says(&[INIT], 0) };
    assert_eq!(run_one(&fake, none, "hi").await.1, (STATUS_DONE.to_string(), 1));

    let fake = FakeDocker::start().await;
    let running = Turn { still_running: true, ..Turn::says(&[INIT], 0) };
    assert_eq!(run_one(&fake, running, "hi").await.1, (STATUS_DONE.to_string(), 1));
}

#[tokio::test]
async fn a_broken_output_stream_fails_the_turn_without_inspecting_it() {
    let fake = FakeDocker::start().await;
    let cut = Turn { cut_mid_frame: true, ..Turn::says(&[INIT], 0) };
    let (ctrl, ended) = run_one(&fake, cut, "hi").await;
    assert_eq!(ended, (STATUS_DONE.to_string(), 1));
    assert!(!fake.labels().contains(&"inspect".to_string()), "{:?}", fake.labels());
    assert_eq!(ctrl.session_id().as_deref(), Some("sid-one"), "what arrived before the break is kept");
}

#[tokio::test]
async fn a_json_line_split_across_frames_and_a_last_line_without_a_newline_are_both_read() {
    let fake = FakeDocker::start().await;
    let split = Turn {
        frames: vec![(1, br#"{"type":"system","subtype":"init","#.to_vec()), (1, br#""session_id":"sid-split"}"#.to_vec())],
        exit: Some(0),
        ..Default::default()
    };
    let (ctrl, ended) = run_one(&fake, split, "hi").await;
    assert_eq!(ended, (STATUS_DONE.to_string(), 0));
    assert_eq!(ctrl.session_id().as_deref(), Some("sid-split"), "the line is joined, and flushed at the end of the stream");
}

#[tokio::test]
async fn stderr_does_not_become_output_and_a_failing_exit_still_ends_the_turn_done() {
    let fake = FakeDocker::start().await;
    let noisy = Turn { frames: vec![(2, b"boom: something broke\n".to_vec()), (1, format!("{INIT}\n").into_bytes())], exit: Some(2), ..Default::default() };
    let (ctrl, ended) = run_one(&fake, noisy, "hi").await;
    assert_eq!(ended, (STATUS_DONE.to_string(), 2));
    assert_eq!(ctrl.session_id().as_deref(), Some("sid-one"));
}

#[tokio::test]
async fn a_refused_exec_fails_the_turn_and_removes_the_uploaded_prompt() {
    let fake = FakeDocker::start().await;
    let (_, ended) = run_one(&fake, Turn { refuse: true, ..Default::default() }, "secret words").await;
    assert_eq!(ended, (STATUS_DONE.to_string(), 1));
    assert_eq!(fake.labels(), ["exec:oom", "exec:idprobe", "upload", "exec:turn", "exec:removeprompt"], "no inspect, no second memory read");
    let path = format!("/tmp/{}", fake.uploads()[0].0);
    let cleanup = &fake.execs(Kind::RemovePrompt)[0].0;
    assert_eq!(cleanup.last(), Some(&path), "the orphaned prompt is deleted from out here");
}

#[tokio::test]
async fn a_container_whose_user_cannot_be_probed_fails_the_turn_before_uploading_anything() {
    let fake = FakeDocker::start().await;
    fake.state.lock().unwrap().id_probe_fails = true;
    let (_, ended) = run_one(&fake, Turn::says(&[INIT], 0), "hi").await;
    assert_eq!(ended, (STATUS_DONE.to_string(), 1));
    assert_eq!(fake.labels(), ["exec:oom", "exec:idprobe"]);
}

#[tokio::test]
async fn a_refused_prompt_upload_fails_the_turn_without_running_the_cli() {
    let fake = FakeDocker::start().await;
    fake.state.lock().unwrap().upload_fails = true;
    let (_, ended) = run_one(&fake, Turn::says(&[INIT], 0), "hi").await;
    assert_eq!(ended, (STATUS_DONE.to_string(), 1));
    assert_eq!(fake.labels(), ["exec:oom", "exec:idprobe", "upload"]);
}

/// The step that readies the container runs for each message, in the turn's own task.
fn startup_counting(ctrl: &SubprocessController, ran: &Arc<AtomicUsize>, pause_ms: u64) {
    let ran = ran.clone();
    ctrl.set_container_startup(Arc::new(move || {
        let ran = ran.clone();
        Box::pin(async move {
            tokio::time::sleep(Duration::from_millis(pause_ms)).await;
            ran.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
    }));
}

#[tokio::test]
async fn a_message_sent_while_a_turn_runs_is_queued_and_runs_after_it_with_its_own_startup() {
    let fake = FakeDocker::start().await;
    fake.script(vec![Turn::says(&[INIT, RESULT_OK], 0), Turn::says(&[RESULT_OK], 0)]);
    let ctrl = controller(&unique("blk"), None);
    let ran = Arc::new(AtomicUsize::new(0));
    startup_counting(&ctrl, &ran, 200);
    let (cm, name) = (fake.manager(), unique("c"));
    ctrl.spawn_container_turn(cm.clone(), name.clone(), cli(), config("first")).unwrap();
    ctrl.spawn_container_turn(cm, name, cli(), config("second")).expect("a busy container queues the message");
    assert_eq!(ctrl.inner.lock().unwrap().pending_messages.len(), 1);

    wait_for("both turns", || fake.execs(Kind::Turn).len() == 2 && !ctrl.run_lock.load(Ordering::SeqCst)).await;
    assert_eq!(wait_done(&ctrl).await, (STATUS_DONE.to_string(), 0));
    let texts: Vec<String> = fake.uploads().into_iter().map(|u| u.3).collect();
    assert_eq!(texts, ["first", "second"], "in the order they were sent");
    assert_eq!(ran.load(Ordering::SeqCst), 2, "each message readies the container, so a failed start is retried by the next");
    assert!(ctrl.inner.lock().unwrap().pending_messages.is_empty());
    assert_eq!(&fake.execs(Kind::Turn)[1].0[5..], ["claude", "-p", "--resume", "sid-one"], "the queued turn resumes what the first captured");
}

#[tokio::test]
async fn a_failed_turn_still_hands_the_container_to_the_message_queued_behind_it() {
    let fake = FakeDocker::start().await;
    fake.script(vec![Turn { refuse: true, ..Default::default() }, Turn::says(&[INIT, RESULT_OK], 0)]);
    let ctrl = controller(&unique("blk"), None);
    let ran = Arc::new(AtomicUsize::new(0));
    startup_counting(&ctrl, &ran, 200);
    let (cm, name) = (fake.manager(), unique("c"));
    ctrl.spawn_container_turn(cm.clone(), name.clone(), cli(), config("first")).unwrap();
    ctrl.spawn_container_turn(cm, name, cli(), config("second")).unwrap();

    wait_for("the queued turn to run", || fake.execs(Kind::Turn).len() == 2 && !ctrl.run_lock.load(Ordering::SeqCst)).await;
    assert_eq!(wait_done(&ctrl).await, (STATUS_DONE.to_string(), 0), "the second turn's outcome is the last one");
    let texts: Vec<String> = fake.uploads().into_iter().map(|u| u.3).collect();
    assert_eq!(texts, ["first", "second"]);
    assert_eq!(ctrl.session_id().as_deref(), Some("sid-one"));
}

#[tokio::test]
async fn stopping_a_running_turn_interrupts_the_cli_and_removes_the_prompt() {
    let fake = FakeDocker::start().await;
    fake.script(vec![Turn { hold_open: true, ..Turn::says(&[INIT], 0) }]);
    let ctrl = controller(&unique("blk"), None);
    ctrl.spawn_container_turn(fake.manager(), unique("c"), cli(), config("long job")).unwrap();
    wait_for("the CLI to be started", || ctrl.session_id().as_deref() == Some("sid-one")).await;
    assert_eq!(ctrl.get_status_snapshot().shellprocstatus, STATUS_RUNNING);

    ctrl.stop_subprocess(false).expect("the stop is accepted");
    assert_eq!(wait_done(&ctrl).await, (STATUS_DONE.to_string(), -1), "an interrupted turn is not reported as idle");
    let interrupt = &fake.execs(Kind::Interrupt)[0].0;
    assert_eq!(interrupt, &["pkill", "-TERM", "-f", "claude"]);
    let path = format!("/tmp/{}", fake.uploads()[0].0);
    assert_eq!(fake.execs(Kind::RemovePrompt)[0].0.last(), Some(&path), "the killed wrapper cannot clean up, so this does");
    let labels = fake.labels();
    assert!(!labels.contains(&"inspect".to_string()), "{labels:?}");
    assert_eq!(labels.iter().filter(|l| *l == "exec:oom").count(), 1, "no memory read after an interrupt: {labels:?}");
}

#[tokio::test]
async fn a_forced_stop_kills_instead_of_terminating() {
    let fake = FakeDocker::start().await;
    fake.script(vec![Turn { hold_open: true, ..Turn::says(&[INIT], 0) }]);
    let ctrl = controller(&unique("blk"), None);
    ctrl.spawn_container_turn(fake.manager(), unique("c"), cli(), config("long job")).unwrap();
    wait_for("the CLI to be started", || ctrl.session_id().as_deref() == Some("sid-one")).await;
    ctrl.stop_subprocess(true).unwrap();
    assert_eq!(wait_done(&ctrl).await.1, -1);
    assert_eq!(fake.execs(Kind::Interrupt)[0].0, ["pkill", "-KILL", "-f", "claude"]);
}

#[tokio::test]
async fn a_provider_that_takes_its_prompt_as_an_argument_is_refused_before_anything_starts() {
    let fake = FakeDocker::start().await;
    let ctrl = controller(&unique("blk"), None);
    let cfg = SubprocessSpawnConfig { prompt_arg_flag: "-p".into(), ..config("hi") };
    let err = ctrl.spawn_container_turn(fake.manager(), unique("c"), cli(), cfg).unwrap_err();
    assert!(err.contains("takes its message on the command line"), "{err}");
    assert!(!ctrl.run_lock.load(Ordering::SeqCst));
    assert!(fake.labels().is_empty());
}

#[tokio::test]
async fn a_codex_compact_request_is_refused_in_a_container() {
    let fake = FakeDocker::start().await;
    let ctrl = controller(&unique("blk"), None);
    let cfg = SubprocessSpawnConfig { resume_strategy: "codex-exec".into(), ..config("/compact") };
    let err = ctrl.spawn_container_turn(fake.manager(), unique("c"), cli(), cfg).unwrap_err();
    assert!(err.contains("Compacting a Codex agent that runs in a container"), "{err}");
    assert!(!ctrl.run_lock.load(Ordering::SeqCst));
}

#[tokio::test]
async fn an_argv_that_cannot_be_built_releases_the_lock_and_starts_nothing() {
    let fake = FakeDocker::start().await;
    let ctrl = controller(&unique("blk"), None);
    // A known session with a resume strategy that needs a flag the provider does not have.
    let cfg = SubprocessSpawnConfig { resume_flag: String::new(), session_id: Some("sid-known".into()), ..config("hi") };
    let err = ctrl.spawn_container_turn(fake.manager(), unique("c"), cli(), cfg).unwrap_err();
    assert!(err.contains("requires a non-empty resume flag"), "{err}");
    assert!(!ctrl.run_lock.load(Ordering::SeqCst), "the lock taken for the turn is given back");
    assert!(fake.labels().is_empty());
}

/// Everything the broker was asked to publish, as `(event, data)`.
fn watch(broker: &Broker) -> Arc<Mutex<Vec<(String, serde_json::Value)>>> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    broker.add_observer(Arc::new(move |ev| {
        log.lock().unwrap().push((ev.event.clone(), ev.data.clone().unwrap_or_default()));
    }));
    seen
}

#[tokio::test]
async fn the_pane_is_told_the_turn_started_and_ended_and_gets_each_output_line() {
    let fake = FakeDocker::start().await;
    fake.script(vec![Turn::says(&[INIT, ANSWER, RESULT_OK], 0)]);
    let broker = Arc::new(Broker::new());
    let seen = watch(&broker);
    let ctrl = controller(&unique("blk"), Some(broker));
    ctrl.spawn_container_turn(fake.manager(), unique("c"), cli(), config("hi")).unwrap();
    wait_done(&ctrl).await;

    let events = seen.lock().unwrap().clone();
    let statuses: Vec<(String, bool)> = events
        .iter()
        .filter(|(name, _)| name == "controllerstatus")
        .map(|(_, data)| (data["shellprocstatus"].as_str().unwrap_or_default().to_string(), data["turn_active"].as_bool().unwrap_or(false)))
        .collect();
    assert_eq!(statuses.first(), Some(&(STATUS_RUNNING.to_string(), true)), "{statuses:?}");
    assert_eq!(statuses.last(), Some(&(STATUS_DONE.to_string(), false)), "{statuses:?}");

    let appended: String = events
        .iter()
        .filter(|(name, data)| name == "blockfile" && data["fileop"] == "append")
        .filter_map(|(_, data)| data["data64"].as_str().map(str::to_string))
        .filter_map(|b64| base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64).ok())
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .collect();
    for line in [INIT, ANSWER, RESULT_OK] {
        assert!(appended.contains(line), "the pane gets {line}: {appended}");
    }
}

#[tokio::test]
async fn a_failed_exec_is_written_into_the_pane_as_an_error_result() {
    let fake = FakeDocker::start().await;
    fake.script(vec![Turn { refuse: true, ..Default::default() }]);
    let broker = Arc::new(Broker::new());
    let seen = watch(&broker);
    let ctrl = controller(&unique("blk"), Some(broker));
    ctrl.spawn_container_turn(fake.manager(), unique("c"), cli(), config("hi")).unwrap();
    assert_eq!(wait_done(&ctrl).await, (STATUS_DONE.to_string(), 1));

    let events = seen.lock().unwrap().clone();
    let appended: String = events
        .iter()
        .filter(|(name, data)| name == "blockfile" && data["fileop"] == "append")
        .filter_map(|(_, data)| data["data64"].as_str().map(str::to_string))
        .filter_map(|b64| base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64).ok())
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .collect();
    assert!(appended.contains("[AgentMux] container exec failed:"), "{appended}");
    assert!(appended.contains(r#""subtype":"error_during_execution""#), "{appended}");
}

fn failures(seen: &Mutex<Vec<(String, serde_json::Value)>>) -> Vec<serde_json::Value> {
    seen.lock().unwrap().iter().filter(|(name, _)| name == crate::backend::mps::EVENT_AGENT_FAILURE).map(|(_, data)| data.clone()).collect()
}

#[tokio::test]
async fn a_failing_exit_publishes_its_classified_failure_and_a_clean_one_publishes_none() {
    let fake = FakeDocker::start().await;
    let broker = Arc::new(Broker::new());
    let seen = watch(&broker);
    fake.script(vec![Turn { frames: vec![(2, b"HTTP 429 rate_limit\n".to_vec())], exit: Some(1), ..Default::default() }, Turn::says(&[RESULT_OK], 0)]);
    let ctrl = controller(&unique("blk"), Some(broker));
    let (cm, name) = (fake.manager(), unique("c"));
    ctrl.spawn_container_turn(cm.clone(), name.clone(), cli(), config("one")).unwrap();
    wait_done(&ctrl).await;
    let first = failures(&seen);
    assert_eq!(first.len(), 1, "{first:?}");
    assert_eq!(first[0]["exitCode"], 1);
    assert!(first[0]["stderrTail"].as_str().unwrap_or_default().contains("429"), "{:?}", first[0]);

    ctrl.spawn_container_turn(cm, name, cli(), config("two")).unwrap();
    wait_done(&ctrl).await;
    assert_eq!(failures(&seen).len(), 1, "a clean turn after it publishes no failure");
}

#[tokio::test]
async fn an_error_the_provider_reports_in_band_is_a_failure_even_when_the_cli_exits_zero() {
    let fake = FakeDocker::start().await;
    let broker = Arc::new(Broker::new());
    let seen = watch(&broker);
    let in_band = r#"{"type":"assistant","isApiErrorMessage":true,"error":"authentication_failed","message":{"role":"assistant","content":[{"type":"text","text":"Invalid API key - authentication_failed 401"}]}}"#;
    fake.script(vec![Turn::says(&[INIT, in_band, RESULT_OK], 0)]);
    let ctrl = controller(&unique("blk"), Some(broker));
    ctrl.spawn_container_turn(fake.manager(), unique("c"), cli(), config("hi")).unwrap();
    assert_eq!(wait_done(&ctrl).await, (STATUS_DONE.to_string(), 0), "the exit code is the CLI's own");
    assert_eq!(failures(&seen).len(), 1, "but the turn is not reported as a success");
}

#[tokio::test]
async fn a_process_killed_for_memory_in_the_container_is_named_in_the_failure() {
    let fake = FakeDocker::start().await;
    fake.state.lock().unwrap().oom_counts = [0u64, 1].into();
    fake.script(vec![Turn::says(&[INIT], 137)]);
    let broker = Arc::new(Broker::new());
    let seen = watch(&broker);
    let ctrl = controller(&unique("blk"), Some(broker));
    ctrl.spawn_container_turn(fake.manager(), unique("c"), cli(), config("hi")).unwrap();
    assert_eq!(wait_done(&ctrl).await, (STATUS_DONE.to_string(), 137));
    assert_eq!(fake.labels(), ["exec:oom", "exec:idprobe", "upload", "exec:turn", "inspect", "exec:oom"], "the counter is read again after the turn");
    let published = failures(&seen);
    assert_eq!(published.len(), 1, "{published:?}");
    assert_eq!(published[0]["title"], "Ran out of memory");
}
