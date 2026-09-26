// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Covers `PersistentInner`'s thin `poison_resume` / `try_capture_session_id`
//! wrappers — that they correctly plumb `session_id`/`resume_poisoned`
//! bookkeeping (unrelated to the race, still simple fields) alongside
//! delegating to `persistent_resume::update` for the resume/retry
//! decision itself. The exhaustive race-condition coverage (poison-
//! before/after the error line, message-batch growth, stop overrides,
//! mismatched sids, stale generations) now lives in
//! `persistent_resume::tests` against the pure function directly —
//! deterministic, and without needing this `PersistentInner` scaffolding
//! at all. Keeping both would just duplicate the same assertions at two
//! layers.

use super::super::*;

fn inner_with_session_id(session_id: Option<&str>) -> PersistentInner {
    PersistentInner {
        proc_status: STATUS_INIT.to_string(),
        proc_exit_code: 0,
        status_version: 0,
        session_id: session_id.map(str::to_string),
        resume_poisoned: None,
        fork_copy: None,
        restart_when_idle: false,
        restart_pending: false,
        stop_pending: false,
        resume: persistent_resume::ResumeState::default(),
        spawning_in_progress: false,
        pending_send_messages: VecDeque::new(),
        deferred_deliveries: VecDeque::new(),
        deferred_watchdog_armed: false,
        tool_wait: ToolWait::Writing,
        drain_claim: false,
        next_message_seq: 0,
        drain_send_in_flight: false,
        current_pid: None,
        stdin_tx: None,
        kill_tx: None,
        shutdown_generation: None,
        stop_exit: None,
        spawn_generation: 0,
        leftover_resume_candidate: None,
        pending_questions: HashMap::new(),
        pending_permissions: HashMap::new(),
        agent_lease: None,
    }
}

fn dummy_spawn_config() -> PersistentSpawnConfig {
    PersistentSpawnConfig {
        cli_command: "claude".to_string(),
        cli_args: vec![],
        working_dir: String::new(),
        env_vars: HashMap::new(),
        session_id_field: "session_id".to_string(),
        resume_flag: "--resume".to_string(),
        session_id: "dead-sid".to_string(),
        message_id: None,
    }
}

fn spawned_with_resume(inner: &mut PersistentInner, generation: u64, attempted_sid: &str) {
    // Mirror production's invariant (`spawn_process` bumps
    // `spawn_generation` in the same lock acquisition that applies
    // the spawn event): ambient adoption in `try_capture_session_id`
    // is gated on generation currency (issue #2366), so a helper
    // that left `spawn_generation` at 0 would make every capture
    // look stale.
    inner.spawn_generation = generation;
    inner.apply_resume_event(persistent_resume::ResumeEvent::SpawnedWithResume {
        generation,
        attempted_sid: attempted_sid.to_string(),
        retry: persistent_resume::RetryPayload { config: dummy_spawn_config(), messages: vec![persistent_resume::QueuedRetryEntry { seq: 1, json: "{}".to_string() }] },
    });
}

/// Issue #2366 regression: a superseded generation's still-draining
/// stdout reader must not re-install its stale sid into ambient
/// `session_id` after a fallback respawn's plain clear.
/// `respawn_once_for_leftover_queue` deliberately does NOT poison
/// (the death may be unrelated to a stale resume — see its round-13
/// comment), so `resume_poisoned` cannot catch this echo; only the
/// generation gate can.
#[test]
fn a_stale_generations_capture_does_not_adopt_into_ambient_session_id() {
    let mut inner = inner_with_session_id(Some("stale-sid"));
    spawned_with_resume(&mut inner, 1, "stale-sid");

    // The gen-1 process died; the fallback respawn cleared the
    // ambient sid and spawned gen 2 fresh (no --resume, so no new
    // resume tracking).
    inner.session_id = None;
    inner.spawn_generation = 2;
    inner.apply_resume_event(persistent_resume::ResumeEvent::SpawnedFresh { generation: 2 });

    // Gen 1's stdout reader finally drains its buffered echo of the
    // stale attempted sid.
    let (adopted, _effects) = inner.try_capture_session_id("stale-sid", 1, false);

    assert!(!adopted, "a superseded generation's echo must not be adopted");
    assert_eq!(
        inner.session_id, None,
        "ambient session_id must stay clear for the live generation's own capture"
    );
}

/// Control case for the gate above: the CURRENT generation's capture
/// into a cleared ambient sid must still adopt normally.
#[test]
fn the_current_generations_capture_still_adopts_after_a_clear() {
    let mut inner = inner_with_session_id(None);
    inner.spawn_generation = 2;
    inner.apply_resume_event(persistent_resume::ResumeEvent::SpawnedFresh { generation: 2 });

    let (adopted, _effects) = inner.try_capture_session_id("fresh-sid", 2, false);

    assert!(adopted, "the live generation's first capture must adopt");
    assert_eq!(inner.session_id.as_deref(), Some("fresh-sid"));
}

// stderr wins the race: poisons the id and clears it from session_id,
// then the stdout reader's later echo of the same dead id is refused.
#[test]
fn stderr_first_then_stdout_echo_is_refused() {
    let mut inner = inner_with_session_id(Some("dead-sid"));
    inner.poison_resume("dead-sid", 1);
    assert_eq!(inner.session_id, None, "poisoning the live session id clears it");

    let (captured, _effects) = inner.try_capture_session_id("dead-sid", 1, true);
    assert!(!captured, "must refuse to re-adopt a confirmed-poisoned id");
    assert_eq!(inner.session_id, None);
}

// stdout wins the race (echoes the dead id before stderr's "No
// conversation found" arrives): the later poison must still clear it.
#[test]
fn stdout_first_then_stderr_poison_still_clears() {
    let mut inner = inner_with_session_id(None);
    // A capture for generation 1 can only originate from a gen-1
    // spawn's own reader task (issue #2366's currency gate).
    inner.spawn_generation = 1;
    let (captured, _effects) = inner.try_capture_session_id("dead-sid", 1, true);
    assert!(captured, "first capture with no prior state succeeds");
    assert_eq!(inner.session_id.as_deref(), Some("dead-sid"));

    inner.poison_resume("dead-sid", 1);
    assert_eq!(inner.session_id, None, "poison must clear it even though stdout set it first");
}

// A genuinely fresh session id (the CLI gave up on --resume and started
// a new conversation) is unaffected by an unrelated prior poison.
#[test]
fn different_fresh_session_id_is_captured_normally() {
    let mut inner = inner_with_session_id(None);
    // A capture for generation 1 can only originate from a gen-1
    // spawn's own reader task (issue #2366's currency gate).
    inner.spawn_generation = 1;
    inner.poison_resume("dead-sid", 1);

    let (captured, _effects) = inner.try_capture_session_id("fresh-sid", 1, true);
    assert!(captured, "a different id is not blocked by an unrelated poison");
    assert_eq!(inner.session_id.as_deref(), Some("fresh-sid"));
}

// reagentx P1 (round 4 on this PR): the realistic precondition here
// is `session_id` already holding the STALE attempted sid — a
// `--resume <sid>` spawn always hydrates `session_id` to it BEFORE
// the process even starts (`spawn_process`). `adopted` used to be
// gated solely on `session_id.is_none()`, which this exact scenario
// (session_id already the stale sid, resume tracking still live)
// never satisfies — leaving `session_id` stuck on the stale sid
// forever even though the CLI had genuinely moved on to a brand-new
// conversation.
#[test]
fn adopts_a_genuinely_different_sid_even_when_session_id_already_holds_the_stale_attempted_one() {
    let mut inner = inner_with_session_id(Some("dead-sid"));
    spawned_with_resume(&mut inner, 1, "dead-sid");

    // A DIFFERENT (fresh) sid — the CLI gave up on --resume
    // internally and started its own new conversation without ever
    // hitting the stderr "No conversation found" path. A different
    // sid is unambiguous proof of progress on its own, even from a
    // frame that isn't itself a confirmed terminal success.
    let (captured, _effects) = inner.try_capture_session_id("brand-new-sid", 1, false);
    assert!(
        captured,
        "a genuinely different sid must be adopted even though session_id already held \
         the stale attempted one"
    );
    assert_eq!(
        inner.session_id.as_deref(),
        Some("brand-new-sid"),
        "session_id must move on to the fresh conversation, not stay stuck on the stale \
         attempted sid"
    );
    assert_eq!(inner.resume, persistent_resume::ResumeState::NotTracking { current_generation: 1 });
}

// Once a session id is already held, a second stdout line (e.g. a
// duplicate echo) must not overwrite it.
#[test]
fn does_not_overwrite_an_already_captured_session_id() {
    let mut inner = inner_with_session_id(Some("first-sid"));
    let (captured, _effects) = inner.try_capture_session_id("second-sid", 1, true);
    assert!(!captured, "must not overwrite an already-captured session id");
    assert_eq!(inner.session_id.as_deref(), Some("first-sid"));
}

// codex P1 on PR #2371: a `--resume <sid>` spawn ALWAYS has
// `session_id` already `Some` before the process starts (that's what
// makes `--resume` get attached at all) — so on the common
// resume-SUCCEEDED case, the CLI's first-line echo of that SAME sid
// must still resolve this generation's resume tracking even though
// `captured` itself is `false` (nothing new was adopted). Without
// this, persistent mode never exiting between turns meant that
// tentative state sat live for the rest of the process's potentially
// long lifetime, wrongly holding back every LATER, unrelated
// `is_error:true` result as if it might still need to be dropped for
// a stale-resume retry.
#[test]
fn resolves_pending_retry_on_a_successful_resume_even_though_session_id_was_already_held() {
    let mut inner = inner_with_session_id(Some("dead-sid"));
    spawned_with_resume(&mut inner, 1, "dead-sid");

    let (captured, _effects) = inner.try_capture_session_id("dead-sid", 1, true);

    assert!(!captured, "nothing new was adopted — session_id was already this exact sid");
    assert_eq!(inner.session_id.as_deref(), Some("dead-sid"));
    assert_eq!(
        inner.resume,
        persistent_resume::ResumeState::NotTracking { current_generation: 1 },
        "a successful resume must stand down the retry safety net even when \
         session_id was already held before this call"
    );
    assert_eq!(
        inner.resume,
        persistent_resume::ResumeState::NotTracking { current_generation: 1 },
        "a successful resume must stand down the retry safety net even when \
         session_id was already held before this call"
    );
}

// reagentx P0 on PR #2373: `try_capture_session_id` must surface the
// `FlushErrorLine` effect `apply_resume_event` returns, not just the
// `adopted` bool — an earlier cut of this method discarded it
// entirely, silently losing an EARLIER turn's held-back error line
// the moment a LATER, genuinely successful capture on this same
// still-alive generation resolved tracking. This is exactly the
// integration-level gap a pure `persistent_resume::update()` unit
// test can't catch (that function's own return value was always
// correct — see `persistent_resume::tests::
// session_captured_flushes_a_held_error_line_from_an_earlier_turn`);
// the bug was the caller silently dropping what `update()` handed
// back.
#[test]
fn try_capture_session_id_surfaces_a_held_error_line_from_an_earlier_turn() {
    let mut inner = inner_with_session_id(Some("dead-sid"));
    spawned_with_resume(&mut inner, 1, "dead-sid");
    // An earlier turn on this same generation held an error line back
    // (tracking still undecided at the time).
    let held = inner.apply_resume_event(persistent_resume::ResumeEvent::ErrorResultLine {
        generation: 1,
        line: "boom\n".to_string(),
    });
    assert!(held.is_empty(), "sanity: the line must be held back, not persisted immediately");

    // A LATER, genuinely successful capture on this same generation
    // resolves tracking — the caller must see (and flush) the
    // earlier held line, not silently lose it.
    let (_, effects) = inner.try_capture_session_id("dead-sid", 1, true);
    assert_eq!(
        effects,
        vec![
            persistent_resume::ResumeEffect::FlushErrorLine("boom\n".to_string()),
            persistent_resume::ResumeEffect::EmitSessionOutcome {
                outcome: persistent_resume::SessionOutcome::Resumed,
                attempted_sid: "dead-sid".to_string(),
                actual_sid: None,
            },
        ],
        "a held-back error line from an earlier turn must be surfaced to the caller \
         when a later capture resolves tracking, not silently discarded"
    );
}

// reagentx P0 on PR #2371 (originally), superseded by reagentx P0 on
// PR #2373: the real CLI's stream-json protocol embeds
// `session_id_field` on EVERY event, including the terminal `result`
// — so the doomed attempt's OWN `is_error:true` line carries the same
// (stale) sid it was given. The stdout reader's `!is_error_result`
// gate skips calling `try_capture_session_id` for that exact line,
// but this test proves the deeper fix: even if it WERE called here
// (belt-and-suspenders against a caller mistake, or a future frame
// type the gate doesn't anticipate), passing the correct
// `is_confirmed_success: false` (an error is never a confirmed
// success) means the ambiguous same-sid echo alone no longer resolves
// tracking — see `ResumeEvent::SessionCaptured`'s own doc comment.
#[test]
fn calling_try_capture_session_id_on_the_doomed_error_frame_does_not_clear_tracking() {
    let mut inner = inner_with_session_id(Some("dead-sid"));
    spawned_with_resume(&mut inner, 1, "dead-sid");

    // Simulates the terminal error-result line's OWN embedded
    // session_id field — the same sid this generation attempted,
    // not yet poisoned (the stderr reader hasn't necessarily run
    // yet), with `is_confirmed_success` correctly computed as `false`
    // since this frame IS the error.
    let (captured, _effects) = inner.try_capture_session_id("dead-sid", 1, false);
    assert!(!captured);
    assert!(
        matches!(inner.resume, persistent_resume::ResumeState::AwaitingOutcome { .. }),
        "an ambiguous same-sid echo on a frame that isn't a confirmed success must not \
         resolve tracking"
    );

    // Tracking is still live, so the ErrorResultLine event correctly
    // holds the line back pending the retry decision.
    let effects = inner.apply_resume_event(persistent_resume::ResumeEvent::ErrorResultLine {
        generation: 1,
        line: r#"{"type":"result","is_error":true}"#.to_string() + "\n",
    });
    assert!(
        effects.is_empty(),
        "tracking is still live, so the line must be held back, not persisted immediately"
    );
}
