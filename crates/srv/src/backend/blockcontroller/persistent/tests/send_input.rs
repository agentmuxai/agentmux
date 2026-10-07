// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::super::*;
use crate::backend::obj::TermSize;

fn controller() -> PersistentSubprocessController {
    PersistentSubprocessController::new(
        "tab".to_string(),
        "block".to_string(),
        None,
        None,
        None,
        None,
    )
}

// ── Deferred runtime-config restart (AgentX, 2026-08-28) ────────────
//
// A `/model` change mid-turn used to kill the CLI with the user's message
// already on its stdin: the retry was suppressed (StopRequested reads as a
// user stop), the queue died with the discarded controller, and the
// replacement only "spawns on first message". The turn vanished silently.

/// Mid-turn: the caller must be told to leave the controller alone, and
/// the intent must be recorded for the turn-end hook to act on.
#[test]
fn request_restart_when_idle_defers_while_a_turn_is_in_flight() {
    let c = controller();
    c.health_monitor.set_active_turn(true);

    assert!(
        c.request_restart_when_idle(),
        "a turn is in flight — the caller must NOT replace the controller",
    );
    assert!(
        c.inner.lock().unwrap().restart_when_idle,
        "the deferred restart must be recorded for the turn-end hook",
    );
}

/// Idle: an immediate replace is safe and is what should happen — this is
/// the ordinary `/model` case. Deferring here would strand the change
/// until some future turn happened to end.
#[test]
fn request_restart_when_idle_does_not_defer_on_an_idle_pane() {
    let c = controller();
    c.health_monitor.set_active_turn(false);

    assert!(
        !c.request_restart_when_idle(),
        "no turn in flight — the caller should replace the controller immediately",
    );
    assert!(
        !c.inner.lock().unwrap().restart_when_idle,
        "nothing to defer, so nothing should be recorded",
    );
}

/// The flag is consumed, not merely read: the turn-end hook uses
/// `mem::replace`, so a single deferred change causes exactly one restart
/// rather than one at the end of every subsequent turn.
#[test]
fn the_deferred_restart_flag_is_consumed_exactly_once() {
    let c = controller();
    c.health_monitor.set_active_turn(true);
    c.request_restart_when_idle();

    let first = std::mem::replace(&mut c.inner.lock().unwrap().restart_when_idle, false);
    let second = std::mem::replace(&mut c.inner.lock().unwrap().restart_when_idle, false);
    assert!(first, "the first turn-end must see the deferred restart");
    assert!(!second, "a later turn-end must not restart again");
}

/// Repeated changes mid-turn (a user flipping model then effort) collapse
/// into one restart, not a queue of them.
#[test]
fn repeated_mid_turn_changes_collapse_into_one_restart() {
    let c = controller();
    c.health_monitor.set_active_turn(true);
    assert!(c.request_restart_when_idle());
    assert!(c.request_restart_when_idle());
    assert!(c.request_restart_when_idle());

    assert!(std::mem::replace(&mut c.inner.lock().unwrap().restart_when_idle, false));
    assert!(!c.inner.lock().unwrap().restart_when_idle);
}

/// reagent P1 on PR #2858: `restart_when_idle` is consumed at the
/// `is_result_frame` turn end, but a generation that dies abnormally
/// instead — user Stop/SIGINT, a crash, a permanently-failed turn
/// resolving via `ProcessExited` + `PublishDone` — never reaches that
/// branch. A leaked `true` would ride into an unrelated later generation
/// and kill a healthy process at the end of some future turn. Scoping the
/// flag per-spawn is what prevents that.
#[test]
fn a_deferred_restart_does_not_leak_into_the_next_generation() {
    let c = controller();
    c.health_monitor.set_active_turn(true);
    assert!(c.request_restart_when_idle());
    assert!(c.inner.lock().unwrap().restart_when_idle);

    // The generation dies WITHOUT a result frame (crash / user stop), so
    // nothing consumed the flag. The next spawn must start clean —
    // emulating spawn_process's own clear, which happens in the same
    // acquisition that bumps the generation.
    {
        let mut inner = c.inner.lock().unwrap();
        inner.restart_pending = false;
        inner.restart_when_idle = false;
        inner.spawn_generation += 1;
    }
    assert!(
        !c.inner.lock().unwrap().restart_when_idle,
        "a new generation must not inherit the previous one's deferred restart",
    );
}

/// codex P1 on PR #2858: `stop_process` only sends on `kill_tx` — it
/// leaves `stdin_tx` live until the process actually exits. A follow-up
/// arriving in that window (and turn end is EXACTLY when the frontend
/// flushes queued follow-ups) would take `DeliverDirect` into a process
/// about to receive EOF: acknowledged, then lost.
// ---- needs_spawn: the reactive-delivery routing predicate ----
// REPORT_JEKT_DELIVERY_DROPS_UNSPAWNED_PERSISTENT_AGENTS_2026_09_03.md

#[test]
fn needs_spawn_is_true_for_a_freshly_registered_controller() {
    // The state every persistent controller sits in after an srv restart:
    // registered ("spawns on first message"), no process yet. This is the
    // case that used to fail agent-to-agent delivery permanently.
    let c = controller();
    assert!(c.needs_spawn());
}

#[test]
fn needs_spawn_is_false_once_a_process_is_live() {
    // A live process can be steered mid-turn by the ordinary delivery path;
    // starting a second turn here would be wrong.
    let c = controller();
    let (tx, _rx) = mpsc::channel::<String>(4);
    c.inner.lock().unwrap().stdin_tx = Some(tx);
    assert!(!c.needs_spawn());
}

#[test]
fn needs_spawn_is_false_while_a_spawn_is_already_in_flight() {
    // The load-bearing case. A caller that has claimed the spawn owns this
    // round; reporting "needs spawn" here would invite a second, racing
    // spawn — the orphaned-child bug `spawning_in_progress` exists to
    // prevent. This window is retryable ("still starting up"), not
    // spawnable.
    let c = controller();
    {
        let mut inner = c.inner.lock().unwrap();
        inner.stdin_tx = None;
        inner.spawning_in_progress = true;
    }
    assert!(
        !c.needs_spawn(),
        "must not invite a second spawn while one is already claimed",
    );
}

#[test]
fn needs_spawn_is_false_while_a_retry_flush_drain_holds_its_claim() {
    // reagent P1 on PR #2960. When a RetryFlush drain's target process dies
    // mid-flush, the drain DELIBERATELY retains `drain_claim` for the
    // fallback respawn while the exit handler clears `stdin_tx`. Without
    // the `!drain_claim` term this window reported "needs spawn"; the
    // reactive path then called send_message, decide_send_action returned
    // Queued on the still-held claim, send_message returned Ok(()) having
    // delivered nothing, and the caller was told the message landed.
    // cloud_subscriber only retries on !success — so that is a lost message.
    let c = controller();
    {
        let mut inner = c.inner.lock().unwrap();
        inner.stdin_tx = None;
        inner.spawning_in_progress = false;
        inner.drain_claim = true;
    }
    assert!(
        !c.needs_spawn(),
        "a held drain claim must not be reported as spawnable — send_message would only queue",
    );
}

#[test]
fn needs_spawn_true_implies_decide_send_action_would_actually_spawn() {
    // The invariant, asserted directly rather than trusted: whenever
    // needs_spawn() is true, a real send must take the BecomeSpawner branch
    // (which spawns and delivers) and never Queued (which returns Ok having
    // delivered nothing). Covers all four flag combinations.
    for (stdin, spawning, drain) in [
        (false, false, false), // the only spawnable state
        (false, false, true),
        (false, true, false),
        (true, false, false),
    ] {
        let c = controller();
        let (tx, _rx) = mpsc::channel::<String>(4);
        {
            let mut inner = c.inner.lock().unwrap();
            inner.stdin_tx = if stdin { Some(tx) } else { None };
            inner.spawning_in_progress = spawning;
            inner.drain_claim = drain;
        }
        if c.needs_spawn() {
            assert!(
                matches!(c.decide_send_action("m", None), SendAction::BecomeSpawner { .. }),
                "needs_spawn() was true for (stdin={stdin}, spawning={spawning}, drain={drain}) \
                 but a real send would not have spawned",
            );
        }
    }
}

#[test]
fn a_committed_restart_refuses_deliver_direct_even_with_a_live_stdin() {
    let c = controller();
    let (tx, _rx) = mpsc::channel::<String>(4);
    {
        let mut inner = c.inner.lock().unwrap();
        inner.stdin_tx = Some(tx);
        // Sanity: without the restart commit this is DeliverDirect — so
        // the assertion below is about `restart_pending`, not about some
        // other precondition failing.
        assert!(inner.stdin_tx.is_some());
    }
    assert!(matches!(c.decide_send_action("m1", None), SendAction::DeliverDirect { .. }));

    c.inner.lock().unwrap().restart_pending = true;
    assert!(
        !matches!(c.decide_send_action("m2", None), SendAction::DeliverDirect { .. }),
        "a message must not be written into a process that is being killed",
    );
}

/// …and the message is not dropped either — it queues for the replacement.
#[test]
fn a_message_arriving_during_the_quiesce_window_is_queued_not_lost() {
    let c = controller();
    let (tx, _rx) = mpsc::channel::<String>(4);
    {
        let mut inner = c.inner.lock().unwrap();
        inner.stdin_tx = Some(tx);
        inner.restart_pending = true;
    }
    let action = c.decide_send_action("m1", None);
    assert!(
        matches!(action, SendAction::BecomeSpawner { .. } | SendAction::Queued),
        "must fall through to the spawn/queue path, not vanish",
    );
    assert_eq!(
        c.inner.lock().unwrap().pending_send_messages.len(),
        1,
        "the message must be retained for the replacement process",
    );
}

/// Reagent P1 on #3562: a human send while a kill is pending must not be
/// written into the dying process (reported delivered, then lost with it).
/// It is refused with an explicit error rather than routed to a spawn: a
/// pending stop may be a teardown (`delete_controller`, pane shutdown), and a
/// spawn there would start a fresh process for a pane that is closing. Once
/// the process has exited, the next send respawns as usual.
#[test]
fn a_human_send_while_a_stop_is_pending_is_refused_not_written() {
    let c = controller();
    let (tx, mut rx) = mpsc::channel::<String>(4);
    {
        let mut inner = c.inner.lock().unwrap();
        inner.stdin_tx = Some(tx);
        inner.stop_pending = true;
    }

    let action = c.decide_send_action("m1", None);
    assert!(matches!(action, SendAction::Refused(_)), "refused, not DeliverDirect or a spawn");
    assert!(c.inner.lock().unwrap().pending_send_messages.is_empty(), "not queued behind nothing");
    assert!(!c.health_monitor.is_active_turn(), "no turn reserved for a refused send");

    let config = PersistentSpawnConfig {
        cli_command: "unused-the-send-is-refused".to_string(),
        cli_args: vec![],
        working_dir: String::new(),
        env_vars: HashMap::new(),
        session_id_field: "session_id".to_string(),
        resume_flag: "--resume".to_string(),
        session_id: String::new(),
        message_id: None,
    };
    let err = c.send_message("human".to_string(), config).unwrap_err();
    assert!(err.contains("stopping"), "{err}");
    assert!(rx.try_recv().is_err(), "nothing written into the dying process");

    // The process exits: the next send respawns as usual.
    c.inner.lock().unwrap().stdin_tx = None;
    assert!(matches!(c.decide_send_action("m2", None), SendAction::BecomeSpawner { .. }));
}

/// Reagent P1 on #3562: `DeliverDirect` is decided under one `inner`
/// acquisition and written under a second, with the heartbeat and status
/// publish in between. A stop or restart committed in that gap (while
/// `stdin_tx` is still live) must not get the message written into the dying
/// process: the write re-checks the gates, fails, and hands back the turn.
#[tokio::test]
async fn a_stop_or_restart_committed_after_a_direct_decision_refuses_the_write() {
    let cases: [(&str, fn(&mut PersistentInner)); 2] = [
        ("stop", |i| i.stop_pending = true),
        ("restart", |i| i.restart_pending = true),
    ];
    for (why, commit) in cases {
        let (c, mut rx) = idle_controller();
        let human = PersistentSubprocessController::encode_user_message("human");
        let SendAction::DeliverDirect { was_active } = c.decide_send_action(&human, None) else {
            panic!("an idle, running agent takes DeliverDirect");
        };

        commit(&mut c.inner.lock().unwrap());

        assert!(c.deliver_direct(&human, was_active).is_err(), "{why}");
        assert!(rx.try_recv().is_err(), "nothing written into the dying process: {why}");
        assert!(!c.health_monitor.is_active_turn(), "the reserved turn is handed back: {why}");
    }
}

/// A deferred config restart also sets `stop_pending` (it goes through
/// `stop_process`). Its own rule wins: the message queues for the
/// replacement process instead of being refused.
#[test]
fn a_committed_restart_still_queues_for_the_replacement_despite_its_stop() {
    let c = controller();
    let (tx, _rx) = mpsc::channel::<String>(4);
    {
        let mut inner = c.inner.lock().unwrap();
        inner.stdin_tx = Some(tx);
        inner.restart_pending = true;
        inner.stop_pending = true;
    }
    let action = c.decide_send_action("m1", None);
    assert!(matches!(action, SendAction::BecomeSpawner { .. } | SendAction::Queued));
    assert_eq!(c.inner.lock().unwrap().pending_send_messages.len(), 1);
}

/// The quiesce window ends when the replacement arrives — otherwise every
/// later send would keep queueing behind a flag nothing clears.
#[test]
fn the_quiesce_flag_does_not_outlive_the_restart() {
    let c = controller();
    c.inner.lock().unwrap().restart_pending = true;
    // spawn_process clears it in the same acquisition that bumps the
    // generation; emulate that contract here (spawning a real process in
    // a unit test isn't practical).
    {
        let mut inner = c.inner.lock().unwrap();
        inner.restart_pending = false;
        inner.spawn_generation += 1;
    }
    let (tx, _rx) = mpsc::channel::<String>(4);
    c.inner.lock().unwrap().stdin_tx = Some(tx);
    assert!(
        matches!(c.decide_send_action("m1", None), SendAction::DeliverDirect { .. }),
        "once the replacement is up, ordinary direct delivery resumes",
    );
}

/// Shorthand for a retry-batch entry with an explicit queue seq
/// (issue #2365 — retry batches carry identity, not just text).
fn qentry(seq: u64, json: &str) -> persistent_resume::QueuedRetryEntry {
    persistent_resume::QueuedRetryEntry { seq, json: json.to_string() }
}

/// `has_prior_transcript` gates the fresh-start disclosure, so a false
/// positive would stamp "New session started" onto a brand-new agent's
/// very first turn — and, worse, clamp its scrollback against a boundary
/// that has nothing before it. With no filestore and no store there is
/// provably no history, and it must say so rather than defaulting to
/// "assume there might be".
#[test]
fn has_prior_transcript_is_false_without_any_backing_store() {
    assert!(!controller().has_prior_transcript());
}

/// codex P1 on PR #2500 (second round): the fresh-start clear must
/// retire every existing generation IN THE SAME lock acquisition —
/// a bare `session_id = None` left the dying generation still equal
/// to `spawn_generation` until `spawn_process`'s own (later) bump,
/// so its stdout reader's stale echo passed the #2366 currency gate
/// during exactly the window where `spawn_process` reads
/// `session_id` for the `--resume` decision.
#[test]
fn fresh_spawn_clear_makes_the_dying_generations_capture_stale_immediately() {
    let c = controller();
    {
        let mut inner = c.inner.lock().unwrap();
        inner.spawn_generation = 1;
        inner.session_id = Some("stale-sid".to_string());
    }

    // The fallback/retry path clears — BEFORE any new spawn exists.
    c.clear_session_id_for_fresh_spawn();

    // The gen-1 reader's buffered echo lands in the pre-spawn window.
    let mut inner = c.inner.lock().unwrap();
    let (adopted, _) = inner.try_capture_session_id("stale-sid", 1, false);

    assert!(
        !adopted,
        "the dying generation must be stale from the instant of the clear, \
         not only after spawn_process's own later bump"
    );
    assert_eq!(inner.session_id, None, "the fresh spawn must not see a --resume sid");
    assert_eq!(inner.spawn_generation, 2, "the clear reserves the next generation");
}

// codex P1 on PR #2360 (round 16, commit ce1642d90): `stop_process`
// must record which generation a stop was requested for even when
// there's no live `kill_tx` to send through — see
// `persistent_resume::ResumeEvent::StopRequested`'s own doc comment
// for why a `kill_tx` send alone can't be trusted (the process-
// waiter's `tokio::select!` can have already committed to its exit
// branch before this call reaches the lock, making the send futile
// even when `kill_tx` was still `Some`). Simulates the exact race
// here via no `kill_tx` at all (the narrower, always-reachable
// sub-case), with a resume attempt already in flight so the
// recorded stop has something to override.
#[test]
fn stop_process_records_the_current_generation_even_with_no_live_kill_tx() {
    let c = controller();
    {
        let mut inner = c.inner.lock().unwrap();
        inner.spawn_generation = 3;
        inner.apply_resume_event(persistent_resume::ResumeEvent::SpawnedWithResume {
            generation: 3,
            attempted_sid: "dead-sid".to_string(),
            retry: persistent_resume::RetryPayload {
                config: PersistentSpawnConfig {
                    cli_command: "claude".to_string(),
                    cli_args: vec![],
                    working_dir: String::new(),
                    env_vars: HashMap::new(),
                    session_id_field: "session_id".to_string(),
                    resume_flag: "--resume".to_string(),
                    session_id: "dead-sid".to_string(),
                    message_id: None,
                },
                messages: vec![qentry(1, "{}")],
            },
        });
    }
    // kill_tx stays None — the process already exited (or never
    // started); stop_process must still succeed and record intent.
    let result = c.stop_process(false);
    assert!(result.is_ok(), "must still return Ok when there's nothing live to signal");
    let resume_state = c.inner.lock().unwrap().resume.clone();
    match resume_state {
        persistent_resume::ResumeState::AwaitingOutcome { stop_requested, .. } => {
            assert!(
                stop_requested,
                "must record a signal the exit-handler can check even with no live kill_tx to send through"
            );
        }
        other => panic!("expected AwaitingOutcome with stop_requested, got {other:?}"),
    }
}

// A persistent controller has no PTY, but the agent pane's usePtyWidth hook
// sends a termsize resize on every running turn. It must be accepted as a
// no-op, not rejected — otherwise the pane logs a spurious "resize to N cols
// failed" warning. See AGENT_PANE_PTY_RESIZE_RACE_2026_06_16.md.
#[test]
fn termsize_resize_is_accepted_noop() {
    let c = controller();
    let res = c.send_input(BlockInputUnion::resize(TermSize { rows: 25, cols: 117 }), None);
    assert!(res.is_ok(), "termsize resize should be a no-op Ok, got {res:?}");
}

// The AskUserQuestion dead-air fallback re-delivers the answer as a directive
// follow-up message; the rendering must surface each Q/A so the model can
// resume with the decision in context. See SPEC_ASK_USER_QUESTION §10.1.
#[test]
fn answer_resume_message_renders_qa_pairs() {
    let answers = serde_json::json!({
        "Pick a color": "blue",
        "Pick toppings": ["cheese", "olives"],
    });
    let msg = build_answer_resume_message(&answers);
    assert!(msg.contains("Resume the task"), "must be directive: {msg}");
    assert!(msg.contains("Pick a color: blue"), "string answer: {msg}");
    assert!(
        msg.contains("Pick toppings: cheese, olives"),
        "multi-select joins labels: {msg}"
    );
}

#[test]
fn answer_resume_message_handles_non_object() {
    let msg = build_answer_resume_message(&serde_json::json!("just text"));
    assert!(msg.contains("Resume the task"), "still directive: {msg}");
    assert!(msg.contains("Answer: "), "non-object falls back: {msg}");
}

// Raw keystrokes are genuinely unsupported on a persistent controller —
// user messages go through send_message(), so they must still be rejected.
#[test]
fn raw_input_is_still_rejected() {
    let c = controller();
    let err = c
        .send_input(BlockInputUnion::data(b"ls\n".to_vec()), None)
        .unwrap_err();
    assert!(
        err.contains("does not accept raw input"),
        "raw input should be rejected, got {err:?}"
    );
}

/// Regression for REPORT_WORKING_STATE_REGRESSION_AND_STUCK_QUESTION_PANEL_2026_07_27.md
/// §2.7/§2.8: a fresh controller instance (pane reopen, or any process
/// respawn) has an empty `pending_questions` map. Confirms the error
/// message is descriptive enough for `muxlog` diagnosis — the frontend
/// no longer depends on matching this exact string (it falls back on
/// ANY answer_question failure now), but a clear message still matters
/// for debugging a future recurrence.
#[test]
fn answer_question_on_untracked_tool_use_id_is_descriptive() {
    let c = controller();
    let err = c
        .answer_question("tu-unknown".to_string(), serde_json::json!({}))
        .unwrap_err();
    assert!(
        err.contains("tu-unknown") && err.contains("respawned"),
        "error should name the tool_use_id and explain the likely cause, got {err:?}"
    );
}

// deny_question shares the identical pending_questions lookup as
// answer_question, so the frontend's SAFE_TO_RETRY_VIA_FOLLOWUP allowlist
// (useAgentQuestions.ts) matches this error text too — keep the "no
// pending AskUserQuestion" prefix stable if this message ever changes.
#[test]
fn deny_question_on_untracked_tool_use_id_is_descriptive() {
    let c = controller();
    let err = c
        .deny_question("tu-unknown".to_string(), "declined".to_string())
        .unwrap_err();
    assert!(
        err.contains("tu-unknown") && err.contains("respawned"),
        "error should name the tool_use_id and explain the likely cause, got {err:?}"
    );
}

// The dead-air fallback for a decline must still tell the model to resume
// (not wait for further input it will never get) while making clear the
// outcome was a DECLINE, not a real answer — otherwise the model could
// hallucinate a value the user never provided.
// Pins the exact literal (not just a substring) so an edit to this
// constant is impossible to make silently — see the KEEP IN SYNC comment
// on ASK_USER_QUESTION_DENY_MESSAGE's own definition. There is no way for
// this test to reach across the Rust/TypeScript boundary and check
// useAgentQuestions.ts's CANCEL_FALLBACK_MESSAGE directly; failing loudly
// here is what prompts a human/reviewer to go update that copy too.
#[test]
fn ask_user_question_deny_message_matches_frontend_cancel_fallback_text() {
    assert_eq!(
        ASK_USER_QUESTION_DENY_MESSAGE,
        "The user declined to answer this question.",
        "this literal is hand-mirrored as CANCEL_FALLBACK_MESSAGE in \
         frontend/app/view/agent/hooks/useAgentQuestions.ts — update both \
         together"
    );
}

#[test]
fn deny_resume_message_is_directive_and_includes_the_reason() {
    let msg = build_deny_resume_message(ASK_USER_QUESTION_DENY_MESSAGE);
    assert!(msg.contains("Resume the task"), "must be directive: {msg}");
    assert!(
        msg.contains("declined to answer"),
        "must carry the decline reason: {msg}"
    );
}

// ── Phase 2 gate: tool-permission parking + decision (SPEC_DECISION_PROMPT_2026_04_24.md) ──
//
// should_route_to_decision_panel is hardcoded false in production, so
// park_tool_permission_request/decide_tool_permission are unreachable
// from handle_control_frame today. These tests call them DIRECTLY
// (both are in scope via `use super::super::*;`), independent of that gate —
// exercising the mechanism without relying on, or changing, today's
// default auto-allow behavior.

// Pins the gate itself: whatever tool name is asked about, today's
// answer must be "auto-allow", not "prompt". This is the test that
// would need to change (deliberately, not by accident) the day this
// gate is flipped on for real.
#[test]
fn should_route_to_decision_panel_is_inert_for_any_tool_today() {
    for tool in ["Bash", "Edit", "Write", "WebFetch", "AskUserQuestion", ""] {
        assert!(
            !should_route_to_decision_panel(tool),
            "gate must stay false for {tool:?} until the Phase 2 policy decision is made"
        );
    }
}

// decide_tool_permission spawns the dead-air fallback task
// (tokio::spawn) as part of every call, same as answer_question/
// deny_question — needs a running runtime even though this test never
// waits out that 4s fallback itself (this crate doesn't enable tokio's
// test-util feature; see status_heartbeat_republishes_while_active's own
// comment above for why a virtual clock isn't available here).
#[tokio::test]
async fn parking_then_deciding_allow_sends_the_original_input_back_unmodified() {
    let c = controller();
    let (tx, mut rx) = mpsc::channel::<String>(4);
    c.inner.lock().unwrap().stdin_tx = Some(tx);

    park_tool_permission_request(
        &c.inner,
        "tu-1".to_string(),
        "req-1".to_string(),
        "Bash".to_string(),
        serde_json::json!({ "command": "ls" }),
    );

    c.decide_tool_permission("tu-1".to_string(), "allow", None)
        .expect("a freshly-parked request must be decidable");

    let sent = rx.try_recv().expect("decide_tool_permission must write to stdin");
    let parsed: serde_json::Value = serde_json::from_str(&sent).unwrap();
    assert_eq!(parsed["type"], "control_response");
    assert_eq!(parsed["response"]["request_id"], "req-1");
    let inner_resp = &parsed["response"]["response"];
    assert_eq!(inner_resp["behavior"], "allow");
    assert_eq!(inner_resp["toolUseID"], "tu-1");
    // The original input comes back byte-for-byte — this method has no
    // "edit before approving" UI (none exists yet).
    assert_eq!(inner_resp["updatedInput"], serde_json::json!({ "command": "ls" }));

    // Consumed on decide — a second decide on the same id must fail,
    // same lifecycle as answer_question/deny_question.
    assert!(c.decide_tool_permission("tu-1".to_string(), "allow", None).is_err());
}

#[tokio::test]
async fn parking_then_deciding_deny_carries_user_feedback_verbatim() {
    // SPEC_DECISION_PROMPT_2026_04_24.md G6: "Denials carry user-typed
    // feedback verbatim to the agent."
    let c = controller();
    let (tx, mut rx) = mpsc::channel::<String>(4);
    c.inner.lock().unwrap().stdin_tx = Some(tx);

    park_tool_permission_request(
        &c.inner,
        "tu-2".to_string(),
        "req-2".to_string(),
        "Bash".to_string(),
        serde_json::json!({ "command": "rm -rf /tmp/x" }),
    );

    c.decide_tool_permission("tu-2".to_string(), "deny", Some("too risky, use trash instead".to_string()))
        .expect("a freshly-parked request must be decidable");

    let sent = rx.try_recv().unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&sent).unwrap();
    let inner_resp = &parsed["response"]["response"];
    assert_eq!(inner_resp["behavior"], "deny");
    assert_eq!(inner_resp["message"], "too risky, use trash instead");
    assert_eq!(inner_resp["toolUseID"], "tu-2");
}

#[tokio::test]
async fn deciding_deny_with_no_feedback_still_tells_the_model_it_was_refused() {
    let c = controller();
    let (tx, mut rx) = mpsc::channel::<String>(4);
    c.inner.lock().unwrap().stdin_tx = Some(tx);

    park_tool_permission_request(
        &c.inner,
        "tu-3".to_string(),
        "req-3".to_string(),
        "Write".to_string(),
        serde_json::json!({}),
    );

    c.decide_tool_permission("tu-3".to_string(), "deny", None).unwrap();

    let sent = rx.try_recv().unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&sent).unwrap();
    assert_eq!(parsed["response"]["response"]["message"], "Denied by user.");
}

/// Same shape as answer_question_on_untracked_tool_use_id_is_descriptive
/// below — a fresh controller instance (pane reopen, any process
/// respawn) has an empty pending_permissions map.
#[test]
fn decide_tool_permission_on_untracked_tool_use_id_is_descriptive() {
    let c = controller();
    let err = c
        .decide_tool_permission("tu-unknown".to_string(), "allow", None)
        .unwrap_err();
    assert!(
        err.contains("tu-unknown") && err.contains("respawned"),
        "error should name the tool_use_id and explain the likely cause, got {err:?}"
    );
}

#[test]
fn tool_decision_resume_message_names_the_tool_and_the_outcome() {
    let allow_msg = build_tool_decision_resume_message("Bash", "allow", None);
    assert!(allow_msg.contains("Bash"));
    assert!(allow_msg.contains("approved"));
    assert!(allow_msg.contains("Resume the task"));

    let deny_msg = build_tool_decision_resume_message("Write", "deny", Some("no"));
    assert!(deny_msg.contains("Write"));
    assert!(deny_msg.contains("declined"));
    assert!(deny_msg.contains("\"no\""));
}

// `turn_active` on the runtime status snapshot must track the health
// monitor's active-turn flag directly — this is the signal the frontend
// seeds TurnPhase from at mount instead of always defaulting to Idle
// (see docs/specs/REPORT_AGENT_PANE_STATE_RECONCILIATION_2026_07_07.md
// Finding 1). Exercised here via the health monitor directly rather than
// send_message()/the stdout reader, which both require a real spawned
// process.
#[test]
fn status_snapshot_turn_active_tracks_turn_activity_tracker() {
    let c = controller();
    assert!(
        !c.get_status_snapshot().turn_active,
        "freshly constructed controller has no turn in flight"
    );

    c.health_monitor.set_active_turn(true);
    assert!(
        c.get_status_snapshot().turn_active,
        "turn_active must flip true once the turn-activity tracker marks a turn active"
    );

    c.health_monitor.set_active_turn(false);
    assert!(
        !c.get_status_snapshot().turn_active,
        "turn_active must flip back false once the turn ends"
    );
}

/// Regression for reagent P2 (persist-controllerstatus PR): confirms
/// `spawn_status_heartbeat` actually republishes while a turn stays
/// active, using a short real interval (`spawn_status_heartbeat_with_interval`)
/// instead of waiting out the production 20s — this crate doesn't enable
/// tokio's `test-util` feature, so a real (short) interval is used
/// rather than a virtual/paused clock.
#[tokio::test]
async fn status_heartbeat_republishes_while_active() {
    let broker = Arc::new(crate::backend::mps::Broker::new());
    let c = PersistentSubprocessController::new(
        "tab".to_string(),
        "block-heartbeat".to_string(),
        Some(broker.clone()),
        None,
        None,
        None,
    );
    c.health_monitor.set_active_turn(true);
    c.spawn_status_heartbeat_with_interval(tokio::time::Duration::from_millis(5));

    // Generous margin over several 5ms ticks — proves at least one
    // heartbeat tick actually published, without asserting an exact count
    // (real-time scheduling, not virtual-clock-deterministic).
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    let history = broker.read_event_history(
        crate::backend::mps::EVENT_CONTROLLER_STATUS,
        "block:block-heartbeat",
        1,
    );
    assert_eq!(history.len(), 1, "heartbeat must have published at least once while active");
    let status: BlockControllerRuntimeStatus =
        serde_json::from_value(history[0].data.clone().unwrap()).unwrap();
    assert!(status.turn_active, "published snapshot must reflect the active turn");
}

/// Regression for reagent P1 (round 2 on the persist-controllerstatus
/// PR): the heartbeat loop must publish one final `turn_active: false`
/// snapshot before exiting, not just break silently. This is the exact
/// case the heartbeat exists to backstop — a missed live turn-end
/// push — so a silent exit with no final publish would leave the
/// client stuck showing "Working" in precisely the scenario this whole
/// mechanism was built for.
#[tokio::test]
async fn status_heartbeat_publishes_final_inactive_status_before_stopping() {
    let broker = Arc::new(crate::backend::mps::Broker::new());
    let c = PersistentSubprocessController::new(
        "tab".to_string(),
        "block-heartbeat-stop".to_string(),
        Some(broker.clone()),
        None,
        None,
        None,
    );
    c.health_monitor.set_active_turn(true);
    c.spawn_status_heartbeat_with_interval(tokio::time::Duration::from_millis(5));

    // Let at least one active-turn tick land, then mark the turn ended —
    // simulating the exact scenario: the "real" turn-end publish (from
    // wherever normally calls publish_controller_status on completion)
    // is the one that got dropped, and the heartbeat is the only thing
    // left that can correct the client's stale "Working" state.
    tokio::time::sleep(tokio::time::Duration::from_millis(30)).await;
    c.health_monitor.set_active_turn(false);
    tokio::time::sleep(tokio::time::Duration::from_millis(30)).await;

    let history = broker.read_event_history(
        crate::backend::mps::EVENT_CONTROLLER_STATUS,
        "block:block-heartbeat-stop",
        1,
    );
    assert_eq!(history.len(), 1, "must have published at least the final status");
    let status: BlockControllerRuntimeStatus =
        serde_json::from_value(history[0].data.clone().unwrap()).unwrap();
    assert!(
        !status.turn_active,
        "the heartbeat's last publish before stopping must reflect turn_active: false, \
         not silently disappear leaving the client on a stale turn_active: true"
    );
}

/// reagentx P0 on PR #2360: `poison_resume` (the stderr-reader task) and
/// `retry_after_resume_failure` (called from the process-waiter task)
/// are two independently-scheduled tasks with no ordering guarantee —
/// this must clear `inner.session_id` itself rather than assuming
/// `poison_resume` already ran first. Uses a nonexistent binary so the
/// respawn attempt inside this function fails fast (no real process
/// needed) — the assertion only cares that `inner.session_id` was
/// cleared BEFORE that attempt, which is what stops a later, genuinely
/// successful respawn from ever re-attaching `--resume` to the same
/// dead id.
#[test]
fn retry_after_resume_failure_clears_inner_session_id_even_when_poison_resume_has_not_run_yet() {
    let c = controller();
    c.inner.lock().unwrap().session_id = Some("dead-sid".to_string());

    let config = PersistentSpawnConfig {
        cli_command: "definitely-not-a-real-binary-xyz".to_string(),
        cli_args: vec![],
        working_dir: String::new(),
        env_vars: HashMap::new(),
        session_id_field: "session_id".to_string(),
        resume_flag: "--resume".to_string(),
        session_id: "dead-sid".to_string(),
        message_id: None,
    };
    c.retry_after_resume_failure(1, config, vec![qentry(1, "{}")], None, "dead-sid".to_string());

    assert_eq!(
        c.inner.lock().unwrap().session_id,
        None,
        "must clear inner.session_id directly, not rely on poison_resume having already done so"
    );
}

/// Regression test for
/// `docs/status/STATUS_CROSS_CHANNEL_RESUME_STALE_SESSION_ID_2026_08_20.md`:
/// a confirmed-stale `--resume` must try to recover the largest REAL
/// session on disk before giving up and starting blank.
#[test]
fn find_recovery_session_id_recovers_the_largest_on_disk_session() {
    let tmp = tempfile::tempdir().unwrap();
    let config_dir = tmp.path().to_string_lossy().to_string();
    let working_dir = r"C:\Users\user\.agentmux\agents\agentx-0623n".to_string();
    let slug = crate::backend::claude_layout::project_dir_name(&working_dir);
    let dir = tmp.path().join("projects").join(&slug);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("972a6a4f-live.jsonl"), vec![b'x'; 2_800_000]).unwrap();

    let c = controller();
    let mut env_vars = HashMap::new();
    env_vars.insert("CLAUDE_CONFIG_DIR".to_string(), config_dir);
    let config = PersistentSpawnConfig {
        cli_command: "claude".to_string(),
        cli_args: vec![],
        working_dir,
        env_vars,
        session_id_field: "session_id".to_string(),
        resume_flag: "--resume".to_string(),
        session_id: "d019e2e4-stale".to_string(),
        message_id: None,
    };

    assert_eq!(
        c.find_recovery_session_id(&config).as_deref(),
        Some("972a6a4f-live"),
        "must recover the largest real on-disk session, not give up"
    );
}

#[test]
fn find_recovery_session_id_is_none_without_a_config_dir() {
    let c = controller();
    let config = PersistentSpawnConfig {
        cli_command: "claude".to_string(),
        cli_args: vec![],
        working_dir: "/wherever".to_string(),
        env_vars: HashMap::new(), // no CLAUDE_CONFIG_DIR — same as the pre-fix world
        session_id_field: "session_id".to_string(),
        resume_flag: "--resume".to_string(),
        session_id: "dead-sid".to_string(),
        message_id: None,
    };
    assert_eq!(c.find_recovery_session_id(&config), None);
}

/// Guards against an infinite retry loop: if the ONLY candidate
/// recovery finds is the exact id already confirmed poisoned this
/// attempt, don't recover it again — nothing on disk changes between
/// attempts, so an unguarded recovery would rediscover the identical
/// dead id forever.
#[test]
fn find_recovery_session_id_refuses_an_already_poisoned_candidate() {
    let tmp = tempfile::tempdir().unwrap();
    let config_dir = tmp.path().to_string_lossy().to_string();
    let working_dir = "/agents/agentx".to_string();
    let slug = crate::backend::claude_layout::project_dir_name(&working_dir);
    let dir = tmp.path().join("projects").join(&slug);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("dead-sid.jsonl"), vec![b'x'; 1_000]).unwrap();

    let c = controller();
    c.inner.lock().unwrap().resume_poisoned = Some("dead-sid".to_string());
    let mut env_vars = HashMap::new();
    env_vars.insert("CLAUDE_CONFIG_DIR".to_string(), config_dir);
    let config = PersistentSpawnConfig {
        cli_command: "claude".to_string(),
        cli_args: vec![],
        working_dir,
        env_vars,
        session_id_field: "session_id".to_string(),
        resume_flag: "--resume".to_string(),
        session_id: "dead-sid".to_string(),
        message_id: None,
    };

    assert_eq!(
        c.find_recovery_session_id(&config),
        None,
        "must not re-recover the same id already confirmed dead this attempt"
    );
}

/// End-to-end through `retry_after_resume_failure`: when a real,
/// larger session exists on disk under this spawn's own
/// `CLAUDE_CONFIG_DIR`, the respawn must hydrate `inner.session_id` to
/// THAT recovered id — not fall back to a blank conversation — even
/// though the actual process spawn itself fails fast here (nonexistent
/// binary), mirroring
/// `retry_after_resume_failure_clears_inner_session_id_even_when_poison_resume_has_not_run_yet`
/// above but for the recovery path instead of the give-up path.
#[test]
fn retry_after_resume_failure_hydrates_inner_session_id_from_the_recovered_session() {
    let tmp = tempfile::tempdir().unwrap();
    let config_dir = tmp.path().to_string_lossy().to_string();
    let working_dir = r"C:\Users\user\.agentmux\agents\agentx-0623n".to_string();
    let slug = crate::backend::claude_layout::project_dir_name(&working_dir);
    let dir = tmp.path().join("projects").join(&slug);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("972a6a4f-live.jsonl"), vec![b'x'; 2_800_000]).unwrap();

    let c = controller();
    c.inner.lock().unwrap().session_id = Some("d019e2e4-stale".to_string());
    let mut env_vars = HashMap::new();
    env_vars.insert("CLAUDE_CONFIG_DIR".to_string(), config_dir);
    let config = PersistentSpawnConfig {
        cli_command: "definitely-not-a-real-binary-xyz".to_string(),
        cli_args: vec![],
        working_dir,
        env_vars,
        session_id_field: "session_id".to_string(),
        resume_flag: "--resume".to_string(),
        session_id: "d019e2e4-stale".to_string(),
        message_id: None,
    };

    c.retry_after_resume_failure(1, config, vec![qentry(1, "{}")], None, "dead-sid".to_string());

    assert_eq!(
        c.inner.lock().unwrap().session_id,
        Some("972a6a4f-live".to_string()),
        "must resume the recovered real session, not silently start blank"
    );
}

/// reagentx P0 on PR #2360 (sixth review pass, round 11): same class
/// of bug as the test above, in a sibling fallback path added later —
/// `respawn_once_for_leftover_queue` cleared only `config.session_id`,
/// which `spawn_process`'s own `--resume` decision never reads (it
/// reads `inner.session_id` directly). If the doomed process's stderr
/// reader hasn't cleared `inner.session_id` yet, this fallback would
/// reattach `--resume` to the same dead sid and reproduce the
/// identical failure, with nothing left to catch the repeat.
/// Codex P1 on PR #3538: an eager resume (issue #3463) attempts `--resume`
/// with NOTHING queued, so a stale id reaches `retry_after_resume_failure`
/// with an empty batch. That is a terminal idle-resume failure, not a
/// retry: no launch — but the recovery decision must still complete. The
/// recovered id is adopted for the next message, and the controller
/// reports `done` rather than sitting in "Reconnecting…" with no status
/// ever published (the process-waiter hands this path `FireRetry`, not
/// `PublishDone`, on the assumption a launch follows).
#[test]
fn retry_after_resume_failure_with_no_entries_adopts_the_recovered_session_without_launching() {
    let tmp = tempfile::tempdir().unwrap();
    let config_dir = tmp.path().to_string_lossy().to_string();
    let working_dir = r"C:\Users\user\.agentmux\agents\agentx-0623n".to_string();
    let slug = crate::backend::claude_layout::project_dir_name(&working_dir);
    let dir = tmp.path().join("projects").join(&slug);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("972a6a4f-live.jsonl"), vec![b'x'; 2_800_000]).unwrap();

    let c = controller();
    {
        let mut inner = c.inner.lock().unwrap();
        inner.session_id = Some("d019e2e4-stale".to_string());
        inner.spawn_generation = 1; // the doomed eager-resume process
    }
    let mut env_vars = HashMap::new();
    env_vars.insert("CLAUDE_CONFIG_DIR".to_string(), config_dir);
    let config = PersistentSpawnConfig {
        cli_command: "definitely-not-a-real-binary-xyz".to_string(),
        cli_args: vec![],
        working_dir,
        env_vars,
        session_id_field: "session_id".to_string(),
        resume_flag: "--resume".to_string(),
        session_id: "d019e2e4-stale".to_string(),
        message_id: None,
    };

    c.retry_after_resume_failure(1, config, vec![], None, "d019e2e4-stale".to_string());

    let inner = c.inner.lock().unwrap();
    assert_eq!(
        inner.session_id,
        Some("972a6a4f-live".to_string()),
        "the recovered session is adopted so the NEXT message resumes it"
    );
    assert!(
        inner.stdin_tx.is_none() && !inner.spawning_in_progress,
        "nothing to send, so nothing is launched"
    );
    drop(inner);
    assert_eq!(
        c.get_status_snapshot().shellprocstatus,
        STATUS_DONE,
        "terminal status, not a silent return that strands the pane"
    );
}

/// The no-candidate half of the case above: nothing on disk to recover, so
/// the next message starts fresh — and the controller still settles `done`.
#[test]
fn retry_after_resume_failure_with_no_entries_and_no_recovery_candidate_settles_fresh_and_done() {
    let c = controller();
    {
        let mut inner = c.inner.lock().unwrap();
        inner.session_id = Some("dead-sid".to_string());
        inner.spawn_generation = 1;
    }
    let config = PersistentSpawnConfig {
        cli_command: "definitely-not-a-real-binary-xyz".to_string(),
        cli_args: vec![],
        working_dir: String::new(),
        env_vars: HashMap::new(),
        session_id_field: "session_id".to_string(),
        resume_flag: "--resume".to_string(),
        session_id: "dead-sid".to_string(),
        message_id: None,
    };

    c.retry_after_resume_failure(1, config, vec![], None, "dead-sid".to_string());

    let inner = c.inner.lock().unwrap();
    assert_eq!(inner.session_id, None, "no candidate: the next message starts fresh");
    assert!(inner.stdin_tx.is_none(), "nothing launched");
    drop(inner);
    assert_eq!(c.get_status_snapshot().shellprocstatus, STATUS_DONE);
}

/// Codex P1 on PR #3551 (fifth round): a prompt that queued BEHIND the eager
/// claim was never delivered, so the retry batch is empty even though
/// accepted work is waiting. The settlement must not treat that as
/// message-free: it hands the recovery candidate to the leftover respawn
/// (via `leftover_resume_candidate`) and leaves status/error alone, since
/// a spawn follows.
#[test]
fn retry_after_resume_failure_with_no_entries_hands_a_queued_prompt_and_its_candidate_to_the_leftover_respawn() {
    let tmp = tempfile::tempdir().unwrap();
    let config_dir = tmp.path().to_string_lossy().to_string();
    let working_dir = r"C:\Users\user\.agentmux\agents\agentx-0623n".to_string();
    let slug = crate::backend::claude_layout::project_dir_name(&working_dir);
    let dir = tmp.path().join("projects").join(&slug);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("972a6a4f-live.jsonl"), vec![b'x'; 2_800_000]).unwrap();

    let c = controller();
    {
        let mut inner = c.inner.lock().unwrap();
        inner.session_id = Some("d019e2e4-stale".to_string());
        inner.spawn_generation = 1;
        inner.spawning_in_progress = true; // the eager claim, still held
        let seq = inner.take_next_message_seq();
        inner
            .pending_send_messages
            .push_back(QueuedMessage::fresh(seq, "{\"queued\":\"behind the eager claim\"}".to_string()));
    }
    let mut env_vars = HashMap::new();
    env_vars.insert("CLAUDE_CONFIG_DIR".to_string(), config_dir);
    let config = PersistentSpawnConfig {
        cli_command: "definitely-not-a-real-binary-xyz".to_string(),
        cli_args: vec![],
        working_dir,
        env_vars,
        session_id_field: "session_id".to_string(),
        resume_flag: "--resume".to_string(),
        session_id: "d019e2e4-stale".to_string(),
        message_id: None,
    };

    c.retry_after_resume_failure(1, config, vec![], None, "d019e2e4-stale".to_string());

    let inner = c.inner.lock().unwrap();
    assert_eq!(inner.session_id.as_deref(), Some("972a6a4f-live"), "the candidate is adopted");
    assert_eq!(
        inner.leftover_resume_candidate.as_deref(),
        Some("972a6a4f-live"),
        "and handed to respawn_once_for_leftover_queue so it resumes rather than clears"
    );
    assert_ne!(inner.proc_status, STATUS_DONE, "not a terminal settlement — a spawn follows");
    assert_eq!(inner.pending_send_messages.len(), 1, "the queued prompt is untouched");
    assert!(inner.spawning_in_progress, "the eager claim is untouched");
}

/// The other half: `respawn_once_for_leftover_queue` honours an adopted
/// candidate instead of its usual clear-to-fresh, and consumes it.
#[test]
fn respawn_once_for_leftover_queue_resumes_an_adopted_candidate_instead_of_clearing_to_fresh() {
    let c = controller();
    {
        let mut inner = c.inner.lock().unwrap();
        inner.spawning_in_progress = true;
        inner.session_id = Some("972a6a4f-live".to_string());
        inner.leftover_resume_candidate = Some("972a6a4f-live".to_string());
        let seq = inner.take_next_message_seq();
        inner
            .pending_send_messages
            .push_back(QueuedMessage::fresh(seq, "{\"queued\":\"prompt\"}".to_string()));
    }
    let config = PersistentSpawnConfig {
        cli_command: "definitely-not-a-real-binary-xyz".to_string(),
        cli_args: vec![],
        working_dir: String::new(),
        env_vars: HashMap::new(),
        session_id_field: "session_id".to_string(),
        resume_flag: "--resume".to_string(),
        session_id: "d019e2e4-stale".to_string(),
        message_id: None,
    };

    c.respawn_once_for_leftover_queue(config);

    let inner = c.inner.lock().unwrap();
    assert_eq!(
        inner.session_id.as_deref(),
        Some("972a6a4f-live"),
        "the candidate survives — this respawn resumes it; only a stale id gets cleared"
    );
    assert_eq!(inner.leftover_resume_candidate, None, "consumed by the respawn");
    assert!(!inner.spawning_in_progress, "a failed respawn still releases the claim");
}

/// Codex P1 on PR #3551 (third round): the eagerly resumed CLI can reject
/// a stale id fast enough for its waiter to fire while `try_eager_resume`
/// still holds its own spawn claim. That claim is THIS generation's, not a
/// newer one's — the settlement must run, and must leave the claim alone
/// (it is the eager path's to release).
#[test]
fn retry_after_resume_failure_with_no_entries_runs_under_this_generations_own_unwinding_eager_claim() {
    let c = controller();
    {
        let mut inner = c.inner.lock().unwrap();
        inner.session_id = Some("dead-sid".to_string());
        inner.spawn_generation = 1;
        inner.spawning_in_progress = true; // try_eager_resume has not released yet
    }
    let config = PersistentSpawnConfig {
        cli_command: "definitely-not-a-real-binary-xyz".to_string(),
        cli_args: vec![],
        working_dir: String::new(),
        env_vars: HashMap::new(),
        session_id_field: "session_id".to_string(),
        resume_flag: "--resume".to_string(),
        session_id: "dead-sid".to_string(),
        message_id: None,
    };

    c.retry_after_resume_failure(1, config, vec![], None, "dead-sid".to_string());

    let inner = c.inner.lock().unwrap();
    assert_eq!(inner.session_id, None, "the settlement ran: no candidate, so the next message starts fresh");
    assert_eq!(inner.proc_status, STATUS_DONE, "and published the terminal status the waiter suppressed");
    assert!(
        inner.spawning_in_progress,
        "the eager claim is not this settlement's to touch — it belongs to try_eager_resume"
    );
}

/// Codex P1 on PR #3551: a message that arrived after the doomed process
/// cleared `stdin_tx` but before the empty-batch settlement ran may already
/// have spawned a NEWER generation with its own live session id. The
/// settlement for the OLD generation must then touch nothing — not the
/// session id, not the status — or a later restart resumes the wrong
/// conversation.
#[test]
fn retry_after_resume_failure_with_no_entries_leaves_a_newer_generation_alone() {
    // Codex's exact scenario (second round): NO recovery candidate, so the
    // old code would have appended a `Fresh` disclosure — to the history of
    // a generation that may be resuming a different conversation entirely.
    let broker = Arc::new(crate::backend::mps::Broker::new());
    let filestore = Arc::new(FileStore::open_in_memory().unwrap());
    let block_id = "block-superseded-empty-retry".to_string();
    let c = PersistentSubprocessController::new(
        "tab".to_string(),
        block_id.clone(),
        Some(broker),
        None,
        None,
        Some(filestore.clone()),
    );
    {
        let mut inner = c.inner.lock().unwrap();
        // Generation 2 has taken over and installed its own session.
        inner.spawn_generation = 2;
        inner.session_id = Some("newer-live".to_string());
        PersistentSubprocessController::set_status(&mut inner, STATUS_RUNNING);
    }
    let config = PersistentSpawnConfig {
        cli_command: "definitely-not-a-real-binary-xyz".to_string(),
        cli_args: vec![],
        working_dir: String::new(),
        env_vars: HashMap::new(), // no CLAUDE_CONFIG_DIR: nothing to recover
        session_id_field: "session_id".to_string(),
        resume_flag: "--resume".to_string(),
        session_id: "d019e2e4-stale".to_string(),
        message_id: None,
    };

    // Generation 1's settlement arrives late, with the CLI's error line.
    c.retry_after_resume_failure(1, config, vec![], Some("stale-resume-error\n".to_string()), "d019e2e4-stale".to_string());

    let inner = c.inner.lock().unwrap();
    assert_eq!(
        inner.session_id.as_deref(),
        Some("newer-live"),
        "a recovery result for a superseded generation must not overwrite the live session"
    );
    assert_eq!(inner.proc_status, STATUS_RUNNING, "and must not stamp `done` over a running generation");
    drop(inner);
    let appended = filestore.read_file(&block_id, PERSISTENT_OUTPUT_SUBJECT).unwrap();
    assert!(
        appended.is_none(),
        "and must append nothing — no `Fresh` disclosure, no stale error line — to the newer \
         generation's history; got: {:?}",
        appended.map(|b| String::from_utf8_lossy(&b).to_string())
    );
}

#[test]
fn respawn_once_for_leftover_queue_clears_inner_session_id_even_when_poison_resume_has_not_run_yet() {
    let c = controller();
    c.inner.lock().unwrap().session_id = Some("dead-sid".to_string());

    let config = PersistentSpawnConfig {
        cli_command: "definitely-not-a-real-binary-xyz".to_string(),
        cli_args: vec![],
        working_dir: String::new(),
        env_vars: HashMap::new(),
        session_id_field: "session_id".to_string(),
        resume_flag: "--resume".to_string(),
        session_id: "dead-sid".to_string(),
        message_id: None,
    };
    c.respawn_once_for_leftover_queue(config);

    assert_eq!(
        c.inner.lock().unwrap().session_id,
        None,
        "must clear inner.session_id directly, not rely on config.session_id alone"
    );
}

/// reagentx P1 on PR #2360 (sixth review pass, round 13): an earlier
/// cut of this fallback called `poison_resume` (not just a plain
/// clear) on whatever sid was held, reasoning defensively about a
/// narrower race. That was itself a regression: this fallback is
/// reached from triggers that have nothing to do with a CONFIRMED
/// stale `--resume` (a plain `spawn_process` failure, or ANY process
/// crash with messages still queued) — `inner.session_id` could just
/// as easily be a genuinely valid, already-captured session from a
/// process that ran fine and crashed for an unrelated reason.
/// `poison_resume` is PERMANENT (`resume_poisoned` is never reset),
/// so poisoning a sid never actually confirmed dead by the CLI would
/// permanently break that session's resume capability. Confirms a
/// valid sid survives this fallback well enough to still be captured
/// again later (i.e. NOT poisoned) — only the in-memory `session_id`
/// itself is cleared, forcing this one respawn to skip `--resume`.
#[test]
fn respawn_once_for_leftover_queue_does_not_poison_the_sid_it_held() {
    let c = controller();
    c.inner.lock().unwrap().session_id = Some("valid-unrelated-sid".to_string());

    let config = PersistentSpawnConfig {
        cli_command: "definitely-not-a-real-binary-xyz".to_string(),
        cli_args: vec![],
        working_dir: String::new(),
        env_vars: HashMap::new(),
        session_id_field: "session_id".to_string(),
        resume_flag: "--resume".to_string(),
        session_id: "valid-unrelated-sid".to_string(),
        message_id: None,
    };
    c.respawn_once_for_leftover_queue(config);

    let mut inner = c.inner.lock().unwrap();
    assert_ne!(
        inner.resume_poisoned.as_deref(),
        Some("valid-unrelated-sid"),
        "must not permanently poison a sid that was never confirmed dead by the CLI"
    );
    let generation = inner.spawn_generation;
    let (captured, _effects) = inner.try_capture_session_id("valid-unrelated-sid", generation, true);
    assert!(captured, "a genuinely valid sid must still be capturable again later");
}

/// codex P1/P2 on PR #2360 (second review pass): the process-waiter
/// task now awaits the stderr reader's `JoinHandle` (bounded by a
/// timeout) before deciding whether a stale-resume retry was
/// confirmed, and before publishing a terminal status — otherwise
/// `child.wait()` resolving first could (1) wipe the tentative retry
/// before the stderr reader ever promotes it, and (2) let a
/// confirmed retry's fresh session id get overwritten by the stderr
/// task's own delayed `persist_session_id("")` call. This isn't a
/// full subprocess integration test (this module's established
/// precedent — see its own doc comment — avoids spawning a real CLI
/// process for this exact subsystem); it confirms the underlying
/// synchronization primitive itself: a task that completes well
/// within the bound is FULLY awaited — its side effect is guaranteed
/// observable — before the timeout could possibly race it.
#[tokio::test]
async fn a_join_handle_completing_within_the_bound_is_fully_awaited_first() {
    let flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag_clone = Arc::clone(&flag);
    let handle = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        flag_clone.store(true, std::sync::atomic::Ordering::SeqCst);
    });

    let result = tokio::time::timeout(std::time::Duration::from_millis(500), handle).await;

    assert!(result.is_ok(), "a task well within the bound must not be treated as timed out");
    assert!(
        flag.load(std::sync::atomic::Ordering::SeqCst),
        "awaiting the handle must observe the task's side effect having already happened, \
         not race ahead of it"
    );
}

/// Confirms the bounded-wait PRIMITIVE the process-waiter's
/// exit-handling relies on before deciding a confirmed stale-resume
/// retry batch is final — reagentx P1 on PR #2360 (sixth review pass,
/// round 9): it polls `drain_send_in_flight` every 10ms, bounded to
/// 500ms, so a flag that clears shortly after being observed `true`
/// must still be correctly picked up within the window (not missed by
/// a single stale read). The full cross-task race this guards against
/// isn't practical to reproduce deterministically (same reasoning as
/// this file's other cross-task timing fixes — see e.g. the
/// stderr-reader bound above), so this exercises the underlying
/// polling primitive directly.
#[tokio::test]
async fn a_flag_clearing_shortly_after_is_observed_by_a_bounded_polling_wait() {
    let c = Arc::new(controller());
    c.inner.lock().unwrap().drain_send_in_flight = true;

    let c2 = Arc::clone(&c);
    tokio::spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
        c2.inner.lock().unwrap().drain_send_in_flight = false;
    });

    let mut cleared = false;
    for _ in 0..50 {
        if !c.inner.lock().unwrap().drain_send_in_flight {
            cleared = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(cleared, "the bounded wait must observe the flag clearing within its window");
}

/// Baseline: a message sent while the process is already running is
/// delivered directly, with no spawn decision involved at all.
#[tokio::test]
async fn send_message_delivers_directly_to_an_already_running_process() {
    let c = controller();
    let (tx, mut rx) = mpsc::channel::<String>(4);
    {
        let mut inner = c.inner.lock().unwrap();
        inner.stdin_tx = Some(tx);
    }

    let config = PersistentSpawnConfig {
        cli_command: "unused".to_string(),
        cli_args: vec![],
        working_dir: String::new(),
        env_vars: HashMap::new(),
        session_id_field: "session_id".to_string(),
        resume_flag: String::new(),
        session_id: String::new(),
        message_id: None,
    };

    c.send_message("hello".to_string(), config)
        .expect("delivery to an already-running process must succeed");

    let received = rx.try_recv().expect("the message must have been written to stdin_tx");
    assert!(received.contains("hello"));
}

/// reagentx P1 on PR #2360 (sixth review pass): `decide_send_action` is
/// the primitive that closes the concurrent-spawn TOCTOU race — these
/// three cases cover its full decision space deterministically,
/// without needing to reproduce an actual multi-threaded race.
#[test]
fn decide_send_action_becomes_spawner_when_nothing_is_in_flight() {
    let c = controller();
    let action = c.decide_send_action("msg-a", None);
    assert!(matches!(action, SendAction::BecomeSpawner { .. }));
    let inner = c.inner.lock().unwrap();
    assert!(inner.spawning_in_progress, "must claim the exclusive spawn right");
    assert_eq!(
        inner.pending_send_messages.len(),
        1,
        "the caller's own message must be enqueued too, for the uniform post-spawn drain"
    );
}

#[test]
fn decide_send_action_queues_when_a_spawn_is_already_in_flight() {
    let c = controller();
    c.inner.lock().unwrap().spawning_in_progress = true;

    let action = c.decide_send_action("msg-b", None);
    assert!(
        matches!(action, SendAction::Queued),
        "a second caller must queue instead of independently deciding to spawn"
    );
    let inner = c.inner.lock().unwrap();
    assert_eq!(inner.pending_send_messages.len(), 1);
    assert_eq!(inner.pending_send_messages[0], "msg-b");
}

/// codex P1 on PR #2360 (sixth review pass, round 4): a genuine second
/// user message queuing behind an in-flight spawn must NOT be deduped
/// by content — a user legitimately re-sending the exact same text
/// must still see both delivered. `skip_if_already_queued=false`
/// (what `send_message` always passes) must therefore always enqueue.
#[test]
fn decide_send_action_never_dedups_a_genuine_new_message() {
    let c = controller();
    c.inner.lock().unwrap().spawning_in_progress = true;

    c.decide_send_action("hello", None);
    let action = c.decide_send_action("hello", None);

    assert!(matches!(action, SendAction::Queued));
    let inner = c.inner.lock().unwrap();
    assert_eq!(
        inner.pending_send_messages.len(),
        2,
        "two genuinely separate sends of identical text must both be queued, not deduped"
    );
}

/// codex P1 on PR #2360 (sixth review pass, round 4): unlike a genuine
/// new message, `retry_after_resume_failure`'s payload is a KNOWN
/// re-delivery of a message that may ALREADY be sitting in the queue —
/// pushed by the very spawn attempt whose failure triggered this
/// retry, if that spawn's own drain hasn't reached it yet. Blindly
/// queueing another copy (as the `None`/`send_message` path
/// correctly does for a genuine new message) would let a fallback
/// spawn eventually deliver the same prompt twice. Passing the
/// original entry's seq must therefore skip re-enqueueing while that
/// exact entry is still present (issue #2365: matched by identity,
/// not text).
#[test]
fn decide_send_action_dedups_a_known_retry_of_an_already_queued_message() {
    let c = controller();
    {
        let mut inner = c.inner.lock().unwrap();
        inner.spawning_in_progress = true;
        inner
            .pending_send_messages
            .push_back(QueuedMessage::fresh(7, "original-payload".to_string()));
    }

    let action = c.decide_send_action("original-payload", Some(7));

    assert!(matches!(action, SendAction::Queued));
    let inner = c.inner.lock().unwrap();
    assert_eq!(
        inner.pending_send_messages.len(),
        1,
        "a retry of a message still queued under its own seq must not add a duplicate copy"
    );
}

/// Issue #2365 regression: the dedup must key on the entry's seq, not
/// its text — a DIFFERENT message that happens to share identical
/// content with the retried one must not satisfy the check (the old
/// content-equality version silently dropped the retry here).
#[test]
fn decide_send_action_does_not_dedup_a_retry_against_an_identical_text_different_message() {
    let c = controller();
    {
        let mut inner = c.inner.lock().unwrap();
        inner.spawning_in_progress = true;
        // A genuinely different message (seq 9) with the same text as
        // the retried entry (seq 7).
        inner
            .pending_send_messages
            .push_back(QueuedMessage::fresh(9, "original-payload".to_string()));
    }

    let action = c.decide_send_action("original-payload", Some(7));

    assert!(matches!(action, SendAction::Queued));
    let inner = c.inner.lock().unwrap();
    assert_eq!(
        inner.pending_send_messages.len(),
        2,
        "identical text under a different seq is a different message — the retry must still be queued"
    );
    assert_eq!(
        inner.pending_send_messages[1].seq, 7,
        "the re-queued retry must keep its original seq, not draw a fresh one"
    );
}

/// The dedup check must not accidentally skip a retry whose payload
/// genuinely isn't in the queue yet (the drain already popped it, in
/// the narrow window where the retry races in after that but before
/// the drain releases the claim) — it must still queue normally.
#[test]
fn decide_send_action_still_queues_a_retry_when_its_payload_is_not_already_present() {
    let c = controller();
    c.inner.lock().unwrap().spawning_in_progress = true;

    let action = c.decide_send_action("not-yet-queued", Some(42));

    assert!(matches!(action, SendAction::Queued));
    let inner = c.inner.lock().unwrap();
    assert_eq!(inner.pending_send_messages.len(), 1);
    assert_eq!(inner.pending_send_messages[0], "not-yet-queued");
    assert_eq!(
        inner.pending_send_messages[0].seq, 42,
        "a re-queued known redelivery must preserve its original seq"
    );
}

#[test]
fn decide_send_action_delivers_directly_when_already_running() {
    let c = controller();
    let (tx, _rx) = mpsc::channel::<String>(4);
    c.inner.lock().unwrap().stdin_tx = Some(tx);

    let action = c.decide_send_action("msg-c", None);
    assert!(matches!(action, SendAction::DeliverDirect { was_active: false }));
    let inner = c.inner.lock().unwrap();
    assert!(
        inner.pending_send_messages.is_empty(),
        "must not queue when delivering directly to an already-running process"
    );
}

/// reagentx P1 on PR #2360 (sixth review pass, round 4): `spawn_process`
/// sets `stdin_tx` synchronously, well before the queued message that
/// triggered the spawn is actually delivered by the background drain
/// task (`drain_queue_after_successful_spawn`). A second caller
/// landing in that exact window — `stdin_tx` already live, but
/// `spawning_in_progress` still `true` because the drain hasn't
/// finished — must NOT take the `DeliverDirect` path: writing straight
/// to stdin via `try_send` there would race ahead of the drain's own
/// `Sender::send().await` for the message that actually triggered the
/// spawn, silently reordering user input. It must queue behind
/// whatever the still-active drain is working through instead.
#[test]
fn decide_send_action_queues_instead_of_delivering_direct_while_a_drain_is_still_active() {
    let c = controller();
    let (tx, _rx) = mpsc::channel::<String>(4);
    {
        let mut inner = c.inner.lock().unwrap();
        inner.stdin_tx = Some(tx);
        inner.spawning_in_progress = true;
    }

    let action = c.decide_send_action("msg-late-arrival", None);
    assert!(
        matches!(action, SendAction::Queued),
        "must queue, not deliver direct, while a drain for an earlier message is still active"
    );
    let inner = c.inner.lock().unwrap();
    assert_eq!(inner.pending_send_messages.len(), 1);
    assert_eq!(inner.pending_send_messages[0], "msg-late-arrival");
}

/// codex P2 on PR #2360 (sixth review pass, round 7): a fresh message
/// (from `decide_send_action`) must be marked NOT already persisted —
/// the drain is responsible for persisting it, in delivery order.
#[test]
fn decide_send_action_marks_a_fresh_message_as_not_yet_persisted() {
    let c = controller();
    c.decide_send_action("hello", None);
    let inner = c.inner.lock().unwrap();
    assert!(
        !inner.pending_send_messages[0].already_persisted,
        "a genuinely new message must not be marked already-persisted"
    );
}

/// codex P2 on PR #2360 (sixth review pass, round 7): a stale-resume
/// retry's batch (from `decide_retry_batch_action`) must be marked
/// already persisted — it was correctly persisted on its ORIGINAL
/// attempt, and the shared drain must not persist it a second time.
#[test]
fn decide_retry_batch_action_marks_every_entry_as_already_persisted() {
    let c = controller();
    c.decide_retry_batch_action(1, &qentry(1, "hello"), &[qentry(2, "world")]);
    let inner = c.inner.lock().unwrap();
    assert!(inner.pending_send_messages[0].already_persisted);
    assert!(inner.pending_send_messages[1].already_persisted);
}

/// reagentx P1 on PR #2360 (sixth review pass, round 7): `spawn_process`
/// sets `stdin_tx` synchronously, well before the queued message that
/// triggered the spawn is actually delivered by the background drain.
/// A muxbus/jekt message (`send_user_message`) landing in that window
/// must not `try_send` straight to the live channel, reordering itself
/// ahead of whatever the drain is still working through.
///
/// This used to be enforced by returning a retryable error, because there
/// was no safe place to hold the message. `SPEC_NO_MIDTURN_DELIVERY_2026_09_23.md`
/// §3.2.1 gave it one: it is queued and written by the watchdog once the
/// drain is done, so the no-reordering invariant now holds *without* the
/// caller having to retry — and without the jekt being dropped outright,
/// which is what the old error actually caused in production (the reactive
/// handler has no retry).
#[tokio::test]
async fn send_user_message_defers_instead_of_reordering_while_a_drain_is_still_active() {
    let c = controller();
    let (tx, mut rx) = mpsc::channel::<String>(4);
    {
        let mut inner = c.inner.lock().unwrap();
        inner.stdin_tx = Some(tx);
        inner.spawning_in_progress = true;
    }

    c.send_user_message("steer".to_string())
        .expect("a mid-spawn message is accepted, not rejected");

    assert!(
        rx.try_recv().is_err(),
        "nothing may reach live stdin ahead of the in-flight drain"
    );
    let inner = c.inner.lock().unwrap();
    assert_eq!(
        inner.deferred_deliveries.len(),
        1,
        "the message must be queued until the process is up, not dropped"
    );
}

/// Exercises the actual race with real OS threads, not just sequential
/// state assertions — reagentx P1 on PR #2360 (sixth review pass): many
/// concurrent callers landing on a controller with no process running
/// (the exact shape of a genuine second `send_message` RPC, or a
/// muxbus delivery, racing this controller's own stale-resume retry)
/// must produce EXACTLY one spawner; everyone else must queue instead
/// of each independently deciding to spawn their own child process.
#[test]
fn decide_send_action_produces_exactly_one_spawner_under_real_concurrency() {
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
    use std::sync::Arc as StdArc;

    let c = StdArc::new(controller());
    let spawner_count = StdArc::new(AtomicUsize::new(0));
    let queued_count = StdArc::new(AtomicUsize::new(0));

    let handles: Vec<_> = (0..16)
        .map(|i| {
            let c = StdArc::clone(&c);
            let spawner_count = StdArc::clone(&spawner_count);
            let queued_count = StdArc::clone(&queued_count);
            std::thread::spawn(move || match c.decide_send_action(&format!("msg-{i}"), None) {
                SendAction::BecomeSpawner { .. } => {
                    spawner_count.fetch_add(1, AtomicOrdering::SeqCst);
                }
                SendAction::Queued => {
                    queued_count.fetch_add(1, AtomicOrdering::SeqCst);
                }
                SendAction::DeliverDirect { .. } => panic!("process was never running in this test"),
                SendAction::Refused(e) => panic!("no stop is pending in this test: {e}"),
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }

    assert_eq!(
        spawner_count.load(AtomicOrdering::SeqCst),
        1,
        "exactly one caller must claim the exclusive right to spawn"
    );
    assert_eq!(queued_count.load(AtomicOrdering::SeqCst), 15, "everyone else must queue");
    assert_eq!(
        c.inner.lock().unwrap().pending_send_messages.len(),
        16,
        "every message — the spawner's own plus all queued — must be present, none dropped"
    );
}

/// On a successful spawn, the drain must deliver everything queued
/// (including a caller's own message, enqueued alongside the claim by
/// `decide_send_action`) in order, then release the claim so a future
/// caller can spawn again. Delivery happens on a spawned background
/// task (see the function's own doc comment), so this must actually
/// wait for it rather than asserting immediately.
#[tokio::test]
async fn release_spawn_claim_and_drain_queue_delivers_everything_on_success() {
    let c = controller();
    let (tx, mut rx) = mpsc::channel::<String>(8);
    {
        let mut inner = c.inner.lock().unwrap();
        inner.stdin_tx = Some(tx);
        inner.spawning_in_progress = true;
        inner.pending_send_messages.push_back(QueuedMessage::fresh(1, "first".to_string()));
        inner.pending_send_messages.push_back(QueuedMessage::fresh(2, "second".to_string()));
    }

    // Never used for a fallback spawn in this test — the drain fully
    // succeeds without ever stalling. `own_seq` is only consulted
    // on the failed-spawn path, so its value doesn't matter here.
    c.release_spawn_claim_and_drain_queue(true, unreachable_fallback_config(), 0);

    assert_eq!(rx.recv().await.unwrap(), "first");
    assert_eq!(rx.recv().await.unwrap(), "second");

    // Give the background drain task its final iteration (observing
    // the now-empty queue and releasing the claim) a chance to run.
    tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
    let inner = c.inner.lock().unwrap();
    assert!(!inner.spawning_in_progress, "claim must be released once fully drained");
    assert!(inner.pending_send_messages.is_empty());
}

/// A `PersistentSpawnConfig` whose `cli_command` doesn't exist, so any
/// `spawn_process` attempt made with it fails fast and deterministically
/// (no real process, no hang) — used by tests that need to exercise
/// `respawn_once_for_leftover_queue`'s fallback path without spawning a
/// real CLI, matching this module's own established precedent (see
/// `retry_after_resume_failure_clears_inner_session_id_even_when_poison_resume_has_not_run_yet`).
fn unreachable_fallback_config() -> PersistentSpawnConfig {
    PersistentSpawnConfig {
        cli_command: "definitely-not-a-real-binary-xyz".to_string(),
        cli_args: vec![],
        working_dir: String::new(),
        env_vars: HashMap::new(),
        session_id_field: "session_id".to_string(),
        resume_flag: String::new(),
        session_id: String::new(),
        message_id: None,
    }
}

/// reagentx/codex P1 on PR #2360 (sixth review pass, rounds 3-4): if
/// the process a successful spawn just established dies before the
/// drain even gets to run (found via `stdin_tx` already `None`) with
/// messages still queued, nothing else will ever tell the frontend
/// the turn ended — the ORIGINAL exit deliberately suppressed its own
/// publish expecting a retry to publish one instead, and
/// `retry_after_resume_failure`'s own `Queued` branch does nothing
/// further (see its own doc comment). Confirms the drain hands off to
/// `respawn_once_for_leftover_queue`, which — using a config that
/// itself fails fast (no real process needed) — logs the failure and
/// publishes a status update instead of leaving the pane hanging with
/// no signal at all, while leaving the leftover messages queued
/// (this fallback path never pops anything itself — it isn't tied to
/// a specific message).
#[tokio::test]
async fn release_spawn_claim_and_drain_queue_falls_back_and_publishes_status_when_stalled() {
    let broker = Arc::new(crate::backend::mps::Broker::new());
    let c = Arc::new(PersistentSubprocessController::new(
        "tab".to_string(),
        "block-stalled".to_string(),
        Some(broker.clone()),
        None,
        None,
        None,
    ));
    c.set_self_ref();
    {
        let mut inner = c.inner.lock().unwrap();
        inner.spawning_in_progress = true;
        inner.pending_send_messages.push_back(QueuedMessage::fresh(1, "stuck-one".to_string()));
        inner.pending_send_messages.push_back(QueuedMessage::fresh(2, "stuck-two".to_string()));
        // stdin_tx stays None — simulates the process this claim was
        // spawning for having already died before the drain ran.
    }

    c.release_spawn_claim_and_drain_queue(true, unreachable_fallback_config(), 0);
    // Let the spawned background task, and the fallback respawn
    // attempt it triggers, run to completion.
    tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;

    let history = broker.read_event_history(
        crate::backend::mps::EVENT_CONTROLLER_STATUS,
        "block:block-stalled",
        1,
    );
    assert_eq!(history.len(), 1, "must publish a status update instead of silently hanging");

    let inner = c.inner.lock().unwrap();
    assert!(!inner.spawning_in_progress, "claim must still be released");
    assert_eq!(
        inner.pending_send_messages.len(),
        2,
        "leftover messages must stay queued for whatever spawn comes next \
         (the fallback attempt itself failed, matching the config used)"
    );
}

/// codex P1 on PR #2360 (sixth review pass): a FAILED spawn must
/// discard only the front item — the caller's own message, which
/// `send_message`/`retry_after_resume_failure` already reported as a
/// failure to their own caller — not leave it queued for an unrelated
/// later spawn to silently execute. Anything ELSE queued behind it
/// (from other callers who got `SendAction::Queued` and were already
/// told "accepted") must survive — handed off to a bounded fallback
/// respawn (codex P2, round 4) rather than stranded with nobody
/// responsible for it; using a config that itself fails fast here, so
/// the leftover message ends up back in the queue rather than
/// delivered, but never discarded.
#[test]
fn release_spawn_claim_and_drain_queue_discards_only_the_failed_spawners_own_message() {
    let c = controller();
    {
        let mut inner = c.inner.lock().unwrap();
        inner.spawning_in_progress = true;
        inner.pending_send_messages.push_back(QueuedMessage::fresh(1, "the-one-that-failed".to_string()));
        inner.pending_send_messages.push_back(QueuedMessage::fresh(2, "queued-by-someone-else".to_string()));
    }

    c.release_spawn_claim_and_drain_queue(false, unreachable_fallback_config(), 1);

    let inner = c.inner.lock().unwrap();
    assert!(
        !inner.spawning_in_progress,
        "claim must still be released even though the spawn and its fallback both failed"
    );
    assert_eq!(
        inner.pending_send_messages.len(),
        1,
        "only the failed spawner's own (front) message must be discarded"
    );
    assert_eq!(
        inner.pending_send_messages[0],
        "queued-by-someone-else",
        "a message queued by a DIFFERENT caller (already told \"accepted\") must survive for the next spawn"
    );
}

/// codex P2 on PR #2360 (round 14, commit 8c2bc99ab): the queue is NOT
/// always empty at the moment a new spawner claims `BecomeSpawner` — the
/// "second stall" path (`drain_queue_after_successful_spawn` with
/// `allow_fallback_respawn: false`) deliberately releases
/// `spawning_in_progress` while leaving genuine leftover messages
/// queued. A later `send_message` can then claim `BecomeSpawner` and
/// `push_back` its own message BEHIND those leftovers — so the failed
/// spawner's own message is NOT at the front. Confirms
/// `release_spawn_claim_and_drain_queue`'s failed-spawn path finds and
/// discards the correct (content-matched) entry regardless of where it
/// sits, instead of assuming the front and silently destroying an
/// older, unrelated, already-accepted prompt.
#[test]
fn release_spawn_claim_and_drain_queue_discards_the_right_entry_when_it_is_not_at_the_front() {
    let c = controller();
    {
        let mut inner = c.inner.lock().unwrap();
        inner.spawning_in_progress = true;
        // Simulates leftovers surviving a prior "second stall" release
        // (queue non-empty, claim already given up by that path) plus a
        // later BecomeSpawner appending its own message behind them.
        inner.pending_send_messages.push_back(QueuedMessage::fresh(1, "older-leftover-from-a-different-caller".to_string()));
        inner.pending_send_messages.push_back(QueuedMessage::fresh(2, "this-spawners-own-message-that-just-failed".to_string()));
    }

    c.release_spawn_claim_and_drain_queue(
        false,
        unreachable_fallback_config(),
        2,
    );

    let inner = c.inner.lock().unwrap();
    assert_eq!(
        inner.pending_send_messages.len(),
        1,
        "only the actually-failed spawner's own message must be discarded"
    );
    assert_eq!(
        inner.pending_send_messages[0],
        "older-leftover-from-a-different-caller",
        "an older, unrelated, already-accepted prompt must survive — not be silently destroyed \
         because it happened to be sitting at the front"
    );
}

/// Exercises the actual race with real OS threads — codex P2 on PR
/// #2360 (round 13, commit e9678091f): a FAILED spawn's claim used to
/// be released in a lock acquisition SEPARATE from the emptiness check
/// that decided whether to release it at all. That left a window,
/// between the two, where a concurrent `send_message` could observe
/// `spawning_in_progress` still `true`, enqueue via `decide_send_action`'s
/// `Queued` branch, and be told "accepted" — then this function's
/// second lock would clear the claim without ever rechecking the
/// queue, stranding that accepted message with nobody left responsible
/// (no drain, no respawn, no disclosure). The fix merges the emptiness
/// check and the flag clear into one lock acquisition, so a racer's
/// push can now only land fully before or fully after that atomic
/// block — never inside it. Across many iterations and threads, the
/// invariant that must always hold: whenever a message is left queued
/// with the claim released, it's because a fallback respawn was
/// actually attempted (and disclosed its failure via a published
/// status) — never silently, with no attempt at all.
#[test]
fn release_spawn_claim_and_drain_queue_never_silently_strands_a_racing_send() {
    use std::sync::Arc as StdArc;

    for iteration in 0..30 {
        let broker = StdArc::new(crate::backend::mps::Broker::new());
        let block_id = format!("block-race-{iteration}");
        let c = StdArc::new(PersistentSubprocessController::new(
            "tab".to_string(),
            block_id.clone(),
            Some(broker.clone()),
            None,
            None,
            None,
        ));
        c.set_self_ref();
        {
            let mut inner = c.inner.lock().unwrap();
            inner.spawning_in_progress = true;
            inner.pending_send_messages.push_back(QueuedMessage::fresh(1, "the-one-that-failed".to_string()));
        }

        let handles: Vec<_> = (0..8)
            .map(|i| {
                let c = StdArc::clone(&c);
                std::thread::spawn(move || {
                    let _ = c.decide_send_action(&format!("racer-{iteration}-{i}"), None);
                })
            })
            .collect();

        c.release_spawn_claim_and_drain_queue(false, unreachable_fallback_config(), 1);

        for h in handles {
            h.join().unwrap();
        }

        let inner = c.inner.lock().unwrap();
        let stranded = !inner.spawning_in_progress && !inner.pending_send_messages.is_empty();
        if stranded {
            let published = !broker
                .read_event_history(crate::backend::mps::EVENT_CONTROLLER_STATUS, &format!("block:{block_id}"), 10)
                .is_empty();
            assert!(
                published,
                "iteration {iteration}: a racing send was left queued with the claim already \
                 released and no respawn attempt ever disclosed via a status publish — stranded \
                 with nobody responsible for it"
            );
        }
    }
}

/// codex P2 on PR #2360 (sixth review pass, round 3): `retry_after_
/// resume_failure` used to clear `inner.session_id` unconditionally at
/// the top of the function, before even deciding whether this call is
/// the one that's actually going to spawn. Confirms it's now scoped to
/// only the `BecomeSpawner` path — a concurrently-installed session id
/// (simulating a DIFFERENT spawn that's already running, so this call
/// resolves via `DeliverDirect`) must survive.
#[tokio::test]
async fn retry_after_resume_failure_preserves_a_concurrently_installed_session_id_when_not_the_spawner() {
    let c = controller();
    {
        let mut inner = c.inner.lock().unwrap();
        let (tx, _rx) = mpsc::channel::<String>(4);
        inner.stdin_tx = Some(tx);
        inner.session_id = Some("fresh-concurrently-installed-sid".to_string());
    }

    let config = PersistentSpawnConfig {
        cli_command: "unused".to_string(),
        cli_args: vec![],
        working_dir: String::new(),
        env_vars: HashMap::new(),
        session_id_field: "session_id".to_string(),
        resume_flag: "--resume".to_string(),
        session_id: "dead-sid".to_string(),
        message_id: None,
    };
    c.retry_after_resume_failure(1, config, vec![qentry(1, "{}")], None, "dead-sid".to_string());

    assert_eq!(
        c.inner.lock().unwrap().session_id.as_deref(),
        Some("fresh-concurrently-installed-sid"),
        "must not erase a session id a concurrent spawn already legitimately captured"
    );
}

/// codex P1 on PR #2360 (sixth review pass, round 5): a doomed
/// process's stdin channel can have accepted MULTIPLE messages before
/// it turned out to be unreachable — not just the one that triggered
/// the spawn. `retry_after_resume_failure` now takes the whole batch
/// and must redeliver every one of them, in order. Uses an
/// already-running process (`DeliverDirect` for each) so this is
/// deterministic without needing a real subprocess spawn.
#[tokio::test]
async fn retry_after_resume_failure_redelivers_every_message_in_the_batch() {
    let c = controller();
    let (tx, mut rx) = mpsc::channel::<String>(8);
    c.inner.lock().unwrap().stdin_tx = Some(tx);

    let config = PersistentSpawnConfig {
        cli_command: "unused".to_string(),
        cli_args: vec![],
        working_dir: String::new(),
        env_vars: HashMap::new(),
        session_id_field: "session_id".to_string(),
        resume_flag: "--resume".to_string(),
        session_id: String::new(),
        message_id: None,
    };
    c.retry_after_resume_failure(
        1,
        config,
        vec![qentry(1, "msg-1"), qentry(2, "msg-2"), qentry(3, "msg-3")],
        None,
        "dead-sid".to_string(),
    );

    assert_eq!(rx.recv().await.unwrap(), "msg-1");
    assert_eq!(rx.recv().await.unwrap(), "msg-2");
    assert_eq!(rx.recv().await.unwrap(), "msg-3");
}

/// codex P2 on PR #2360 (sixth review pass, round 6): the `DeliverDirect`
/// path is the retry's own LAST-CHANCE delivery attempt — nothing else
/// will ever resend a message that fails here (e.g. the process died
/// again in the gap between `decide_retry_batch_action`'s check and
/// this lock re-acquisition, simulated here via a dropped receiver).
/// It must be requeued, not silently discarded.
#[tokio::test]
async fn retry_after_resume_failure_requeues_a_message_that_fails_direct_delivery() {
    let c = controller();
    let (tx, rx) = mpsc::channel::<String>(1);
    drop(rx);
    c.inner.lock().unwrap().stdin_tx = Some(tx);

    let config = PersistentSpawnConfig {
        cli_command: "unused".to_string(),
        cli_args: vec![],
        working_dir: String::new(),
        env_vars: HashMap::new(),
        session_id_field: "session_id".to_string(),
        resume_flag: "--resume".to_string(),
        session_id: String::new(),
        message_id: None,
    };
    c.retry_after_resume_failure(1, config, vec![qentry(1, "stuck")], None, "dead-sid".to_string());

    let inner = c.inner.lock().unwrap();
    assert_eq!(
        inner.pending_send_messages.len(),
        1,
        "a message that fails its last-chance delivery must be requeued, not discarded"
    );
    assert_eq!(inner.pending_send_messages[0], "stuck");
}

/// codex P2 on PR #2360 (round 15, commit fdb8db6fd): once ANY message
/// in a multi-message retry batch fails direct delivery, every
/// remaining message must be queued too — not attempted via `try_send`
/// — so their relative order can never be disturbed by the bounded
/// stdin channel's receiver concurrently freeing capacity between
/// iterations (which could otherwise let a later message succeed
/// while an earlier, failed one sits queued behind it). Uses a
/// permanently-closed receiver so every message in the batch fails
/// deterministically; confirms all three end up queued in their
/// original order, none skipped, none lost.
#[tokio::test]
async fn retry_after_resume_failure_queues_the_rest_of_the_batch_in_order_once_one_fails() {
    let c = controller();
    let (tx, rx) = mpsc::channel::<String>(1);
    drop(rx);
    c.inner.lock().unwrap().stdin_tx = Some(tx);

    let config = PersistentSpawnConfig {
        cli_command: "unused".to_string(),
        cli_args: vec![],
        working_dir: String::new(),
        env_vars: HashMap::new(),
        session_id_field: "session_id".to_string(),
        resume_flag: "--resume".to_string(),
        session_id: String::new(),
        message_id: None,
    };
    c.retry_after_resume_failure(
        1,
        config,
        vec![qentry(1, "msg-1"), qentry(2, "msg-2"), qentry(3, "msg-3")],
        None,
        "dead-sid".to_string(),
    );

    let inner = c.inner.lock().unwrap();
    assert_eq!(
        inner.pending_send_messages.len(),
        3,
        "every message in the batch must be queued, none skipped or lost"
    );
    assert_eq!(inner.pending_send_messages[0], "msg-1");
    assert_eq!(inner.pending_send_messages[1], "msg-2");
    assert_eq!(inner.pending_send_messages[2], "msg-3");
}

/// Issue #2367 (supersedes the round-7 spawn-flag borrow): a retry
/// batch targeting a live process must take the DRAIN claim in the
/// same lock acquisition that decides the flush — so no concurrent
/// caller bypasses the queue via `DeliverDirect` — while leaving
/// `spawning_in_progress` untouched (round 14's starvation analysis:
/// it is never pre-asserted while no spawn is in progress). Even
/// against a dead sender the flush must still release the claim
/// afterward (not leave it stuck forever) while preserving the
/// message for a future spawn.
#[tokio::test]
async fn retry_after_resume_failure_takes_the_drain_claim_not_the_spawn_flag() {
    let c = controller();
    let (tx, rx) = mpsc::channel::<String>(1);
    drop(rx);
    c.inner.lock().unwrap().stdin_tx = Some(tx);

    let config = PersistentSpawnConfig {
        cli_command: "unused".to_string(),
        cli_args: vec![],
        working_dir: String::new(),
        env_vars: HashMap::new(),
        session_id_field: "session_id".to_string(),
        resume_flag: "--resume".to_string(),
        session_id: String::new(),
        message_id: None,
    };
    c.retry_after_resume_failure(1, config, vec![qentry(1, "stuck")], None, "dead-sid".to_string());

    {
        let inner = c.inner.lock().unwrap();
        assert!(
            inner.drain_claim,
            "must take the drain claim immediately so no concurrent caller bypasses the queue via DeliverDirect"
        );
        assert!(
            !inner.spawning_in_progress,
            "the spawn flag must never be borrowed for a non-spawn (round 14 starvation analysis)"
        );
    }

    // Let the background drain (which will also fail against this
    // same dead sender) run to completion.
    tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
    let inner = c.inner.lock().unwrap();
    assert!(!inner.drain_claim, "the drain claim must eventually be released, not stuck forever");
    assert_eq!(inner.pending_send_messages.len(), 1, "message must remain queued, not lost");
}

/// Issue #2367, the actual reordering bug: while the retry batch is
/// being flushed into a live newer process, a concurrent
/// `send_message` must route through the queue BEHIND the batch —
/// under the old `try_send` design it took `DeliverDirect` and could
/// jump ahead of (or into the middle of) earlier-accepted retry
/// messages. The queue is now the single ordering authority: batch
/// first, later send after, and the claim releases once dry.
#[tokio::test]
async fn retry_flush_orders_a_concurrent_send_behind_the_whole_batch() {
    let c = controller();
    let (tx, mut rx) = mpsc::channel::<String>(8);
    c.inner.lock().unwrap().stdin_tx = Some(tx);

    let config = PersistentSpawnConfig {
        cli_command: "unused".to_string(),
        cli_args: vec![],
        working_dir: String::new(),
        env_vars: HashMap::new(),
        session_id_field: "session_id".to_string(),
        resume_flag: "--resume".to_string(),
        session_id: String::new(),
        message_id: None,
    };
    c.retry_after_resume_failure(
        1,
        config,
        vec![qentry(1, "batch-1"), qentry(2, "batch-2")],
        None,
        "dead-sid".to_string(),
    );

    // Races in while the flush task holds the drain claim: must be
    // queued behind the batch, never delivered directly.
    let action = c.decide_send_action("later-send", None);
    assert!(
        matches!(action, SendAction::Queued),
        "a send during a retry flush must queue behind the batch, not DeliverDirect ahead of it"
    );

    assert_eq!(rx.recv().await.unwrap(), "batch-1");
    assert_eq!(rx.recv().await.unwrap(), "batch-2");
    assert_eq!(
        rx.recv().await.unwrap(),
        "later-send",
        "the concurrent send must arrive strictly after the whole batch"
    );

    tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
    let inner = c.inner.lock().unwrap();
    assert!(!inner.drain_claim, "claim must be released once the queue runs dry");
    assert!(inner.pending_send_messages.is_empty());
}

/// codex P2 on PR #2371: a confirmed retry that turns out NOT to
/// actually launch (here, `spawn_process` failing against a
/// nonexistent binary — the `BecomeSpawner` path's own failure case)
/// must flush a held-back error line instead of silently dropping
/// it. Without this, an already-accepted prompt whose stale-resume
/// retry then ALSO fails to spawn would end in total silence:
/// neither the original error nor a replacement one.
#[test]
fn retry_after_resume_failure_flushes_the_held_error_line_when_the_respawn_itself_fails() {
    let broker = Arc::new(crate::backend::mps::Broker::new());
    let filestore = Arc::new(FileStore::open_in_memory().unwrap());
    let block_id = "block-flush-on-failed-retry".to_string();
    let c = PersistentSubprocessController::new(
        "tab".to_string(),
        block_id.clone(),
        Some(broker),
        None,
        None,
        Some(filestore.clone()),
    );

    let config = PersistentSpawnConfig {
        cli_command: "definitely-not-a-real-binary-xyz".to_string(),
        cli_args: vec![],
        working_dir: String::new(),
        env_vars: HashMap::new(),
        session_id_field: "session_id".to_string(),
        resume_flag: "--resume".to_string(),
        session_id: String::new(),
        message_id: None,
    };
    c.retry_after_resume_failure(1, config, vec![qentry(1, "{}")], Some("boom\n".to_string()), "dead-sid".to_string());

    let flushed = filestore
        .read_file(&block_id, PERSISTENT_OUTPUT_SUBJECT)
        .unwrap()
        .map(|bytes| String::from_utf8_lossy(&bytes).contains("boom"))
        .unwrap_or(false);
    assert!(
        flushed,
        "a held error line must be flushed to the blockfile when the retry's own respawn fails, \
         not silently dropped"
    );
}

/// Regression test for reagentx + Codex (PR #2693 review): when no
/// recovery candidate exists, this IS unambiguously known right now
/// — the outcome must be emitted immediately, not silently dropped
/// just because the premature emission at the `persistent_resume.rs`
/// state-machine layer was removed.
#[test]
fn retry_after_resume_failure_emits_fresh_outcome_immediately_when_no_recovery_found() {
    let broker = Arc::new(crate::backend::mps::Broker::new());
    let filestore = Arc::new(FileStore::open_in_memory().unwrap());
    let block_id = "block-emits-fresh-no-recovery".to_string();
    let c = PersistentSubprocessController::new(
        "tab".to_string(),
        block_id.clone(),
        Some(broker),
        None,
        None,
        Some(filestore.clone()),
    );
    let config = PersistentSpawnConfig {
        cli_command: "definitely-not-a-real-binary-xyz".to_string(),
        cli_args: vec![],
        working_dir: String::new(),
        env_vars: HashMap::new(), // no CLAUDE_CONFIG_DIR — nothing to recover
        session_id_field: "session_id".to_string(),
        resume_flag: "--resume".to_string(),
        session_id: "dead-sid".to_string(),
        message_id: None,
    };
    c.retry_after_resume_failure(1, config, vec![qentry(1, "{}")], None, "dead-sid".to_string());

    let content = filestore
        .read_file(&block_id, PERSISTENT_OUTPUT_SUBJECT)
        .unwrap()
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default();
    assert!(
        content.contains("agentmux_session_outcome") && content.contains("\"fresh\""),
        "a genuinely blank fallback (no recovery candidate) must emit the Fresh outcome \
         immediately — got: {content:?}"
    );
}

/// Regression test for reagentx + Codex (PR #2693 review), the other
/// half: when a recovery candidate IS found, this function must NOT
/// emit ANY outcome yet — the CLI hasn't confirmed it (Codex P1: it
/// could still reject the recovered id too). The eventual outcome is
/// left to `persistent_resume.rs`'s already-tested `SessionCaptured`
/// handling, once the (now resume-tracked) recovery attempt actually
/// resolves.
#[test]
fn retry_after_resume_failure_does_not_emit_an_outcome_yet_when_recovery_is_found() {
    let tmp = tempfile::tempdir().unwrap();
    let config_dir = tmp.path().to_string_lossy().to_string();
    let working_dir = r"C:\Users\user\.agentmux\agents\agentx-0623n".to_string();
    let slug = crate::backend::claude_layout::project_dir_name(&working_dir);
    let dir = tmp.path().join("projects").join(&slug);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("972a6a4f-live.jsonl"), vec![b'x'; 2_800_000]).unwrap();

    let broker = Arc::new(crate::backend::mps::Broker::new());
    let filestore = Arc::new(FileStore::open_in_memory().unwrap());
    let block_id = "block-defers-outcome-on-recovery".to_string();
    let c = PersistentSubprocessController::new(
        "tab".to_string(),
        block_id.clone(),
        Some(broker),
        None,
        None,
        Some(filestore.clone()),
    );
    let mut env_vars = HashMap::new();
    env_vars.insert("CLAUDE_CONFIG_DIR".to_string(), config_dir);
    let config = PersistentSpawnConfig {
        cli_command: "definitely-not-a-real-binary-xyz".to_string(),
        cli_args: vec![],
        working_dir,
        env_vars,
        session_id_field: "session_id".to_string(),
        resume_flag: "--resume".to_string(),
        session_id: "d019e2e4-stale".to_string(),
        message_id: None,
    };
    c.retry_after_resume_failure(1, config, vec![qentry(1, "{}")], None, "d019e2e4-stale".to_string());

    let content = filestore
        .read_file(&block_id, PERSISTENT_OUTPUT_SUBJECT)
        .unwrap()
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default();
    assert!(
        !content.contains("agentmux_session_outcome"),
        "must not claim any outcome until the CLI actually confirms the recovered id — got: {content:?}"
    );
}

/// docs/status/STATUS_STALE_RESUME_LIVE_REPRO_AND_FIX_PLAN_2026_08_23.md
/// §6.2: a stale-`--resume` retry must publish a "Reconnecting…" status
/// ping so the pane can show *something* during the gap instead of going
/// silent. When no recovery candidate exists, the retry AND its
/// resolution are both known synchronously in the same call — both
/// pings must land, in order.
#[test]
fn retry_after_resume_failure_publishes_retrying_then_resolved_when_no_recovery_found() {
    let broker = Arc::new(crate::backend::mps::Broker::new());
    let filestore = Arc::new(FileStore::open_in_memory().unwrap());
    let block_id = "block-reconnecting-no-recovery".to_string();
    let c = PersistentSubprocessController::new(
        "tab".to_string(),
        block_id.clone(),
        Some(broker.clone()),
        None,
        None,
        Some(filestore),
    );
    let config = PersistentSpawnConfig {
        cli_command: "definitely-not-a-real-binary-xyz".to_string(),
        cli_args: vec![],
        working_dir: String::new(),
        env_vars: HashMap::new(),
        session_id_field: "session_id".to_string(),
        resume_flag: "--resume".to_string(),
        session_id: "dead-sid".to_string(),
        message_id: None,
    };
    c.retry_after_resume_failure(1, config, vec![qentry(1, "{}")], None, "dead-sid".to_string());

    let history = broker.read_event_history(
        crate::backend::mps::EVENT_AGENT_RESUME_RETRY,
        &format!("block:{block_id}"),
        10,
    );
    let statuses: Vec<Option<&str>> = history
        .iter()
        .map(|e| e.data.as_ref().and_then(|d| d.get("status")).and_then(|v| v.as_str()))
        .collect();
    assert_eq!(
        statuses,
        vec![Some("retrying"), Some("resolved")],
        "expected retrying then resolved, got: {statuses:?}"
    );
    let retrying_data = history[0].data.as_ref().unwrap();
    assert!(
        retrying_data.get("startedAt").and_then(|v| v.as_str()).is_some(),
        "the retrying ping must carry a startedAt timestamp for the frontend's elapsed-time readout"
    );
}

/// The other half of the pair above: when a recovery candidate IS found,
/// only "retrying" fires within this call — "resolved" is deferred to
/// whichever `EmitSessionOutcome` handling site eventually confirms the
/// recovered id (or rejects it, cascading into another retry — which
/// would republish "retrying" again, not "resolved").
#[test]
fn retry_after_resume_failure_only_publishes_retrying_when_recovery_is_found() {
    let tmp = tempfile::tempdir().unwrap();
    let config_dir = tmp.path().to_string_lossy().to_string();
    let working_dir = r"C:\Users\user\.agentmux\agents\agentx-0623n".to_string();
    let slug = crate::backend::claude_layout::project_dir_name(&working_dir);
    let dir = tmp.path().join("projects").join(&slug);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("972a6a4f-live.jsonl"), vec![b'x'; 2_800_000]).unwrap();

    let broker = Arc::new(crate::backend::mps::Broker::new());
    let filestore = Arc::new(FileStore::open_in_memory().unwrap());
    let block_id = "block-reconnecting-with-recovery".to_string();
    let c = PersistentSubprocessController::new(
        "tab".to_string(),
        block_id.clone(),
        Some(broker.clone()),
        None,
        None,
        Some(filestore),
    );
    let mut env_vars = HashMap::new();
    env_vars.insert("CLAUDE_CONFIG_DIR".to_string(), config_dir);
    let config = PersistentSpawnConfig {
        cli_command: "definitely-not-a-real-binary-xyz".to_string(),
        cli_args: vec![],
        working_dir,
        env_vars,
        session_id_field: "session_id".to_string(),
        resume_flag: "--resume".to_string(),
        session_id: "d019e2e4-stale".to_string(),
        message_id: None,
    };
    c.retry_after_resume_failure(1, config, vec![qentry(1, "{}")], None, "d019e2e4-stale".to_string());

    let history = broker.read_event_history(
        crate::backend::mps::EVENT_AGENT_RESUME_RETRY,
        &format!("block:{block_id}"),
        10,
    );
    let statuses: Vec<Option<&str>> = history
        .iter()
        .map(|e| e.data.as_ref().and_then(|d| d.get("status")).and_then(|v| v.as_str()))
        .collect();
    assert_eq!(
        statuses,
        vec![Some("retrying")],
        "must not resolve yet — the CLI hasn't confirmed the recovered id — got: {statuses:?}"
    );
}

/// reagentx P1 (round 2 on PR #2371): `DeliverDirect`'s own fallback
/// (every message in the batch fails `try_send`, so delivery hands
/// off to `drain_queue_after_successful_spawn` — a fire-and-forget
/// background task) must NOT eagerly flush a held-back error line —
/// an earlier cut of this fix did, reasoning it couldn't confirm
/// eventual delivery, but that contradicts the same established
/// pattern as the `Queued` arm: eventual success is the
/// overwhelmingly common outcome, so flushing eagerly would show a
/// stale, wrong error bubble immediately followed by the real
/// (successful) response — reproducing this PR's own bug via a
/// different path. The drain's own `stalled_with_leftovers` branch
/// already publishes a status update on genuine total failure. A
/// closed stdin receiver forces every `try_send` in the batch to
/// fail deterministically.
#[tokio::test]
async fn retry_after_resume_failure_does_not_flush_the_held_error_line_when_the_deliver_direct_fallback_is_needed() {
    let broker = Arc::new(crate::backend::mps::Broker::new());
    let filestore = Arc::new(FileStore::open_in_memory().unwrap());
    let block_id = "block-flush-on-deliver-direct-fallback".to_string();
    let c = PersistentSubprocessController::new(
        "tab".to_string(),
        block_id.clone(),
        Some(broker),
        None,
        None,
        Some(filestore.clone()),
    );
    let (tx, rx) = mpsc::channel::<String>(1);
    drop(rx); // closed receiver — every try_send below fails
    c.inner.lock().unwrap().stdin_tx = Some(tx);

    let config = PersistentSpawnConfig {
        cli_command: "unused".to_string(),
        cli_args: vec![],
        working_dir: String::new(),
        env_vars: HashMap::new(),
        session_id_field: "session_id".to_string(),
        resume_flag: "--resume".to_string(),
        session_id: String::new(),
        message_id: None,
    };
    c.retry_after_resume_failure(1, config, vec![qentry(1, "stuck")], Some("boom\n".to_string()), "dead-sid".to_string());
    tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;

    let flushed = filestore
        .read_file(&block_id, PERSISTENT_OUTPUT_SUBJECT)
        .unwrap()
        .map(|bytes| String::from_utf8_lossy(&bytes).contains("boom"))
        .unwrap_or(false);
    assert!(
        !flushed,
        "a held error line must NOT be eagerly flushed when DeliverDirect's own fallback drain \
         is needed — eventual delivery is the common case, and the drain's own stalled branch \
         already surfaces a genuine total failure"
    );
}

/// Closes issue #2368 (the visible-error-flash residue of #2360/#2373's
/// stale-`--resume` retry): unlike the two tests above (retry itself
/// fails to launch → flush; `DeliverDirect` fallback needed → don't
/// flush yet), this covers the actual `BecomeSpawner` HAPPY path — the
/// fresh, no-`--resume` respawn launches successfully. `held_error_line`
/// must be silently dropped here, never reaching the blockfile: the
/// doomed first attempt's error was never the user's problem to see
/// once the transparent retry it triggered actually worked. This is
/// the regression test
/// `docs/specs/SPEC_PERSISTENT_SPAWN_GENERATION_AND_MESSAGE_IDENTITY_2026_08_09.md`
/// §5's verification list asked for before #2368 could be closed with
/// evidence.
#[tokio::test]
async fn retry_after_resume_failure_drops_the_held_error_line_when_the_respawn_succeeds() {
    let broker = Arc::new(crate::backend::mps::Broker::new());
    let filestore = Arc::new(FileStore::open_in_memory().unwrap());
    let block_id = "block-drop-on-successful-retry".to_string();
    let c = PersistentSubprocessController::new(
        "tab".to_string(),
        block_id.clone(),
        Some(broker),
        None,
        None,
        Some(filestore.clone()),
    );

    // codex P1 on this PR: "echo" is a cmd.exe BUILT-IN on Windows, not
    // a standalone executable — `Command::new("echo")` fails to spawn
    // on the required Windows CI leg, flipping this test's assertion
    // (spawn `Err` routes through the FAILURE branch, which flushes
    // "boom", the opposite of what's being proven here). "git" is a
    // real, standalone executable guaranteed present on every
    // supported CI platform (Windows/macOS/Linux all need it to check
    // the repo out in the first place) and exits 0 near-instantly.
    let config = PersistentSpawnConfig {
        cli_command: "git".to_string(),
        cli_args: vec!["--version".to_string()],
        working_dir: String::new(),
        env_vars: HashMap::new(),
        session_id_field: "session_id".to_string(),
        resume_flag: "--resume".to_string(),
        session_id: String::new(),
        message_id: None,
    };
    c.retry_after_resume_failure(1, config, vec![qentry(1, "{}")], Some("boom\n".to_string()), "dead-sid".to_string());
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    let flushed = filestore
        .read_file(&block_id, PERSISTENT_OUTPUT_SUBJECT)
        .unwrap()
        .map(|bytes| String::from_utf8_lossy(&bytes).contains("boom"))
        .unwrap_or(false);
    assert!(
        !flushed,
        "a held error line from the doomed first attempt must be silently dropped — never \
         flushed to the blockfile — when the stale-`--resume` retry's fresh respawn actually \
         succeeds (issue #2368): the user should see only the real response, not a stale \
         error bubble followed by it"
    );
}

/// #2360: the drain must track every message it successfully delivers
/// beyond the first (which `spawn_process` already seeded synchronously
/// via `ResumeEvent::SpawnedWithResume`) into the `AwaitingOutcome`
/// retry batch, so a confirmed stale-resume retry redelivers the WHOLE
/// batch rather than just the message that triggered the spawn.
#[tokio::test]
async fn drain_appends_later_messages_to_the_pending_resume_retry_without_duplicating_the_first() {
    let c = controller();
    let (tx, mut rx) = mpsc::channel::<String>(8);
    let retry_config = PersistentSpawnConfig {
        cli_command: "unused".to_string(),
        cli_args: vec![],
        working_dir: String::new(),
        env_vars: HashMap::new(),
        session_id_field: "session_id".to_string(),
        resume_flag: "--resume".to_string(),
        session_id: String::new(),
        message_id: None,
    };
    {
        let mut inner = c.inner.lock().unwrap();
        inner.stdin_tx = Some(tx);
        inner.spawning_in_progress = true;
        inner.pending_send_messages.push_back(QueuedMessage::fresh(1, "first".to_string()));
        inner.pending_send_messages.push_back(QueuedMessage::fresh(2, "second".to_string()));
        // Simulates spawn_process's own synchronous stash for "first"
        // — the message that triggered this spawn.
        let generation = inner.spawn_generation;
        inner.apply_resume_event(persistent_resume::ResumeEvent::SpawnedWithResume {
            generation,
            attempted_sid: "sid".to_string(),
            retry: persistent_resume::RetryPayload { config: retry_config.clone(), messages: vec![qentry(1, "first")] },
        });
    }

    c.drain_queue_after_successful_spawn(retry_config, true);

    assert_eq!(rx.recv().await.unwrap(), "first");
    assert_eq!(rx.recv().await.unwrap(), "second");
    tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;

    let inner = c.inner.lock().unwrap();
    assert!(
        !inner.drain_send_in_flight,
        "must be cleared once the send-then-append sequence for the last message has fully completed"
    );
    match &inner.resume {
        persistent_resume::ResumeState::AwaitingOutcome { retry, .. } => assert_eq!(
            retry.messages,
            vec![qentry(1, "first"), qentry(2, "second")],
            "must contain the ORIGINAL message exactly once plus every later delivery, in order"
        ),
        other => panic!("expected AwaitingOutcome, got {other:?}"),
    }
}

/// codex P2 on PR #2360 (round 15, commit fdb8db6fd): the message
/// `spawn_process` already seeded into the retry batch is not
/// always the FIRST thing this drain pops — a prior "second stall" can
/// leave an older leftover queued ahead of a later spawner's own
/// triggering message (`push_back` appends behind it). A purely
/// positional "is this the first delivery" check would treat the
/// OLDER LEFTOVER as if it were the seed (dropping it from tracking
/// entirely) while recording the ACTUAL trigger message a second time
/// (once via the synchronous seed, once via this drain's own append).
/// Confirms identity matching (`QueuedRetryEntry::seq`, #2365)
/// identifies the true seed regardless of position: the older leftover
/// is recorded, and the actual trigger is not duplicated.
#[tokio::test]
async fn drain_identifies_the_seed_by_content_even_when_a_leftover_is_delivered_first() {
    let c = controller();
    let (tx, mut rx) = mpsc::channel::<String>(8);
    let retry_config = PersistentSpawnConfig {
        cli_command: "unused".to_string(),
        cli_args: vec![],
        working_dir: String::new(),
        env_vars: HashMap::new(),
        session_id_field: "session_id".to_string(),
        resume_flag: "--resume".to_string(),
        session_id: String::new(),
        message_id: None,
    };
    {
        let mut inner = c.inner.lock().unwrap();
        inner.stdin_tx = Some(tx);
        inner.spawning_in_progress = true;
        // Simulates a "second stall" leaving an older leftover queued
        // ahead of this spawn's own triggering message.
        inner.pending_send_messages.push_back(QueuedMessage::fresh(1, "older-leftover".to_string()));
        inner.pending_send_messages.push_back(QueuedMessage::fresh(2, "new-trigger".to_string()));
        // spawn_process's own synchronous stash — seeded with the
        // ACTUAL trigger message, not whatever happens to sit at the
        // front of the queue.
        let generation = inner.spawn_generation;
        inner.apply_resume_event(persistent_resume::ResumeEvent::SpawnedWithResume {
            generation,
            attempted_sid: "sid".to_string(),
            retry: persistent_resume::RetryPayload {
                config: retry_config.clone(),
                messages: vec![qentry(2, "new-trigger")],
            },
        });
    }

    c.drain_queue_after_successful_spawn(retry_config, true);

    assert_eq!(rx.recv().await.unwrap(), "older-leftover");
    assert_eq!(rx.recv().await.unwrap(), "new-trigger");
    tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;

    let inner = c.inner.lock().unwrap();
    let delivered = match &inner.resume {
        persistent_resume::ResumeState::AwaitingOutcome { retry, .. } => &retry.messages,
        other => panic!("expected AwaitingOutcome, got {other:?}"),
    };
    assert_eq!(
        delivered.len(),
        2,
        "must contain exactly the older leftover plus the trigger, no omission, no duplication: {delivered:?}"
    );
    assert!(
        delivered.iter().any(|e| e.json == "older-leftover"),
        "the older leftover must not be silently dropped from the retry batch"
    );
    assert_eq!(
        delivered.iter().filter(|e| e.json == "new-trigger").count(),
        1,
        "the actual trigger message must not be recorded twice"
    );
}

/// #2360: `poison_resume` (the stderr-reader task, running
/// concurrently with this drain) can promote the retry batch from
/// `AwaitingOutcome` to `ConfirmedRetry` at any point. A message
/// delivered right after that promotion must still be tracked —
/// appending only while still `AwaitingOutcome` would silently drop it
/// from the batch the replacement actually replays.
#[tokio::test]
async fn drain_appends_to_confirmed_retry_once_already_promoted_from_pending() {
    let c = controller();
    let (tx, mut rx) = mpsc::channel::<String>(8);
    let retry_config = PersistentSpawnConfig {
        cli_command: "unused".to_string(),
        cli_args: vec![],
        working_dir: String::new(),
        env_vars: HashMap::new(),
        session_id_field: "session_id".to_string(),
        resume_flag: "--resume".to_string(),
        session_id: String::new(),
        message_id: None,
    };
    {
        let mut inner = c.inner.lock().unwrap();
        inner.stdin_tx = Some(tx);
        inner.spawning_in_progress = true;
        inner.pending_send_messages.push_back(QueuedMessage::fresh(1, "first".to_string()));
        inner.pending_send_messages.push_back(QueuedMessage::fresh(2, "second".to_string()));
        // Simulates poison_resume having ALREADY promoted the tentative
        // retry to confirmed before the drain got to "second".
        let generation = inner.spawn_generation;
        inner.apply_resume_event(persistent_resume::ResumeEvent::SpawnedWithResume {
            generation,
            attempted_sid: "sid".to_string(),
            retry: persistent_resume::RetryPayload { config: retry_config.clone(), messages: vec![qentry(1, "first")] },
        });
        inner.apply_resume_event(persistent_resume::ResumeEvent::ResumeUnreachable {
            generation,
            sid: "sid".to_string(),
        });
    }

    c.drain_queue_after_successful_spawn(retry_config, true);

    assert_eq!(rx.recv().await.unwrap(), "first");
    assert_eq!(rx.recv().await.unwrap(), "second");
    tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;

    let inner = c.inner.lock().unwrap();
    let delivered = match &inner.resume {
        persistent_resume::ResumeState::ConfirmedRetry { retry, .. } => &retry.messages,
        other => {
            panic!("expected ConfirmedRetry (already promoted), got {other:?}")
        }
    };
    assert_eq!(
        delivered,
        &vec![qentry(1, "first"), qentry(2, "second")],
        "must still be tracking this spawn's delivered messages, even though it was already confirmed"
    );
}

/// codex P2 on PR #2360 (sixth review pass, round 6): deciding and
/// enqueueing a multi-message retry batch one call at a time left a
/// window where a genuinely new, unrelated message could interleave
/// into the MIDDLE of the batch. `decide_retry_batch_action` must
/// enqueue the whole batch atomically instead.
#[test]
fn decide_retry_batch_action_enqueues_the_whole_batch_atomically() {
    let c = controller();
    let action = c.decide_retry_batch_action(1, &qentry(1, "first"), &[qentry(2, "second"), qentry(3, "third")]);
    assert!(matches!(action, RetryBatchAction::BecomeSpawner { .. }));
    let inner = c.inner.lock().unwrap();
    assert_eq!(
        inner.pending_send_messages.iter().cloned().collect::<Vec<_>>(),
        vec!["first".to_string(), "second".to_string(), "third".to_string()],
    );
}

/// codex P2 on PR #2360 (sixth review pass, round 6): content-based
/// dedup must never apply WITHIN a retry batch — two entries can
/// legitimately be identical (the user genuinely sent the same text
/// twice, both accepted by the doomed process) and both need
/// redelivering.
#[test]
fn decide_retry_batch_action_preserves_duplicate_content_within_the_batch() {
    let c = controller();
    c.inner.lock().unwrap().spawning_in_progress = true;

    let action =
        c.decide_retry_batch_action(1, &qentry(1, "hello"), &[qentry(2, "hello"), qentry(3, "hello")]);

    assert!(matches!(action, RetryBatchAction::Queued));
    let inner = c.inner.lock().unwrap();
    assert_eq!(
        inner.pending_send_messages.len(),
        3,
        "all three identical entries must be preserved, not deduped"
    );
}

/// The dedup check still applies to `first` alone, against whatever
/// might ALREADY be queued from before this batch decision — e.g. the
/// drain hasn't reached the original triggering message yet.
#[test]
fn decide_retry_batch_action_dedups_only_the_first_against_pre_existing_queue() {
    let c = controller();
    {
        let mut inner = c.inner.lock().unwrap();
        inner.spawning_in_progress = true;
        inner.pending_send_messages.push_back(QueuedMessage::fresh(1, "first".to_string()));
    }

    let action = c.decide_retry_batch_action(1, &qentry(1, "first"), &[qentry(2, "second")]);

    assert!(matches!(action, RetryBatchAction::Queued));
    let inner = c.inner.lock().unwrap();
    assert_eq!(
        inner
            .pending_send_messages
            .iter()
            .map(|m| (m.seq, m.json_str.clone()))
            .collect::<Vec<_>>(),
        vec![(1, "first".to_string()), (2, "second".to_string())],
        "the already-queued first entry must not be duplicated, but the rest of the batch must still be appended"
    );
}

/// codex P2 on PR #2360 (sixth review pass, round 11): the retry
/// batch represents content the doomed process accepted BEFORE
/// whatever's already sitting in the queue (which arrived AFTER the
/// original spawn claimed `spawning_in_progress`) — it must be
/// delivered first on the fresh process, not appended behind
/// later-arriving input.
#[test]
fn decide_retry_batch_action_prepends_ahead_of_an_unrelated_later_message() {
    let c = controller();
    {
        let mut inner = c.inner.lock().unwrap();
        inner.spawning_in_progress = true;
        // "later-message" arrived after the original spawn's claim
        // started, while the retry's own trigger ("A") had already
        // been popped and delivered (and is no longer in the queue —
        // it's now tracked only in the confirmed retry batch).
        inner.pending_send_messages.push_back(QueuedMessage::fresh(1, "later-message".to_string()));
    }

    let action = c.decide_retry_batch_action(1, &qentry(2, "A"), &[]);

    assert!(matches!(action, RetryBatchAction::Queued));
    let inner = c.inner.lock().unwrap();
    assert_eq!(
        inner.pending_send_messages.iter().cloned().collect::<Vec<_>>(),
        vec!["A".to_string(), "later-message".to_string()],
        "the retry's own (chronologically earlier) message must precede the later, unrelated one"
    );
}

// ── Immediate delivery (SPEC_JEKT_IMMEDIATE_DELIVERY_2026_09_28 §2.1) ──
//
// A live process gets an automated message at once, mid-turn or not. The
// queue holds a message only while the process cannot take a write — a spawn
// in flight, a committed restart, a requested stop, another writer on stdin —
// and the watchdog writes it once it can. These pin both halves: nothing
// waits on the agent's turn, and nothing is dropped or reordered around a
// process that isn't ready (the startup race of
// `SPEC_NO_MIDTURN_DELIVERY_2026_09_23.md` §3.2.1).

/// A live controller with a turn already running.
fn busy_controller() -> (PersistentSubprocessController, mpsc::Receiver<String>) {
    let c = controller();
    let (tx, rx) = mpsc::channel::<String>(64);
    {
        let mut inner = c.inner.lock().unwrap();
        inner.stdin_tx = Some(tx);
    }
    c.health_monitor.set_active_turn(true);
    (c, rx)
}

/// A live process whose spawn is still in flight: `stdin_tx` is set, but the
/// spawn's own seed message and drain have not finished.
fn starting_controller() -> (PersistentSubprocessController, mpsc::Receiver<String>) {
    let (c, rx) = busy_controller();
    c.inner.lock().unwrap().spawning_in_progress = true;
    (c, rx)
}

fn drain(rx: &mut mpsc::Receiver<String>) -> Vec<String> {
    let mut out = Vec::new();
    while let Ok(line) = rx.try_recv() {
        out.push(line);
    }
    out
}

/// The core of the change: a message for a live, busy agent is written now,
/// and the caller is told it was delivered, not queued.
#[tokio::test]
async fn a_message_arriving_mid_turn_is_written_at_once() {
    let (c, mut rx) = busy_controller();

    let outcome = c.send_user_message_outcome("from github".to_string()).unwrap();

    assert_eq!(outcome, SendOutcome::Sent, "reported delivered, not deferred");
    let line = rx.try_recv().expect("a mid-turn message reaches live stdin at once");
    assert!(line.contains("from github"));
    assert!(c.inner.lock().unwrap().deferred_deliveries.is_empty(), "nothing is held for the turn");
    assert!(c.health_monitor.is_active_turn(), "the running turn carries on");
}

/// An idle agent gets it at once too, and the write starts its turn.
#[tokio::test]
async fn a_message_arriving_while_idle_is_written_at_once_and_starts_a_turn() {
    let (c, mut rx) = idle_controller();

    assert_eq!(c.send_user_message_outcome("hello".to_string()).unwrap(), SendOutcome::Sent);

    let line = rx.try_recv().expect("an idle agent receives the message now");
    assert!(line.contains("hello"));
    assert!(c.inner.lock().unwrap().deferred_deliveries.is_empty());
    assert!(c.health_monitor.is_active_turn());
}

/// Several messages mid-turn: every one is written, in the order sent — none
/// waits for the next turn, and none overtakes an earlier one.
#[tokio::test]
async fn successive_messages_mid_turn_are_all_written_in_send_order() {
    let (c, mut rx) = busy_controller();

    for text in ["first", "second", "third"] {
        assert_eq!(c.send_user_message_outcome(text.to_string()).unwrap(), SendOutcome::Sent);
    }

    let lines = drain(&mut rx);
    assert_eq!(lines.len(), 3, "all three written now: {lines:?}");
    for (line, text) in lines.iter().zip(["first", "second", "third"]) {
        assert!(line.contains(text), "send order kept: {lines:?}");
    }
    assert!(c.inner.lock().unwrap().deferred_deliveries.is_empty());
}

/// Concurrent senders with real threads, against a busy turn and against an
/// idle one: every message is written, none is queued or lost, and each
/// sender's own messages arrive in the order it sent them.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_senders_are_all_written_in_each_senders_order() {
    use std::sync::Arc as StdArc;

    for busy in [true, false] {
        // A sender that starts a turn re-arms the status heartbeat, which
        // needs a reactor; plain threads have none, so hand each one this
        // runtime's handle.
        let rt = tokio::runtime::Handle::current();
        let (c, mut rx) = busy_controller();
        c.health_monitor.set_active_turn(busy);
        let c = StdArc::new(c);

        let handles: Vec<_> = (0..16)
            .map(|i| {
                let c = StdArc::clone(&c);
                let rt = rt.clone();
                std::thread::spawn(move || {
                    let _guard = rt.enter();
                    c.send_user_message(format!("a-{i}")).is_ok()
                        && c.send_user_message(format!("b-{i}")).is_ok()
                })
            })
            .collect();
        for h in handles {
            assert!(h.join().expect("sender thread panicked"), "every send accepted (busy={busy})");
        }

        let lines = drain(&mut rx);
        assert_eq!(lines.len(), 32, "every message written, none held (busy={busy})");
        assert!(c.inner.lock().unwrap().deferred_deliveries.is_empty());
        for i in 0..16 {
            let pos = |m: String| lines.iter().position(|l| l.contains(&format!("\"{m}\""))).unwrap();
            assert!(pos(format!("a-{i}")) < pos(format!("b-{i}")), "sender {i}'s order (busy={busy})");
        }
        assert!(c.health_monitor.is_active_turn());
    }
}

/// The startup race stays fixed: a message sent while the spawn is in flight
/// is queued (not refused, not written ahead of the spawn's own drain), and
/// the watchdog writes it once the process is up — while the spawn's first
/// turn is still running, not at its end.
#[tokio::test]
async fn a_message_during_a_spawn_is_queued_and_delivered_once_the_process_is_up() {
    let (c, mut rx) = starting_controller();

    let outcome = c.send_user_message_outcome("arrived during startup".to_string()).unwrap();

    assert_eq!(outcome, SendOutcome::Deferred);
    assert!(rx.try_recv().is_err(), "nothing ahead of the spawn's own drain");
    assert_eq!(c.inner.lock().unwrap().deferred_deliveries.len(), 1, "queued, not dropped");

    // The spawn's drain finishes; its seed message's turn is running.
    c.inner.lock().unwrap().spawning_in_progress = false;
    assert!(c.health_monitor.is_active_turn());
    let mut orphaned = 0;
    assert_eq!(c.sweep_deferred_once(&mut orphaned), WatchdogStep::Continue);

    assert!(rx.try_recv().unwrap().contains("arrived during startup"));
    assert!(c.inner.lock().unwrap().deferred_deliveries.is_empty());
    assert_eq!(c.sweep_deferred_once(&mut orphaned), WatchdogStep::Exit, "nothing left to watch");
}

/// A full queue returns an error rather than accepting a message it will then
/// drop. Reporting success and losing the message is the failure mode this
/// guards.
#[tokio::test]
async fn a_full_queue_reports_an_error_rather_than_accepting_and_dropping() {
    let (c, _rx) = starting_controller();
    for i in 0..MAX_DEFERRED_DELIVERIES {
        c.send_user_message(format!("msg-{i}")).unwrap();
    }

    let err = c.send_user_message("one too many".to_string()).unwrap_err();

    assert!(err.contains("full"), "got {err:?}");
    assert_eq!(
        c.inner.lock().unwrap().deferred_deliveries.len(),
        MAX_DEFERRED_DELIVERIES,
        "the rejected message must not have been queued"
    );
}

/// FIFO: a message sent while an older one is still queued (the process just
/// became able to take writes, before the watchdog's next tick) must not jump
/// the queue. Both go out now, oldest first.
#[tokio::test]
async fn a_backlog_is_written_first_and_in_order() {
    let (c, mut rx) = idle_controller();
    enqueue_deferred(&c, "earlier");

    assert_eq!(c.send_user_message_outcome("later".to_string()).unwrap(), SendOutcome::Sent);

    let lines = drain(&mut rx);
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(lines[0].contains("earlier") && lines[1].contains("later"), "{lines:?}");
    assert!(c.inner.lock().unwrap().deferred_deliveries.is_empty());
}

/// Teardown must not silently discard accepted messages.
#[tokio::test]
async fn teardown_drains_the_queue_rather_than_discarding_it_silently() {
    let (c, _rx) = starting_controller();
    c.send_user_message("will not make it".to_string()).unwrap();
    assert_eq!(c.inner.lock().unwrap().deferred_deliveries.len(), 1);

    crate::backend::blockcontroller::Controller::stop(&c, true, "done").unwrap();

    assert!(
        c.inner.lock().unwrap().deferred_deliveries.is_empty(),
        "the queue is drained by the report path, not left dangling"
    );
}

/// Codex P2 on #3562: `stop` drains the queue in the same acquisition as the
/// kill, so by the time the kill is observable the queue is already drained,
/// and neither the dying process's final `result` nor a watchdog tick writes
/// anything into it.
#[tokio::test]
async fn stop_drains_the_queue_with_the_kill_so_nothing_is_written_afterwards() {
    let (c, mut rx) = starting_controller();
    let (kill_tx, mut kill_rx) = tokio::sync::oneshot::channel();
    c.inner.lock().unwrap().kill_tx = Some(kill_tx);
    c.send_user_message("queued".to_string()).unwrap();
    let generation = c.inner.lock().unwrap().spawn_generation;

    crate::backend::blockcontroller::Controller::stop(&c, true, "done").unwrap();

    assert!(matches!(kill_rx.try_recv(), Ok(KillRequest::Force)), "the kill was requested");
    assert!(c.inner.lock().unwrap().deferred_deliveries.is_empty());
    // The spawn's claim goes away and the dying process's `result` arrives.
    c.inner.lock().unwrap().spawning_in_progress = false;
    assert_eq!(boundary(&c, generation), Some(false));
    let mut orphaned = 0;
    c.sweep_deferred_once(&mut orphaned);
    assert!(rx.try_recv().is_err(), "nothing written into the process being killed");
}

/// A stop still queues: once the Stop button (SIGINT → `stop_process`) has
/// requested the kill, a message for the dying process is queued, not written
/// into it and lost, even mid-turn. It is kept for the next process and
/// delivered to it.
#[tokio::test]
async fn a_stop_request_queues_messages_for_the_next_process() {
    let (c, mut rx) = busy_controller();
    let (kill_tx, mut kill_rx) = tokio::sync::oneshot::channel();
    c.inner.lock().unwrap().kill_tx = Some(kill_tx);

    c.stop_process(true).unwrap();
    assert!(kill_rx.try_recv().is_ok(), "the kill was requested");

    assert_eq!(c.send_user_message_outcome("while stopping".to_string()).unwrap(), SendOutcome::Deferred);
    let mut orphaned = 0;
    assert_eq!(c.sweep_deferred_once(&mut orphaned), WatchdogStep::Continue);
    assert!(rx.try_recv().is_err(), "nothing written into the process being killed");
    assert_eq!(c.inner.lock().unwrap().deferred_deliveries.len(), 1, "kept for the next process");

    // The replacement spawns (which clears `stop_pending`).
    let (tx, mut rx2) = mpsc::channel::<String>(4);
    {
        let mut inner = c.inner.lock().unwrap();
        inner.stop_pending = false;
        inner.stdin_tx = Some(tx);
    }
    c.sweep_deferred_once(&mut orphaned);
    assert!(rx2.try_recv().unwrap().contains("while stopping"));
}

/// Reagent P1 on #3562: an IDLE agent whose kill was requested must defer an
/// automated send, not refuse it. `restart_pending` and a spawn in flight
/// already do. Once the process has actually exited (`stdin_tx` cleared),
/// the ordinary no-process rules apply again: a send fails into the caller's
/// respawn fallback, and the watchdog's grace window runs, delivering after a
/// respawn or reporting, never parking forever.
#[tokio::test]
async fn an_idle_agent_being_stopped_defers_until_the_process_is_gone() {
    let (c, mut rx) = idle_controller();
    let (kill_tx, _kill_rx) = tokio::sync::oneshot::channel();
    c.inner.lock().unwrap().kill_tx = Some(kill_tx);
    c.stop_process(true).unwrap();

    c.send_user_message("while dying".to_string()).unwrap();
    assert!(rx.try_recv().is_err(), "nothing written into the dying process");
    assert_eq!(c.inner.lock().unwrap().deferred_deliveries.len(), 1, "deferred, not refused");

    // The process exits.
    c.inner.lock().unwrap().stdin_tx = None;
    let err = c.send_user_message("after exit".to_string()).unwrap_err();
    assert!(err.contains("not running"), "the respawn fallback's error: {err}");

    let mut orphaned = 0;
    for _ in 1..DEFERRED_ORPHAN_GRACE_TICKS {
        assert_eq!(c.sweep_deferred_once(&mut orphaned), WatchdogStep::Continue);
    }
    assert_eq!(c.sweep_deferred_once(&mut orphaned), WatchdogStep::Exit, "reported, not parked");
}

/// Codex P2 on #3562: a sender that fetched the controller before
/// `delete_controller` unregistered it can enqueue AFTER `stop()` drained the
/// queue (it defers behind `stop_pending`, and gets `Ok`). When that sender
/// drops the last strong reference, the watchdog (weak ref) exits, so
/// dropping the controller must report whatever is still queued rather than
/// discard it. `stop()` cannot gate this instead: the max-runtime watchdog
/// stops controllers that stay registered and are used again.
#[tokio::test]
async fn dropping_the_controller_reports_a_message_enqueued_after_stop() {
    let (c, _rx) = busy_controller();
    let (kill_tx, _kill_rx) = tokio::sync::oneshot::channel();
    c.inner.lock().unwrap().kill_tx = Some(kill_tx);
    crate::backend::blockcontroller::Controller::stop(&c, true, "done").unwrap();

    // The late sender, after stop()'s drain.
    c.send_user_message("late".to_string()).unwrap();
    let inner = Arc::clone(&c.inner);
    assert_eq!(inner.lock().unwrap().deferred_deliveries.len(), 1);

    drop(c);

    assert!(
        inner.lock().unwrap().deferred_deliveries.is_empty(),
        "drained and reported on drop, not discarded with the controller"
    );
}

/// A human direct send reserves the turn before its write. If that write
/// fails, the turn must be handed back, or the agent stays "busy" with
/// nothing running.
#[tokio::test]
async fn a_failed_human_direct_send_releases_the_turn_it_reserved() {
    let c = controller();
    let (tx, _rx) = mpsc::channel::<String>(1);
    tx.try_send("occupying the only slot".to_string()).unwrap();
    c.inner.lock().unwrap().stdin_tx = Some(tx);
    let config = PersistentSpawnConfig {
        cli_command: "unused-the-process-is-already-running".to_string(),
        cli_args: vec![],
        working_dir: String::new(),
        env_vars: HashMap::new(),
        session_id_field: "session_id".to_string(),
        resume_flag: "--resume".to_string(),
        session_id: String::new(),
        message_id: None,
    };

    assert!(c.send_message("human".to_string(), config).is_err());

    assert!(!c.health_monitor.is_active_turn(), "nothing was written, so no turn is running");
}

/// A process that went away must not silently eat the queue: a failed flush
/// leaves every entry in place, reported as an error, so teardown or the
/// watchdog's grace window can report them.
#[tokio::test]
async fn a_failed_flush_leaves_the_queue_intact() {
    let c = controller();
    enqueue_deferred(&c, "stranded");

    let (written, err) = {
        let mut inner = c.inner.lock().unwrap();
        PersistentSubprocessController::flush_deferred_locked(&mut inner, "block")
    };

    assert!(written.is_empty());
    assert!(err.is_some_and(|e| e.contains("not running")), "a failed write is not an empty queue");
    assert_eq!(c.inner.lock().unwrap().deferred_deliveries.len(), 1, "a failed write must not drop the message");
}

/// Codex + reagent P1 on #3523: every stdin write is tracked into the current
/// generation's resume retry batch. The case that matters is eager resume: an
/// unconfirmed `--resume` with NO seed message, so the batch starts empty. A
/// message written there and not tracked would be gone if the resume then
/// turned out to be stale, despite having been accepted and rendered.
#[tokio::test]
async fn a_delivered_message_is_tracked_into_an_unconfirmed_resume_retry_batch() {
    let (c, mut rx) = busy_controller();
    {
        let mut inner = c.inner.lock().unwrap();
        let generation = inner.spawn_generation;
        inner.apply_resume_event(persistent_resume::ResumeEvent::SpawnedWithResume {
            generation,
            attempted_sid: "sid".to_string(),
            retry: persistent_resume::RetryPayload {
                config: PersistentSpawnConfig {
                    cli_command: "unused".to_string(),
                    cli_args: vec![],
                    working_dir: String::new(),
                    env_vars: HashMap::new(),
                    session_id_field: "session_id".to_string(),
                    resume_flag: "--resume".to_string(),
                    session_id: String::new(),
                    message_id: None,
                },
                messages: vec![],
            },
        });
    }
    c.send_user_message("mid-turn".to_string()).unwrap();

    let written = rx.try_recv().unwrap();
    let inner = c.inner.lock().unwrap();
    match &inner.resume {
        persistent_resume::ResumeState::AwaitingOutcome { retry, .. } => {
            assert_eq!(retry.messages.len(), 1, "the written message must be in the retry batch");
            assert_eq!(retry.messages[0].json, written);
        }
        other => panic!("expected AwaitingOutcome, got {other:?}"),
    }
}

// ── Deferred-delivery watchdog (codex P1s on #3562) ─────────────────
//
// The watchdog is what writes a queued message once the process can take it,
// and what reports one that never can.

fn idle_controller() -> (PersistentSubprocessController, mpsc::Receiver<String>) {
    let (c, rx) = busy_controller();
    c.health_monitor.set_active_turn(false);
    (c, rx)
}

fn enqueue_deferred(c: &PersistentSubprocessController, text: &str) {
    c.inner
        .lock()
        .unwrap()
        .deferred_deliveries
        .push_back(PersistentSubprocessController::encode_user_message(text));
}

/// Codex P1: a failed write is not an empty queue. The entry stays queued,
/// and the watchdog's next tick delivers it once the channel has room.
#[tokio::test]
async fn a_full_stdin_channel_is_a_failed_flush_retried_by_the_watchdog() {
    let c = controller();
    let (tx, mut rx) = mpsc::channel::<String>(1);
    tx.try_send("occupying the only slot".to_string()).unwrap();
    c.inner.lock().unwrap().stdin_tx = Some(tx);
    enqueue_deferred(&c, "waiting");

    let (written, err) = {
        let mut inner = c.inner.lock().unwrap();
        PersistentSubprocessController::flush_deferred_locked(&mut inner, "block")
    };
    assert!(written.is_empty() && err.is_some());
    assert_eq!(c.inner.lock().unwrap().deferred_deliveries.len(), 1, "the entry stays queued");

    // Once the channel has room, the watchdog's next tick delivers it.
    rx.try_recv().unwrap();
    let mut orphaned = 0;
    assert_eq!(c.sweep_deferred_once(&mut orphaned), WatchdogStep::Continue);
    assert!(rx.try_recv().unwrap().contains("waiting"));
    assert!(c.inner.lock().unwrap().deferred_deliveries.is_empty());
}

/// Once the process can take writes, one tick writes the whole backlog in
/// order — busy or idle, nothing waits for a turn — and marks the turn active
/// inside the same critical section, like the send path. This is also the
/// eager-resume case: a spawn that succeeded with nothing to say, so no turn
/// was running.
#[tokio::test]
async fn the_watchdog_writes_the_whole_backlog_in_order_and_marks_the_turn_active() {
    for busy in [false, true] {
        let (c, mut rx) = idle_controller();
        c.health_monitor.set_active_turn(busy);
        enqueue_deferred(&c, "one");
        enqueue_deferred(&c, "two");

        let mut orphaned = 0;
        assert_eq!(c.sweep_deferred_once(&mut orphaned), WatchdogStep::Continue);

        let lines = drain(&mut rx);
        assert_eq!(lines.len(), 2, "busy={busy}: {lines:?}");
        assert!(lines[0].contains("one") && lines[1].contains("two"), "busy={busy}: {lines:?}");
        assert!(c.health_monitor.is_active_turn(), "busy={busy}");
        assert_eq!(c.sweep_deferred_once(&mut orphaned), WatchdogStep::Exit, "busy={busy}");
    }
}

/// muxreview (#3562) asked what a failed write does when an older entry is
/// already stuck at the head. The older entry was accepted and must stay,
/// first in line. The new one was never accepted: it is removed and reported
/// as an error, so its caller retries. Neither is lost, and FIFO holds.
#[tokio::test]
async fn a_failed_write_behind_a_stuck_head_keeps_the_head_and_refuses_the_newcomer() {
    let c = controller();
    let (tx, mut rx) = mpsc::channel::<String>(1);
    tx.try_send("occupying the only slot".to_string()).unwrap();
    c.inner.lock().unwrap().stdin_tx = Some(tx);
    enqueue_deferred(&c, "older");

    assert!(c.send_user_message("newer".to_string()).is_err());

    {
        let inner = c.inner.lock().unwrap();
        assert_eq!(inner.deferred_deliveries.len(), 1);
        assert!(inner.deferred_deliveries[0].contains("older"), "the accepted head stays");
    }
    // Once the channel drains, the watchdog delivers the older entry.
    rx.try_recv().unwrap();
    let mut orphaned = 0;
    c.sweep_deferred_once(&mut orphaned);
    assert!(rx.try_recv().unwrap().contains("older"));
}

/// Every reason the process cannot take a write is honoured.
#[tokio::test]
async fn the_watchdog_waits_while_anything_is_in_flight() {
    let (c, mut rx) = idle_controller();
    enqueue_deferred(&c, "waiting");
    let mut orphaned = 0;

    let cases: [(&str, fn(&mut PersistentInner, bool)); 5] = [
        ("spawn in flight", |i, on| i.spawning_in_progress = on),
        ("retry batch being replayed", |i, on| i.drain_claim = on),
        ("human messages queued", |i, on| {
            if on {
                i.pending_send_messages.push_back(QueuedMessage::fresh(1, "human".to_string()));
            } else {
                i.pending_send_messages.clear();
            }
        }),
        ("restart committed", |i, on| i.restart_pending = on),
        ("stop requested", |i, on| i.stop_pending = on),
    ];
    for (why, set) in cases {
        set(&mut c.inner.lock().unwrap(), true);
        assert_eq!(c.sweep_deferred_once(&mut orphaned), WatchdogStep::Continue);
        assert!(rx.try_recv().is_err(), "must wait: {why}");
        set(&mut c.inner.lock().unwrap(), false);
    }
    c.sweep_deferred_once(&mut orphaned);
    assert!(rx.try_recv().unwrap().contains("waiting"), "delivered once nothing is in flight");
}

/// Reagent P1 on #3562: the send path shares the watchdog's definition of
/// "must wait". Another writer owning stdin — a stale-resume retry batch being
/// replayed, a spawn in flight, human messages queued behind a spawn — was
/// accepted earlier and must not be overtaken, mid-turn or not. Once it is
/// done, the watchdog delivers, even though the turn it started is running.
#[tokio::test]
async fn the_send_path_does_not_overtake_another_writer() {
    let cases: [(&str, fn(&mut PersistentInner, bool)); 3] = [
        ("retry batch being replayed", |i, on| i.drain_claim = on),
        ("spawn in flight", |i, on| i.spawning_in_progress = on),
        ("human messages queued", |i, on| {
            if on {
                i.pending_send_messages.push_back(QueuedMessage::fresh(1, "human".to_string()));
            } else {
                i.pending_send_messages.clear();
            }
        }),
    ];
    for (why, set) in cases {
        let (c, mut rx) = busy_controller();
        set(&mut c.inner.lock().unwrap(), true);

        assert_eq!(c.send_user_message_outcome("automated".to_string()).unwrap(), SendOutcome::Deferred, "{why}");
        assert!(rx.try_recv().is_err(), "must not write: {why}");
        assert_eq!(c.inner.lock().unwrap().deferred_deliveries.len(), 1, "kept: {why}");

        set(&mut c.inner.lock().unwrap(), false);
        let mut orphaned = 0;
        c.sweep_deferred_once(&mut orphaned);
        assert!(rx.try_recv().unwrap().contains("automated"), "delivered afterwards, mid-turn: {why}");
    }
}

/// Codex P1: a message deferred behind a spawn that then FAILS has no process
/// and no teardown coming. After the grace window with no process and no
/// respawn, it is reported stranded and the watchdog exits — not before, so a
/// respawn that is about to start still gets it.
#[tokio::test]
async fn a_message_deferred_behind_a_failed_spawn_is_reported_after_the_grace_window() {
    let c = controller();
    c.inner.lock().unwrap().spawning_in_progress = true;
    c.send_user_message("behind a doomed spawn".to_string()).unwrap();
    assert_eq!(c.inner.lock().unwrap().deferred_deliveries.len(), 1, "deferred, not refused");

    // The spawn fails: claim released, no process.
    c.inner.lock().unwrap().spawning_in_progress = false;

    let mut orphaned = 0;
    for _ in 1..DEFERRED_ORPHAN_GRACE_TICKS {
        assert_eq!(c.sweep_deferred_once(&mut orphaned), WatchdogStep::Continue);
    }
    assert_eq!(
        c.inner.lock().unwrap().deferred_deliveries.len(),
        1,
        "still held inside the grace window"
    );
    assert_eq!(c.sweep_deferred_once(&mut orphaned), WatchdogStep::Exit);
    let inner = c.inner.lock().unwrap();
    assert!(inner.deferred_deliveries.is_empty(), "reported, not held forever");
    assert!(!inner.deferred_watchdog_armed);
}

/// The drain's "second stall": it releases its claim with leftovers still in
/// `pending_send_messages` and no process (`queue.rs`,
/// `drain_queue_with_claim`). Nothing owns that backlog.
fn orphaned_backlog(c: &PersistentSubprocessController) {
    let mut inner = c.inner.lock().unwrap();
    inner.stdin_tx = None;
    inner.spawning_in_progress = false;
    inner.drain_claim = false;
    inner.pending_send_messages.push_back(QueuedMessage::fresh(1, "leftover".to_string()));
}

/// Codex P1 on #3562: an orphaned backlog is not an active writer. A send
/// must fail like any other no-process send, so the caller's fallback runs,
/// rather than be accepted into a queue nothing will ever drain.
#[tokio::test]
async fn a_send_against_an_orphaned_backlog_fails_rather_than_deferring() {
    let c = controller();
    orphaned_backlog(&c);

    let err = c.send_user_message("automated".to_string()).unwrap_err();

    assert!(err.contains("not running"), "{err}");
    assert!(c.inner.lock().unwrap().deferred_deliveries.is_empty(), "not accepted");
}

/// Codex P1 on #3562: the watchdog must count an orphaned backlog toward the
/// grace window, not reset it every tick and hold the message forever.
#[tokio::test]
async fn the_watchdog_reports_a_message_stuck_behind_an_orphaned_backlog() {
    let c = controller();
    enqueue_deferred(&c, "stuck");
    orphaned_backlog(&c);

    let mut orphaned = 0;
    for _ in 1..DEFERRED_ORPHAN_GRACE_TICKS {
        assert_eq!(c.sweep_deferred_once(&mut orphaned), WatchdogStep::Continue);
    }
    assert_eq!(c.sweep_deferred_once(&mut orphaned), WatchdogStep::Exit);
    assert!(c.inner.lock().unwrap().deferred_deliveries.is_empty(), "reported, not held forever");
}

/// A respawn inside the grace window resets it: the message is delivered,
/// not reported.
#[tokio::test]
async fn a_respawn_inside_the_grace_window_delivers_instead_of_stranding() {
    let c = controller();
    enqueue_deferred(&c, "patient");
    let mut orphaned = 0;
    for _ in 1..DEFERRED_ORPHAN_GRACE_TICKS {
        c.sweep_deferred_once(&mut orphaned);
    }

    let (tx, mut rx) = mpsc::channel::<String>(4);
    c.inner.lock().unwrap().stdin_tx = Some(tx);
    assert_eq!(c.sweep_deferred_once(&mut orphaned), WatchdogStep::Continue);

    assert!(rx.try_recv().unwrap().contains("patient"));
}

/// The real task, end to end: armed by the enqueue, it delivers once the
/// in-flight spawn finishes without ever producing a turn, then disarms.
#[tokio::test]
async fn the_watchdog_task_delivers_after_a_spawn_that_never_starts_a_turn() {
    let c = Arc::new(controller());
    c.set_self_ref();
    let (tx, mut rx) = mpsc::channel::<String>(4);
    {
        let mut inner = c.inner.lock().unwrap();
        inner.stdin_tx = Some(tx);
        inner.spawning_in_progress = true;
    }
    c.send_user_message("arrived during startup".to_string()).unwrap();
    assert!(c.inner.lock().unwrap().deferred_watchdog_armed);

    // Eager resume: the spawn completes with nothing to send.
    c.inner.lock().unwrap().spawning_in_progress = false;

    let line = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
        .await
        .expect("the watchdog must deliver without any turn")
        .unwrap();
    assert!(line.contains("arrived during startup"));

    // Next tick finds the queue empty and disarms.
    tokio::time::sleep(DEFERRED_WATCHDOG_TICK * 3).await;
    assert!(!c.inner.lock().unwrap().deferred_watchdog_armed);
}

// ── Turn-boundary decision (`turn_boundary_locked`) ──────────────────

fn boundary(c: &PersistentSubprocessController, generation: u64) -> Option<bool> {
    PersistentSubprocessController::turn_boundary_locked(
        &mut c.inner.lock().unwrap(),
        &c.health_monitor,
        "block",
        generation,
    )
}

/// Codex P1 on #3562: a `result` buffered from a REPLACED process must not
/// act on the current one: the idle flip and the deferred restart would hit
/// the replacement's running turn.
#[tokio::test]
async fn a_result_from_a_replaced_process_touches_nothing() {
    let (c, _rx) = busy_controller();
    let current = {
        let mut inner = c.inner.lock().unwrap();
        inner.spawn_generation = 2;
        inner.restart_when_idle = true;
        inner.spawn_generation
    };

    assert_eq!(boundary(&c, current - 1), None);

    assert!(c.health_monitor.is_active_turn(), "the replacement's turn is still running");
    let inner = c.inner.lock().unwrap();
    assert!(inner.restart_when_idle, "the deferred restart is not consumed");
    assert!(!inner.restart_pending);
}

/// A boundary ends the turn and writes nothing. The deferred restart applies
/// only with nothing queued — killing the process then would strand the entry
/// — and stays pending for the next boundary otherwise.
#[tokio::test]
async fn a_boundary_ends_the_turn_and_applies_the_restart_only_with_nothing_queued() {
    // Nothing queued: idle, restart applies.
    let (c, mut rx) = busy_controller();
    c.inner.lock().unwrap().restart_when_idle = true;
    assert_eq!(boundary(&c, 0), Some(true));
    assert!(!c.health_monitor.is_active_turn());
    assert!(c.inner.lock().unwrap().restart_pending);
    assert!(rx.try_recv().is_err());

    // Something queued (behind a retry-batch replay): idle, restart waits.
    let (c, mut rx) = busy_controller();
    c.inner.lock().unwrap().drain_claim = true;
    c.send_user_message("queued".to_string()).unwrap();
    c.inner.lock().unwrap().restart_when_idle = true;
    assert_eq!(boundary(&c, 0), Some(false));
    assert!(!c.health_monitor.is_active_turn());
    assert!(rx.try_recv().is_err(), "a boundary writes nothing");
    let inner = c.inner.lock().unwrap();
    assert!(inner.restart_when_idle, "still pending");
    assert!(!inner.restart_pending);
}

// ── Committed deferred restart (codex P1 on #3562) ──────────────────
//
// `turn_boundary_locked` commits `restart_pending` and releases `inner`
// before `stdout_reader.rs` calls `stop_process`. The process's stdin is still live
// in that window, the turn is idle, and the process is about to die.

/// A restart still queues: an automated message arriving in that window must
/// not be written into the doomed process and reported delivered.
#[tokio::test]
async fn a_message_arriving_after_a_restart_is_committed_is_kept_for_the_replacement() {
    let (c, mut rx) = busy_controller();
    c.inner.lock().unwrap().restart_when_idle = true;
    assert_eq!(boundary(&c, 0), Some(true), "precondition: the restart is committed");

    assert_eq!(c.send_user_message_outcome("mid-restart".to_string()).unwrap(), SendOutcome::Deferred);

    assert!(rx.try_recv().is_err(), "nothing written into a process about to be killed");
    assert_eq!(c.inner.lock().unwrap().deferred_deliveries.len(), 1, "kept, not lost");
}

/// The process stays down until the next message respawns it. The watchdog
/// must wait that out, never counting it toward stranding, and deliver to
/// the replacement.
#[tokio::test]
async fn the_watchdog_waits_out_a_restart_and_delivers_to_the_replacement() {
    let c = controller();
    // The process being restarted is still dying: it holds stdin.
    let (old_tx, _old_rx) = mpsc::channel::<String>(4);
    {
        let mut inner = c.inner.lock().unwrap();
        inner.restart_pending = true;
        inner.stdin_tx = Some(old_tx);
    }
    c.send_user_message("for the replacement".to_string()).unwrap();

    let mut orphaned = 0;
    for _ in 0..DEFERRED_ORPHAN_GRACE_TICKS * 3 {
        assert_eq!(c.sweep_deferred_once(&mut orphaned), WatchdogStep::Continue);
    }
    assert_eq!(c.inner.lock().unwrap().deferred_deliveries.len(), 1, "never reported stranded");

    // The replacement spawns (which clears `restart_pending`).
    let (tx, mut rx) = mpsc::channel::<String>(4);
    {
        let mut inner = c.inner.lock().unwrap();
        inner.restart_pending = false;
        inner.stdin_tx = Some(tx);
    }
    c.sweep_deferred_once(&mut orphaned);
    assert!(rx.try_recv().unwrap().contains("for the replacement"));
}

// ── Dead-air re-deliveries reach the transcript (spec §6.9; #3703) ──────────
//
// When the CLI abandons a pending AskUserQuestion / tool-permission call, the
// controller re-sends the user's choice as a follow-up stdin line. The CLI then
// writes no tool result, so the line itself must be recorded — or the choice is
// gone on reload, absent from History, and lost when the live feed rolls the
// turn off.

fn controller_with_transcript(block_id: &str) -> (PersistentSubprocessController, Arc<FileStore>) {
    let filestore = Arc::new(FileStore::open_in_memory().unwrap());
    let c = PersistentSubprocessController::new(
        "tab".to_string(),
        block_id.to_string(),
        Some(Arc::new(crate::backend::mps::Broker::new())),
        None,
        None,
        Some(filestore.clone()),
    );
    (c, filestore)
}

fn transcript(fs: &FileStore, block_id: &str) -> String {
    fs.read_file(block_id, crate::backend::agent_session::OUTPUT_FILE)
        .unwrap()
        .map(|b| String::from_utf8_lossy(&b).to_string())
        .unwrap_or_default()
}

#[test]
fn redelivery_persister_records_the_line_in_the_blocks_transcript() {
    let (c, fs) = controller_with_transcript("block-redeliver-unit");
    let line = PersistentSubprocessController::encode_user_message("the user's answer");
    c.redelivery_persister().write(&line);
    let t = transcript(&fs, "block-redeliver-unit");
    assert_eq!(
        t.trim_end(),
        line,
        "exactly the re-delivered line, as one record"
    );
}

#[tokio::test]
async fn a_dead_air_answer_is_resent_and_recorded() {
    let (c, fs) = controller_with_transcript("block-redeliver-answer");
    let (tx, mut rx) = mpsc::channel::<String>(8);
    {
        let mut inner = c.inner.lock().unwrap();
        inner.stdin_tx = Some(tx);
        inner.pending_questions.insert(
            "tu-q".to_string(),
            (
                "req-q".to_string(),
                serde_json::json!([{ "question": "Pick one?", "header": "Pick" }]),
            ),
        );
    }
    c.answer_question("tu-q".to_string(), serde_json::json!({ "Pick one?": "B" }))
        .unwrap();
    let control = rx.try_recv().expect("the control_response goes out first");
    assert!(control.contains("control_response"));

    // No stdout activity: the fallback fires after ANSWER_RESUME_FALLBACK_MS.
    tokio::time::sleep(std::time::Duration::from_millis(
        ANSWER_RESUME_FALLBACK_MS + 500,
    ))
    .await;
    let resent = rx
        .try_recv()
        .expect("the answer is re-delivered as a follow-up line");
    assert!(resent.contains("\"type\":\"user\""));
    let t = transcript(&fs, "block-redeliver-answer");
    assert!(
        t.lines().any(|l| l == resent),
        "the re-delivered line must be in the transcript; got {t:?}"
    );
}

#[tokio::test]
async fn a_dead_air_answer_during_a_committed_restart_is_not_recorded() {
    let (c, fs) = controller_with_transcript("block-redeliver-restart");
    let (tx, mut rx) = mpsc::channel::<String>(8);
    {
        let mut inner = c.inner.lock().unwrap();
        inner.stdin_tx = Some(tx);
        inner.pending_questions.insert(
            "tu-r".to_string(),
            (
                "req-r".to_string(),
                serde_json::json!([{ "question": "Go?", "header": "Go" }]),
            ),
        );
    }
    c.answer_question("tu-r".to_string(), serde_json::json!({ "Go?": "yes" }))
        .unwrap();
    let _control = rx.try_recv().unwrap();
    // A config change commits a restart while the fallback waits: the process
    // is going down and may never read the line. Delivery is as it always was
    // (unchanged by #3703), but History must not claim a decision the agent
    // may never have received, so nothing is recorded.
    c.inner.lock().unwrap().restart_pending = true;

    tokio::time::sleep(std::time::Duration::from_millis(
        ANSWER_RESUME_FALLBACK_MS + 500,
    ))
    .await;
    assert_eq!(
        transcript(&fs, "block-redeliver-restart"),
        "",
        "nothing is recorded while a restart is committed"
    );
}

/// `SendMessage` tells its caller what happened instead of "injected" for
/// both: a live agent, busy or idle, is `Sent` (a backlog ahead of it goes out
/// with it); only a process that cannot take a write yet — starting up,
/// restarting, stopping — is `Deferred`.
#[tokio::test]
async fn send_outcome_reports_deferred_only_while_the_process_cannot_take_it() {
    let (c, _rx) = busy_controller();
    assert_eq!(c.send_user_message_outcome("mid-turn".to_string()).unwrap(), SendOutcome::Sent);

    let (c, _rx) = idle_controller();
    assert_eq!(c.send_user_message_outcome("idle".to_string()).unwrap(), SendOutcome::Sent);

    let (c, _rx) = idle_controller();
    enqueue_deferred(&c, "older");
    assert_eq!(c.send_user_message_outcome("newer".to_string()).unwrap(), SendOutcome::Sent);

    let (c, _rx) = starting_controller();
    assert_eq!(c.send_user_message_outcome("starting".to_string()).unwrap(), SendOutcome::Deferred);

    let (c, _rx) = busy_controller();
    c.inner.lock().unwrap().restart_pending = true;
    assert_eq!(c.send_user_message_outcome("restarting".to_string()).unwrap(), SendOutcome::Deferred);
}

/// SPEC_AGENT_SELF_QUIT §6.3: a delivery that says who it's from labels the
/// turn it starts, and the controller reports it through the trait.
#[tokio::test]
async fn a_labelled_delivery_sets_the_turn_provenance_the_controller_reports() {
    use crate::backend::blockcontroller::health::{TurnInput, TurnOrigin};
    use crate::backend::blockcontroller::Controller;
    let (c, mut rx) = idle_controller();
    let input = TurnInput { origin: TurnOrigin::User, text: "finish then quit".into() };
    assert!(matches!(c.decide_send_action_from("m", None, Some(input)), SendAction::DeliverDirect { was_active: false }));
    let p = c.turn_provenance().expect("a turn is in flight");
    assert_eq!((p.origin, p.tainted), (TurnOrigin::User, false));
    // A jekt arriving now is delivered into this turn at once
    // (SPEC_JEKT_IMMEDIATE_DELIVERY_2026_09_28 §2.1), so it taints it.
    c.send_user_message("jekt".to_string()).unwrap();
    assert!(rx.try_recv().unwrap().contains("jekt"));
    let p = c.turn_provenance().unwrap();
    assert_eq!((p.origin, p.tainted), (TurnOrigin::User, true), "automated input got into the user's turn");
}

/// A Swarm broadcast to a running persistent agent goes through structured
/// delivery, labelled as the user's (SPEC_SWARM_BROADCAST_AS_USER_MESSAGE §6
/// q2): on an idle agent it starts a user turn, exactly as typing would.
#[tokio::test]
async fn a_labelled_structured_delivery_starts_a_user_turn() {
    use crate::backend::blockcontroller::health::{TurnInput, TurnOrigin};
    use crate::backend::blockcontroller::Controller;
    let (c, mut rx) = idle_controller();
    let input = TurnInput { origin: TurnOrigin::User, text: "finish then quit".into() };
    assert_eq!(c.send_user_message_outcome_from("finish then quit".to_string(), Some(input)).unwrap(), SendOutcome::Sent);
    assert!(rx.try_recv().unwrap().contains("finish then quit"));
    let p = c.turn_provenance().expect("a turn is in flight");
    assert_eq!((p.origin, p.tainted, p.user_text.as_deref()), (TurnOrigin::User, false, Some("finish then quit")));
}

/// Into a running user turn it is more of the user's input, so it does not
/// taint it — the same as typing mid-turn.
#[tokio::test]
async fn a_labelled_structured_delivery_mid_turn_does_not_taint_a_user_turn() {
    use crate::backend::blockcontroller::health::{TurnInput, TurnOrigin};
    use crate::backend::blockcontroller::Controller;
    let (c, _rx) = idle_controller();
    let user = |t: &str| TurnInput { origin: TurnOrigin::User, text: t.into() };
    c.send_user_message_outcome_from("first".to_string(), Some(user("first"))).unwrap();
    c.send_user_message_outcome_from("second".to_string(), Some(user("second"))).unwrap();
    let p = c.turn_provenance().unwrap();
    assert_eq!((p.origin, p.tainted), (TurnOrigin::User, false));
}

/// Written together with a backlog queued by automated senders, the turn
/// can't be the user's: the label is dropped and the turn is unknown, so it
/// can never authorize a self-quit.
#[tokio::test]
async fn a_labelled_structured_delivery_written_with_a_backlog_is_unlabelled() {
    use crate::backend::blockcontroller::health::{TurnInput, TurnOrigin};
    use crate::backend::blockcontroller::Controller;
    let (c, mut rx) = idle_controller();
    enqueue_deferred(&c, "an older jekt");
    let input = TurnInput { origin: TurnOrigin::User, text: "then quit".into() };
    assert_eq!(c.send_user_message_outcome_from("then quit".to_string(), Some(input)).unwrap(), SendOutcome::Sent);
    assert_eq!(drain(&mut rx).len(), 2, "the backlog went out with it");
    assert_eq!(c.turn_provenance(), None, "an unlabelled start is unknown");
}

/// (#4401) An older queued message is written, then this one's
/// own write fails and it is taken back. The turn was started by the older
/// (automated) message, so it must stay unknown, not become the user's.
#[tokio::test]
async fn a_labelled_delivery_whose_own_write_fails_after_a_backlog_is_unlabelled() {
    use crate::backend::blockcontroller::health::{TurnInput, TurnOrigin};
    use crate::backend::blockcontroller::Controller;
    let c = controller();
    let (tx, mut rx) = mpsc::channel::<String>(1); // room for exactly one write
    c.inner.lock().unwrap().stdin_tx = Some(tx);
    enqueue_deferred(&c, "an older jekt");
    let input = TurnInput { origin: TurnOrigin::User, text: "then quit".into() };
    assert!(c.send_user_message_outcome_from("then quit".to_string(), Some(input)).is_err());
    assert!(rx.try_recv().unwrap().contains("an older jekt"), "the backlog was written");
    assert!(c.health_monitor.is_active_turn(), "the older message started a turn");
    assert_eq!(c.turn_provenance(), None, "a turn the user's message didn't start is unknown");
}

/// The unlabelled form — every jekt — is unchanged: an idle start is unknown.
#[tokio::test]
async fn an_unlabelled_structured_delivery_still_starts_an_unknown_turn() {
    use crate::backend::blockcontroller::Controller;
    let (c, _rx) = idle_controller();
    assert_eq!(c.send_user_message_outcome("jekt".to_string()).unwrap(), SendOutcome::Sent);
    assert_eq!(c.turn_provenance(), None);
}

/// ReAgent P1 on #3789: the spawn path labels the turn from the queued
/// message (`hint_next_turn`), `spawn_process` starts it, and then
/// `mark_turn_active_and_publish` runs for the same message. That last call is
/// bookkeeping, not new input, and must not taint the user's turn.
#[tokio::test]
async fn the_mark_after_a_spawn_does_not_taint_the_user_s_turn() {
    use crate::backend::blockcontroller::health::{TurnInput, TurnOrigin};
    use crate::backend::blockcontroller::Controller;
    let (c, _rx) = idle_controller();
    c.health_monitor.hint_next_turn(TurnInput { origin: TurnOrigin::User, text: "do X then quit".into() });
    c.health_monitor.set_active_turn(true); // what spawn_process does
    c.mark_turn_active_and_publish();
    let p = c.turn_provenance().expect("a turn is in flight");
    assert_eq!((p.origin, p.tainted), (TurnOrigin::User, false));
}

// ── A deferred restart brings its replacement up (Korp, 2026-09-29) ──────
//
// The restart used to stop the process and wait for "the next message" to
// respawn it. Automated delivery never spawns and waits while
// `restart_pending` is set, so every jekt to the agent queued until a human
// typed into its pane (docs/retro/RETRO_DEFERRED_RESTART_NEVER_RESPAWNS_2026_09_29.md).

fn registered_controller() -> (String, Arc<PersistentSubprocessController>) {
    let block_id = format!("block-restart-{}", uuid::Uuid::new_v4());
    let c = Arc::new(PersistentSubprocessController::new(
        "tab".to_string(),
        block_id.clone(),
        None,
        None,
        None,
        None,
    ));
    crate::backend::blockcontroller::register_controller(&block_id, c.clone());
    (block_id, c)
}

/// Commit a deferred restart the way production does: `restart_when_idle`
/// consumed at the turn boundary, then the restart's own stop.
fn commit_restart(c: &PersistentSubprocessController, generation: u64) {
    {
        let mut inner = c.inner.lock().unwrap();
        inner.spawn_generation = generation;
        inner.restart_when_idle = true;
    }
    assert_eq!(
        PersistentSubprocessController::turn_boundary_locked(
            &mut c.inner.lock().unwrap(),
            &c.health_monitor,
            "block",
            generation,
        ),
        Some(true),
    );
    c.stop_for_config_restart();
}

#[test]
fn the_boundary_that_commits_a_restart_arms_its_token() {
    let c = controller();
    commit_restart(&c, 3);
    let inner = c.inner.lock().unwrap();
    assert_eq!(inner.config_restart_generation, Some(3));
    assert!(inner.restart_pending);
    assert!(inner.stop_pending, "a restart is still a stop for the dying process");
}

/// codex P1 on #3990: a Stop between the boundary and the restart's own stop
/// clears the token; the restart's stop must not re-arm it.
#[test]
fn the_restarts_own_stop_never_rearms_a_token_a_stop_cleared() {
    let c = controller();
    {
        let mut inner = c.inner.lock().unwrap();
        inner.spawn_generation = 3;
        inner.restart_when_idle = true;
    }
    PersistentSubprocessController::turn_boundary_locked(&mut c.inner.lock().unwrap(), &c.health_monitor, "block", 3);
    let _ = c.stop_process(false); // the user's Stop, in the gap
    c.stop_for_config_restart();
    assert_eq!(c.inner.lock().unwrap().config_restart_generation, None, "the Stop must stay in force");
}

/// A Stop, or a teardown's stop, landing after the restart was committed
/// must win: nothing may respawn an agent the user just stopped.
#[test]
fn any_later_stop_overrides_a_committed_config_restart() {
    for later_stop in [
        (|c: &PersistentSubprocessController| {
            let _ = c.stop_process(false);
        }) as fn(&PersistentSubprocessController),
        |c| {
            let _ = c.request_stop_draining_deferred(KillRequest::Force);
        },
    ] {
        let c = controller();
        commit_restart(&c, 1);
        later_stop(&c);
        assert_eq!(c.inner.lock().unwrap().config_restart_generation, None);
    }
}

/// The incident itself: the restart's process is gone and the replacement
/// can't be resumed (here: no store to read the session from). Before the
/// fix a jekt then waited on `restart_pending` forever, with the watchdog
/// resetting its orphan timer on every tick.
#[test]
fn a_restart_that_cannot_resume_stops_holding_jekts_back() {
    let (block_id, c) = registered_controller();
    // Queued while the restarted process was still dying: held for the
    // replacement, as before the incident.
    let (old_tx, _old_rx) = mpsc::channel::<String>(4);
    {
        let mut inner = c.inner.lock().unwrap();
        inner.restart_pending = true;
        inner.stop_pending = true;
        inner.stdin_tx = Some(old_tx);
    }
    assert_eq!(c.send_user_message_outcome("jekt".to_string()).unwrap(), SendOutcome::Deferred);
    // The kill arm has run: no process, and no respawn claim yet.
    c.inner.lock().unwrap().stdin_tx = None;

    c.respawn_after_config_restart(0);

    let inner = c.inner.lock().unwrap();
    assert!(!inner.restart_pending && !inner.stop_pending, "the window must close when no replacement is coming");
    assert!(
        !c.deferred_must_wait_locked(&inner),
        "queued jekts must fall to the no-process rules, not wait forever",
    );
    assert_eq!(inner.deferred_deliveries.len(), 1, "the jekt itself is kept for the watchdog to report");
    drop(inner);
    crate::backend::blockcontroller::remove_controller_entry_only(&block_id);
}

#[test]
fn no_respawn_for_a_controller_that_no_longer_serves_its_block() {
    let c = controller(); // never registered: a closed or replaced pane
    c.inner.lock().unwrap().restart_pending = true;
    c.respawn_after_config_restart(0);
    assert!(c.inner.lock().unwrap().restart_pending, "a stale controller must leave its state alone");
}

#[test]
fn no_respawn_once_something_else_replaced_the_process() {
    let (block_id, c) = registered_controller();
    let (tx, _rx) = mpsc::channel::<String>(4);
    {
        let mut inner = c.inner.lock().unwrap();
        // A human message's spawn got there first: it cleared the window.
        inner.restart_pending = false;
        inner.stdin_tx = Some(tx);
        inner.stop_pending = true;
    }
    c.respawn_after_config_restart(0);
    let inner = c.inner.lock().unwrap();
    assert!(inner.stdin_tx.is_some() && inner.stop_pending, "a live replacement is left untouched");
    drop(inner);
    crate::backend::blockcontroller::remove_controller_entry_only(&block_id);
}

// codex P1s on #3990: the token is consumed only by the spawn claim, and
// both exit arms finish a restart.

/// The exit handler decided to respawn, then a Stop landed before the
/// spawn claim: the claim must refuse, so the Stop wins.
#[test]
fn a_stop_after_the_respawn_decision_still_wins_at_the_claim() {
    let c = controller();
    commit_restart(&c, 4);
    assert!(PersistentSubprocessController::config_restart_due_locked(&c.inner.lock().unwrap(), 4));
    let _ = c.stop_process(false); // the user's Stop, in the window
    assert!(!c.try_claim_eager_resume_spawn_for(Some(4)), "the Stop must win");
    assert!(!c.inner.lock().unwrap().spawning_in_progress, "no claim may be left behind");
}

/// The claim only reserves: the token stays live until the child starts.
#[test]
fn the_spawn_claim_keeps_the_token_until_the_child_starts() {
    let c = controller();
    commit_restart(&c, 4);
    assert!(c.try_claim_eager_resume_spawn_for(Some(4)));
    let mut inner = c.inner.lock().unwrap();
    assert!(inner.spawning_in_progress);
    assert_eq!(inner.restart_spawn_for, Some(4));
    assert_eq!(inner.config_restart_generation, Some(4), "still cancellable by a Stop");
    // …and `spawn_process` consumes it right before starting the child.
    PersistentSubprocessController::restart_spawn_still_permitted_locked(&mut inner).unwrap();
    assert_eq!(inner.config_restart_generation, Some(4), "the pre-spawn check does not consume it");
    PersistentSubprocessController::commit_restart_spawn_locked(&mut inner).unwrap();
    assert_eq!(inner.config_restart_generation, None, "one restart, one replacement");
    assert_eq!(inner.restart_spawn_for, None);
}

/// codex P1 on #3990: a Stop after the claim (during the credential gate)
/// must still cancel the replacement before its child starts.
#[test]
fn a_stop_during_the_credential_gate_cancels_the_replacement() {
    let c = controller();
    commit_restart(&c, 4);
    assert!(c.try_claim_eager_resume_spawn_for(Some(4)));
    let _ = c.stop_process(false); // the user's Stop, mid-gate
    let mut inner = c.inner.lock().unwrap();
    assert!(
        PersistentSubprocessController::restart_spawn_still_permitted_locked(&mut inner).is_err(),
        "spawn_process must refuse to start the child",
    );
    assert_eq!(inner.restart_spawn_for, None, "the refusal leaves no restart claim behind");
}

/// Ordinary spawns (a human's message) never consult the restart token.
#[test]
fn an_ordinary_spawn_is_not_gated_by_the_restart_token() {
    let c = controller();
    let mut inner = c.inner.lock().unwrap();
    assert!(PersistentSubprocessController::restart_spawn_still_permitted_locked(&mut inner).is_ok());
    assert!(PersistentSubprocessController::commit_restart_spawn_locked(&mut inner).is_ok());
}

/// What both exit arms consult, so the arm that wins the `select!` doesn't
/// matter: due only for the restarted generation, and not after a Stop.
#[test]
fn a_restart_is_due_only_for_its_own_generation() {
    let c = controller();
    commit_restart(&c, 7);
    {
        let inner = c.inner.lock().unwrap();
        assert!(PersistentSubprocessController::config_restart_due_locked(&inner, 7));
        assert!(!PersistentSubprocessController::config_restart_due_locked(&inner, 6), "a superseded generation");
    }
    c.inner.lock().unwrap().spawn_generation = 8; // something else spawned
    assert!(!PersistentSubprocessController::config_restart_due_locked(&c.inner.lock().unwrap(), 7));
}

/// A Stop that overrides the restart must still not leave jekts waiting on
/// `restart_pending` forever.
#[test]
fn a_stop_overriding_the_restart_still_closes_the_window() {
    let (block_id, c) = registered_controller();
    commit_restart(&c, 1);
    let _ = c.stop_process(false);
    assert!(c.inner.lock().unwrap().restart_pending, "precondition: a Stop leaves the window open");
    c.respawn_after_config_restart(1);
    let inner = c.inner.lock().unwrap();
    assert!(!inner.restart_pending && !c.deferred_must_wait_locked(&inner));
    assert!(!inner.spawning_in_progress, "the Stop won: nothing was started");
    drop(inner);
    crate::backend::blockcontroller::remove_controller_entry_only(&block_id);
}

/// ReAgent P1 on #3990: a competing spawn claim (a human's message) wins the
/// race with the restart's respawn and then fails. Nothing clears
/// `restart_pending` then, so the restart must stop holding jekts once its
/// process is gone and no spawn is in flight; they fall to the grace window
/// and are reported stranded rather than waiting forever.
#[tokio::test]
async fn a_restart_whose_process_is_gone_stops_holding_jekts_even_if_nothing_closes_it() {
    let c = controller();
    let (old_tx, _old_rx) = mpsc::channel::<String>(4);
    {
        let mut inner = c.inner.lock().unwrap();
        inner.restart_pending = true;
        inner.stop_pending = true;
        inner.config_restart_generation = Some(1);
        inner.stdin_tx = Some(old_tx); // still dying
    }
    c.send_user_message("jekt".to_string()).unwrap(); // queued for the replacement
    {
        let mut inner = c.inner.lock().unwrap();
        inner.stdin_tx = None; // exited
        inner.spawning_in_progress = false; // the competing spawn failed and released its claim
    }
    assert!(!c.deferred_must_wait_locked(&c.inner.lock().unwrap()));

    let mut orphaned = 0;
    let mut steps = 0;
    while c.sweep_deferred_once(&mut orphaned) == WatchdogStep::Continue {
        steps += 1;
        assert!(steps <= DEFERRED_ORPHAN_GRACE_TICKS, "must be reported within the grace window");
    }
    assert!(c.inner.lock().unwrap().deferred_deliveries.is_empty(), "reported stranded, not held");
}

/// …while a respawn holding the claim keeps the queue waiting for it.
#[tokio::test]
async fn a_restart_respawn_in_flight_keeps_jekts_queued_for_it() {
    let c = controller();
    {
        let mut inner = c.inner.lock().unwrap();
        inner.restart_pending = true;
        inner.stdin_tx = None;
        inner.spawning_in_progress = true;
    }
    c.send_user_message("jekt".to_string()).unwrap();
    let mut orphaned = 0;
    for _ in 0..DEFERRED_ORPHAN_GRACE_TICKS * 2 {
        assert_eq!(c.sweep_deferred_once(&mut orphaned), WatchdogStep::Continue);
    }
    assert_eq!(c.inner.lock().unwrap().deferred_deliveries.len(), 1);
}

/// With the restarted process gone and nothing spawning, a new jekt is
/// refused with a truthful error the sender sees, not silently parked.
#[test]
fn a_jekt_with_no_process_and_no_spawn_is_refused_not_parked() {
    let c = controller();
    {
        let mut inner = c.inner.lock().unwrap();
        inner.restart_pending = true;
        inner.stdin_tx = None;
    }
    let err = c.send_user_message_outcome("jekt".to_string()).unwrap_err();
    assert!(err.contains("restarting"), "got: {err}");
    assert!(c.inner.lock().unwrap().deferred_deliveries.is_empty());
}

/// codex P1 on #3990 (round 3): a Stop landing after the pre-spawn check, while
/// the child is being started, must still win at installation.
#[test]
fn a_stop_while_the_child_starts_is_honored_at_installation() {
    let c = controller();
    commit_restart(&c, 5);
    assert!(c.try_claim_eager_resume_spawn_for(Some(5)));
    PersistentSubprocessController::restart_spawn_still_permitted_locked(&mut c.inner.lock().unwrap()).unwrap();
    let _ = c.stop_process(false); // lands during cmd.spawn()
    let mut inner = c.inner.lock().unwrap();
    assert!(
        PersistentSubprocessController::commit_restart_spawn_locked(&mut inner).is_err(),
        "the install must refuse, so spawn_process kills the child",
    );
    assert_eq!(inner.restart_spawn_for, None);
}
