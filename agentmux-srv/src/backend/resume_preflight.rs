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
//! | no sid at all | spawns with no `--resume`; **no recovery scan runs on this path** | [`Verdict::Fresh`] |
//! | provider has no resume flag | resume isn't a concept here | [`Verdict::Unknown`] |
//!
//! The fourth row is the one worth stating explicitly, because it's the case
//! `docs/status/STATUS_CROSS_CHANNEL_RESUME_STALE_SESSION_ID_2026_08_20.md`
//! recorded and the one this whole module is aimed at: a spawn with no sid
//! does **not** consult the on-disk transcripts. A session may well be sitting
//! right there — [`Preflight::recoverable_session_id`] reports it when so —
//! but nothing in the current spawn path will reach for it, so the honest
//! verdict is `Fresh`. Reporting `Recover` here would describe a rehydrate
//! step that doesn't exist yet (that spec's Part B).

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
    /// Wire form, matching the TS union in `gotypes.d.ts`.
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
        // Report what's on disk, but don't let it change the verdict — the
        // no-sid spawn path never looks (module doc, fourth row).
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
        let slug = session_backfill::encode_project_slug(working_dir);
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
    /// `Fresh` — the no-resume spawn path never scans — and the reachable
    /// session is reported separately as evidence, not as a promise.
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
}
