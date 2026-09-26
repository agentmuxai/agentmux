// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Answer "will this pane resume its conversation, or start a new one?" at
//! **pane-open time**, before anything is spawned.
//!
//! ## Why this exists
//!
//! The persistent controller spawns lazily, on the first message — so every
//! signal that reports what actually happened to a resume
//! (`agentmux_session_outcome`, `session:resume_failed`,
//! SPEC_AGENT_PANE_HISTORY_ALIGNMENT_2026_08_05.md Part A) can only speak
//! *after* the user has already typed. The lived experience that produces is
//! the one this module exists to remove: the pane opens showing a long prior
//! conversation, the user types into it, and only then does the transcript
//! clear and announce a new session. The information was knowable the whole
//! time; nothing asked for it.
//!
//! ## Why it can be known in advance
//!
//! `--resume <sid>` fails for exactly one reason in practice: the session's
//! `.jsonl` isn't under the `CLAUDE_CONFIG_DIR` the CLI is about to run with.
//! That is a file-existence question, and
//! `session_backfill::session_is_reachable` answers it using the same path
//! scheme and the same home-dir expansion the spawn path itself uses. So the
//! preflight is not a heuristic or a guess about CLI behaviour — it evaluates
//! the same condition the CLI will, just earlier.
//!
//! ## Faithfulness to the real spawn path
//!
//! [`preflight`] deliberately mirrors `persistent.rs`'s decision sequence
//! rather than modelling an idealized one, so its verdict and the eventual
//! outcome can't disagree:
//!
//! | Pane state | Spawn does | Verdict |
//! |---|---|---|
//! | sid held, transcript present | `--resume <sid>` succeeds | [`Verdict::Resume`] |
//! | sid held, transcript missing, another session on disk | rejected → `retry_after_resume_failure` → `find_recovery_session_id` resumes that one | [`Verdict::Recover`] |
//! | sid held, transcript missing, nothing else on disk | rejected → retry finds nothing → blank | [`Verdict::Fresh`] |
//! | no sid, pane renders history whose session is reachable | first spawn continues it (`find_continuation_session_id`) | [`Verdict::Resume`] |
//! | no sid otherwise | spawns with no `--resume`; **no disk scan runs on this path** | [`Verdict::Fresh`] |
//! | provider has no resume flag | resume isn't a concept here | [`Verdict::Unknown`] |
//!
//! The no-sid rows are the case
//! `docs/status/STATUS_CROSS_CHANNEL_RESUME_STALE_SESSION_ID_2026_08_20.md`
//! recorded: a pane opened in a new channel or version shows the agent's
//! whole prior conversation, but its spawn held no id. Since
//! SPEC_DURABLE_CONVERSATION_MEMORY_2026_09_23.md §5 P0a the spawn continues
//! the session of that rendered history. It still never scans the provider's
//! dir for some other session: one may be sitting there —
//! [`Preflight::recoverable_session_id`] reports it when so — but "largest
//! on disk" can be an archived conversation or a subagent's, so it stays
//! evidence, not a verdict.

use std::time::Instant;

use crate::backend::session_backfill;

/// What will happen to this pane's conversation on its next spawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The exact session this pane would resume is present and reachable.
    Resume,
    /// The held session id is unreachable, but a real session for this working
    /// dir is on disk and the recovery path will pick it up — continuity
    /// survives, after a visible "Reconnecting…" pause.
    Recover,
    /// The next spawn starts a conversation with none of the prior turns.
    Fresh,
    /// Not determinable — the provider has no simple-flag resume, or the pane
    /// carries no working dir / config dir to check against. Callers should
    /// stay silent rather than guess.
    Unknown,
}

impl Verdict {
    /// Wire form, matching the TS union in `srv-types.d.ts`.
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Resume => "resume",
            Verdict::Recover => "recover",
            Verdict::Fresh => "fresh",
            Verdict::Unknown => "unknown",
        }
    }
}

/// One line in the pane's progress list while the preflight runs. Mirrors the
/// launcher splash's `StageRow` shape (`agentmux-launcher/src/splash.rs`) so
/// the two read as the same idea in two places.
#[derive(Debug, Clone)]
pub struct Step {
    pub id: &'static str,
    pub label: String,
    /// Did this step find what it was looking for? A `false` here is normal
    /// (it's how the sequence narrows), not an error.
    pub ok: bool,
    pub detail: String,
    pub duration_ms: u64,
}

/// The full answer, including the trail of how it was reached.
#[derive(Debug, Clone)]
pub struct Preflight {
    pub verdict: Verdict,
    /// The session that would actually end up loaded — the held id for
    /// [`Verdict::Resume`], the recovered one for [`Verdict::Recover`], `None`
    /// otherwise.
    pub session_id: Option<String>,
    /// A real session found on disk that the next spawn will NOT reach for.
    /// Only ever `Some` alongside [`Verdict::Fresh`], where it's the evidence
    /// that this pane's history is recoverable in principle even though
    /// nothing will recover it today (see the module doc's fourth row).
    pub recoverable_session_id: Option<String>,
    pub steps: Vec<Step>,
    pub duration_ms: u64,
}

/// Everything [`preflight`] needs, lifted out of block meta by the caller so
/// this stays a pure-ish function over plain values (one `is_file` and at most
/// one `read_dir` of a single directory — no store, no block, no RPC types).
#[derive(Debug, Clone, Default)]
pub struct PreflightInput {
    /// `agent:resume_flag` — empty means this provider has no `--resume`.
    pub resume_flag: String,
    /// `agent:sessionid` — the id the next spawn would attempt, if any.
    pub session_id: String,
    /// `cmd:cwd` — unexpanded is fine; expansion matches the spawn path.
    pub working_dir: String,
    /// `CLAUDE_CONFIG_DIR` out of `cmd:env`.
    pub config_dir: String,
    /// The session of the history this pane renders
    /// (`persistent::pane_history_session_id`). A first spawn with no
    /// `session_id` continues it when reachable. Empty when there is none.
    pub history_session_id: String,
    /// The head of the agent's segment chain
    /// (`continuity_segments::chain_head`), for the spawn's resume gate
    /// (SPEC_RESUME_GATE_AND_SAME_IDENTITY_CONTINUATION_2026_09_25.md §4.2).
    /// `None` when the pane has no agent UID or the agent no chain.
    pub chain_head: Option<crate::backend::continuity_segments::Head>,
    /// The identity `config_dir` is signed in as
    /// (`account_email::identity_key_from_oauth_dir`), for the gate's
    /// identity check. `None` when unknown.
    pub identity_key: Option<String>,
}

/// The spawn's resume gate (`PersistentSubprocessController::apply_resume_gate`),
/// mirrored: a candidate that isn't the chain head is redirected to the head
/// when reachable here, else refused. `None` when the candidate stands.
fn gate(input: &PreflightInput, candidate: &str, relocate_only: bool, steps: &mut Vec<Step>) -> Option<(Verdict, Option<String>)> {
    use crate::backend::continuity_relocate::{is_resumable_file, source_file};
    use crate::backend::continuity_segments::{resume_gate, GateInput, ResumeGate};
    let t = Instant::now();
    let gate_input = GateInput {
        candidate,
        head: input.chain_head.as_ref(),
        identity: input.identity_key.as_deref(),
        poisoned: None,
        config_dir: &input.config_dir,
        cwd: &input.working_dir,
    };
    let decision = resume_gate(
        &gate_input,
        |sid| session_backfill::session_is_reachable(&input.config_dir, &input.working_dir, sid),
        |dir, sid| source_file(dir, &input.working_dir, sid).is_some_and(|p| is_resumable_file(&p)),
    );
    if relocate_only && !matches!(decision, ResumeGate::Relocate { .. }) {
        return None;
    }
    match decision {
        ResumeGate::Allow => None,
        ResumeGate::Relocate { head, from_config_dir } => {
            steps.push(step(
                "chain",
                "Checking the agent's conversation",
                true,
                format!("{head} is under another login of this identity ({from_config_dir}); continuing it there"),
                t,
            ));
            Some((Verdict::Resume, Some(head)))
        }
        ResumeGate::Redirect { head } => {
            steps.push(step("chain", "Checking the agent's conversation", true, format!("moved on to {head}"), t));
            Some((Verdict::Resume, Some(head)))
        }
        ResumeGate::Refuse { head } => {
            steps.push(step(
                "chain",
                "Checking the agent's conversation",
                false,
                format!("in {head}, which this spawn can't resume here"),
                t,
            ));
            Some((Verdict::Fresh, None))
        }
    }
}

fn step(id: &'static str, label: &str, ok: bool, detail: impl Into<String>, started: Instant) -> Step {
    Step {
        id,
        label: label.to_string(),
        ok,
        detail: detail.into(),
        duration_ms: started.elapsed().as_millis() as u64,
    }
}

/// Decide, without spawning anything, what the next spawn will do to this
/// pane's conversation. See the module doc for the mapping this mirrors.
pub fn preflight(input: &PreflightInput) -> Preflight {
    let overall = Instant::now();
    let mut steps: Vec<Step> = Vec::new();

    let finish = |verdict: Verdict,
                  session_id: Option<String>,
                  recoverable_session_id: Option<String>,
                  steps: Vec<Step>| Preflight {
        verdict,
        session_id,
        recoverable_session_id,
        steps,
        duration_ms: overall.elapsed().as_millis() as u64,
    };

    // 1. Can this provider resume at all?
    let t = Instant::now();
    if input.resume_flag.is_empty() {
        steps.push(step("provider", "Checking provider", false, "no resume flag", t));
        return finish(Verdict::Unknown, None, None, steps);
    }
    if input.config_dir.is_empty() {
        // Without a config dir there's nowhere to look; guessing "fresh" here
        // would warn on panes we know nothing about.
        steps.push(step("provider", "Checking provider", false, "no config dir", t));
        return finish(Verdict::Unknown, None, None, steps);
    }
    steps.push(step("provider", "Checking provider", true, input.resume_flag.clone(), t));

    // 2. Is there a session id to resume in the first place?
    let t = Instant::now();
    if input.session_id.is_empty() {
        steps.push(step("session-id", "Resolving session id", false, "none recorded", t));
        // The first spawn continues the conversation this pane renders when
        // `--resume` can reach it (module doc, fourth row).
        if !input.history_session_id.is_empty() {
            let t = Instant::now();
            let sid = input.history_session_id.clone();
            if session_backfill::session_is_reachable(&input.config_dir, &input.working_dir, &sid) {
                steps.push(step("history", "Continuing this pane's conversation", true, sid.clone(), t));
                if let Some((verdict, sid)) = gate(input, &sid, false, &mut steps) {
                    return finish(verdict, sid, None, steps);
                }
                return finish(Verdict::Resume, Some(sid), None, steps);
            }
            steps.push(step(
                "history",
                "Continuing this pane's conversation",
                false,
                format!("{sid} not under this config dir"),
                t,
            ));
            // Relocation only, as the spawn does: a first spawn resumes a
            // session it can't reach here only by relocating it.
            if let Some((verdict, sid)) = gate(input, &sid, true, &mut steps) {
                return finish(verdict, sid, None, steps);
            }
        }
        // Report what's on disk, but don't let it change the verdict — the
        // spawn only continues the pane's own history, never a disk scan.
        let t = Instant::now();
        let on_disk = session_backfill::find_largest_session_for_working_dir(
            &input.config_dir,
            &input.working_dir,
        );
        match &on_disk {
            Some(sid) => steps.push(step(
                "scan",
                "Scanning for recoverable sessions",
                false,
                format!("found {sid}, but a no-resume spawn won't load it"),
                t,
            )),
            None => steps.push(step("scan", "Scanning for recoverable sessions", false, "none on disk", t)),
        }
        return finish(Verdict::Fresh, None, on_disk, steps);
    }
    steps.push(step("session-id", "Resolving session id", true, input.session_id.clone(), t));
    if let Some((verdict, sid)) = gate(input, &input.session_id, false, &mut steps) {
        return finish(verdict, sid, None, steps);
    }

    // 3. Would `--resume <sid>` actually find it? Same check the CLI makes.
    let t = Instant::now();
    if session_backfill::session_is_reachable(&input.config_dir, &input.working_dir, &input.session_id) {
        steps.push(step("transcript", "Locating transcript", true, "reachable", t));
        return finish(Verdict::Resume, Some(input.session_id.clone()), None, steps);
    }
    steps.push(step("transcript", "Locating transcript", false, "not under this config dir", t));

    // 4. It's unreachable — the spawn will be rejected and recovery will run.
    //    Mirror `find_recovery_session_id`, including its refusal to "recover"
    //    the very id that just failed.
    let t = Instant::now();
    let recovered = session_backfill::find_largest_session_for_working_dir(
        &input.config_dir,
        &input.working_dir,
    )
    .filter(|sid| sid != &input.session_id);
    match recovered {
        Some(sid) => {
            steps.push(step("recovery", "Checking recovery", true, sid.clone(), t));
            finish(Verdict::Recover, Some(sid), None, steps)
        }
        None => {
            steps.push(step("recovery", "Checking recovery", false, "nothing to recover", t));
            finish(Verdict::Fresh, None, None, steps)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    /// Build a config dir containing `projects/<slug>/<sid>.jsonl` files of the
    /// given sizes, mirroring Claude Code's own on-disk layout.
    fn config_dir_with(working_dir: &str, sessions: &[(&str, usize)]) -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        let slug = crate::backend::claude_layout::project_dir_name(working_dir);
        let dir = tmp.path().join("projects").join(slug);
        fs::create_dir_all(&dir).unwrap();
        for (sid, size) in sessions {
            fs::write(dir.join(format!("{sid}.jsonl")), "x".repeat(*size)).unwrap();
        }
        tmp
    }

    fn input(config_dir: &Path, working_dir: &str, session_id: &str) -> PreflightInput {
        PreflightInput {
            resume_flag: "--resume".to_string(),
            session_id: session_id.to_string(),
            working_dir: working_dir.to_string(),
            config_dir: config_dir.to_string_lossy().to_string(),
            history_session_id: String::new(),
            chain_head: None,
            identity_key: None,
        }
    }

    const WORK_DIR: &str = "/home/dev/agents/agentx";

    #[test]
    fn a_reachable_session_resumes() {
        let cfg = config_dir_with(WORK_DIR, &[("sid-live", 4096)]);
        let out = preflight(&input(cfg.path(), WORK_DIR, "sid-live"));
        assert_eq!(out.verdict, Verdict::Resume);
        assert_eq!(out.session_id.as_deref(), Some("sid-live"));
        assert_eq!(out.recoverable_session_id, None);
    }

    /// The STATUS_STALE_RESUME_LIVE_REPRO_AND_FIX_PLAN_2026_08_23 §2 shape: the
    /// held pointer is a real but superseded session that no longer exists
    /// here, while the genuinely-live one sits on disk. Continuity survives via
    /// the recovery path, so the pane must NOT be warned that it's losing the
    /// conversation.
    #[test]
    fn an_unreachable_session_with_another_on_disk_recovers() {
        let cfg = config_dir_with(WORK_DIR, &[("sid-real", 900_000)]);
        let out = preflight(&input(cfg.path(), WORK_DIR, "sid-superseded"));
        assert_eq!(out.verdict, Verdict::Recover);
        assert_eq!(out.session_id.as_deref(), Some("sid-real"));
    }

    /// `find_recovery_session_id` refuses to "recover" the id that just failed;
    /// the preflight must refuse it too, or it would promise a resume that the
    /// real path is specifically coded to reject.
    #[test]
    fn the_failed_id_is_never_offered_back_as_a_recovery() {
        let cfg = config_dir_with(WORK_DIR, &[]);
        // The dir exists but holds nothing — the only "candidate" would be the
        // attempted id itself if the scan somehow returned it.
        let out = preflight(&input(cfg.path(), WORK_DIR, "sid-gone"));
        assert_eq!(out.verdict, Verdict::Fresh);
        assert_eq!(out.session_id, None);
    }

    #[test]
    fn an_unreachable_session_with_nothing_on_disk_is_fresh() {
        let cfg = tempfile::tempdir().unwrap();
        let out = preflight(&input(cfg.path(), WORK_DIR, "sid-gone"));
        assert_eq!(out.verdict, Verdict::Fresh);
        assert_eq!(out.session_id, None);
        assert_eq!(out.recoverable_session_id, None);
    }

    /// The cross-channel open this module exists for: no pointer at all, but
    /// the real conversation is right there on disk. The verdict is still
    /// `Fresh` when the pane renders no history of its own — the spawn never
    /// scans the provider's dir — and the reachable session is reported
    /// separately as evidence, not as a promise.
    #[test]
    fn no_session_id_is_fresh_even_when_a_session_exists_on_disk() {
        let cfg = config_dir_with(WORK_DIR, &[("sid-orphaned", 2_000_000)]);
        let out = preflight(&input(cfg.path(), WORK_DIR, ""));
        assert_eq!(out.verdict, Verdict::Fresh);
        assert_eq!(out.session_id, None);
        assert_eq!(
            out.recoverable_session_id.as_deref(),
            Some("sid-orphaned"),
            "the orphaned session must be reported so the UI can say history exists",
        );
    }

    /// The 2026-09-23 incident: a pane in a new version renders the agent's
    /// prior conversation but holds no id. The spawn now continues it.
    #[test]
    fn no_session_id_but_reachable_rendered_history_resumes_it() {
        let cfg = config_dir_with(WORK_DIR, &[("sid-rendered", 500), ("sid-bigger", 2_000_000)]);
        let mut inp = input(cfg.path(), WORK_DIR, "");
        inp.history_session_id = "sid-rendered".to_string();
        let out = preflight(&inp);
        assert_eq!(out.verdict, Verdict::Resume);
        assert_eq!(
            out.session_id.as_deref(),
            Some("sid-rendered"),
            "the rendered history's session wins, not the largest on disk",
        );
    }

    #[test]
    fn no_session_id_and_unreachable_rendered_history_is_fresh() {
        let cfg = config_dir_with(WORK_DIR, &[("sid-orphaned", 2_000_000)]);
        let mut inp = input(cfg.path(), WORK_DIR, "");
        inp.history_session_id = "sid-other-account".to_string();
        let out = preflight(&inp);
        assert_eq!(out.verdict, Verdict::Fresh);
        assert_eq!(out.recoverable_session_id.as_deref(), Some("sid-orphaned"));
        assert!(out.steps.iter().any(|s| s.id == "history" && !s.ok));
    }

    #[test]
    fn no_session_id_and_no_transcripts_is_plainly_fresh() {
        let cfg = tempfile::tempdir().unwrap();
        let out = preflight(&input(cfg.path(), WORK_DIR, ""));
        assert_eq!(out.verdict, Verdict::Fresh);
        assert_eq!(out.recoverable_session_id, None);
    }

    /// Staying silent matters as much as warning: a provider with no resume
    /// flag, or a pane with no config dir, must not be told it's about to lose
    /// a conversation we never had any way to check.
    #[test]
    fn an_unknowable_pane_reports_unknown_rather_than_guessing() {
        let cfg = tempfile::tempdir().unwrap();

        let mut no_flag = input(cfg.path(), WORK_DIR, "sid");
        no_flag.resume_flag = String::new();
        assert_eq!(preflight(&no_flag).verdict, Verdict::Unknown);

        let mut no_config = input(cfg.path(), WORK_DIR, "sid");
        no_config.config_dir = String::new();
        assert_eq!(preflight(&no_config).verdict, Verdict::Unknown);
    }

    /// The steps are the pane's progress list, so every path must produce a
    /// non-empty, labelled trail — including the early returns.
    #[test]
    fn every_path_reports_labelled_steps() {
        let cfg = config_dir_with(WORK_DIR, &[("sid-live", 10)]);
        for inp in [
            input(cfg.path(), WORK_DIR, "sid-live"),
            input(cfg.path(), WORK_DIR, "sid-missing"),
            input(cfg.path(), WORK_DIR, ""),
            PreflightInput::default(),
        ] {
            let out = preflight(&inp);
            assert!(!out.steps.is_empty(), "every verdict must show its work");
            assert!(
                out.steps.iter().all(|s| !s.label.is_empty() && !s.id.is_empty()),
                "every step needs an id and a human label",
            );
        }
    }

    fn head(sid: &str, identity: Option<&str>) -> crate::backend::continuity_segments::Head {
        crate::backend::continuity_segments::Head {
            session_id: sid.into(),
            identity_key: identity.map(str::to_string),
            ..Default::default()
        }
    }

    // ── the resume gate, mirrored (SPEC_RESUME_GATE_AND_SAME_IDENTITY_CONTINUATION §4.2) ──

    /// The pane holds S1 and both files are here, but the agent's
    /// conversation moved on to S2: the spawn resumes S2, so the verdict
    /// names S2.
    #[test]
    fn a_held_session_that_is_not_the_chain_head_resumes_the_head() {
        let cfg = config_dir_with(WORK_DIR, &[("sid-old", 4096), ("sid-head", 4096)]);
        let mut i = input(cfg.path(), WORK_DIR, "sid-old");
        i.chain_head = Some(head("sid-head", None));
        let out = preflight(&i);
        assert_eq!(out.verdict, Verdict::Resume);
        assert_eq!(out.session_id.as_deref(), Some("sid-head"));
    }

    /// The head lives under another login: the spawn resumes nothing, so the
    /// held (stale but reachable) session must not be promised.
    #[test]
    fn a_held_session_that_is_not_the_chain_head_starts_fresh_when_the_head_is_elsewhere() {
        let cfg = config_dir_with(WORK_DIR, &[("sid-old", 4096)]);
        let mut i = input(cfg.path(), WORK_DIR, "sid-old");
        i.chain_head = Some(head("sid-elsewhere", None));
        let out = preflight(&i);
        assert_eq!(out.verdict, Verdict::Fresh);
        assert_eq!(out.session_id, None);
    }

    #[test]
    fn a_rendered_history_that_is_not_the_chain_head_resumes_the_head() {
        let cfg = config_dir_with(WORK_DIR, &[("sid-old", 4096), ("sid-head", 4096)]);
        let mut i = input(cfg.path(), WORK_DIR, "");
        i.history_session_id = "sid-old".into();
        i.chain_head = Some(head("sid-head", None));
        let out = preflight(&i);
        assert_eq!(out.verdict, Verdict::Resume);
        assert_eq!(out.session_id.as_deref(), Some("sid-head"));
    }

    #[test]
    fn the_chain_head_itself_resumes_unchanged() {
        let cfg = config_dir_with(WORK_DIR, &[("sid-live", 4096)]);
        let mut i = input(cfg.path(), WORK_DIR, "sid-live");
        i.chain_head = Some(head("sid-live", None));
        let out = preflight(&i);
        assert_eq!(out.verdict, Verdict::Resume);
        assert_eq!(out.session_id.as_deref(), Some("sid-live"));
        assert!(out.steps.iter().all(|s| s.id != "chain"), "no gate step when the candidate is the head");
    }

    /// The config dir now signs in as someone else: the head's file is here,
    /// but resuming it would cross identities.
    #[test]
    fn the_head_under_another_identity_starts_fresh() {
        let cfg = config_dir_with(WORK_DIR, &[("sid-live", 4096)]);
        let mut i = input(cfg.path(), WORK_DIR, "sid-live");
        i.chain_head = Some(head("sid-live", Some("k-old")));
        i.identity_key = Some("k-new".into());
        let out = preflight(&i);
        assert_eq!(out.verdict, Verdict::Fresh);
        assert_eq!(out.session_id, None);
    }

    /// A rebuild onto a new login of the same identity: the head is only
    /// under the old login, and the spawn will relocate and fork it.
    #[test]
    fn a_head_under_another_login_of_the_same_identity_resumes() {
        let new_login = config_dir_with(WORK_DIR, &[]);
        let old_login = config_dir_with(WORK_DIR, &[]);
        let slug = crate::backend::claude_layout::project_dir_name(WORK_DIR);
        fs::write(old_login.path().join("projects").join(&slug).join("sid-head.jsonl"), "{\"type\":\"user\"}\n").unwrap();
        let head = crate::backend::continuity_segments::Head {
            session_id: "sid-head".into(),
            identity_key: Some("k1".into()),
            config_dir: Some(old_login.path().to_string_lossy().to_string()),
            provider: "claude".into(),
            cwd: WORK_DIR.into(),
        };

        let mut held = input(new_login.path(), WORK_DIR, "sid-head");
        held.chain_head = Some(head.clone());
        held.identity_key = Some("k1".into());
        let out = preflight(&held);
        assert_eq!(out.verdict, Verdict::Resume);
        assert_eq!(out.session_id.as_deref(), Some("sid-head"));

        let mut first = input(new_login.path(), WORK_DIR, "");
        first.history_session_id = "sid-head".into();
        first.chain_head = Some(head.clone());
        first.identity_key = Some("k1".into());
        assert_eq!(preflight(&first).verdict, Verdict::Resume, "a first spawn relocates its history");

        first.identity_key = Some("k-other".into());
        assert_eq!(preflight(&first).verdict, Verdict::Fresh, "never across identities");
    }
}
