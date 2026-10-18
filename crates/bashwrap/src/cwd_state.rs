// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Where an agent's Bash calls start: the working directory a call ends in is
//! kept for the next one, per instance and, for a Claude Code subagent, per
//! subagent (docs/specs/SPEC_BASHWRAP_PER_AGENT_CWD_2026_10_18.md). Moved out
//! of `bash_wrap.rs`, which runs the calls.

use std::path::{Path, PathBuf};
use std::time::Duration;

// ─────────────────────────────────────────────────────────────────────────────
// Cwd persistence across one-shot `exec` invocations
//
// Every Bash tool call the PreToolUse hook rewrites spawns a brand-new,
// disposable `agentmux-bashwrap exec` process (see main.rs's module doc —
// there is no daemon). Historically the inner bash's starting directory was
// seeded from `std::env::current_dir()`, i.e. THIS process's own inherited
// cwd, which is always the agent's fixed home directory (nothing in this
// process tree ever chdirs it). A `cd` run by the wrapped command only ever
// affected that one throwaway inner bash; the moment it exited, the change
// was gone, and the NEXT call started over at the same fixed directory again
// — every time, not intermittently. Claude Code's own Bash tool expects `cd`
// continuity across calls within a session and (at least on Windows, where
// this rewrite always applies) surfaces the mismatch as a
// "Shell cwd was reset to ..." notice on effectively every call. See
// docs/retro/RETRO_BASH_CWD_RESET_NOTICE_WINDOWS_2026_08_02.md for the full
// investigation.
//
// The fix: persist the shell's ending `$PWD` to a small per-agent state file
// after each call, and restore it as the starting directory of the next
// call. This doesn't change what Claude Code itself believes (that's opaque,
// out of process) but it does make the ACTUAL directory a `cd` in one Bash
// call leaves the agent in survive to the next call, which is the part that
// was silently broken.
// ─────────────────────────────────────────────────────────────────────────────

/// Env var bashwrap sets on the *child* bash process (never interpolated
/// into the script text, so no path-quoting concerns) pointing at the state
/// file `append_cwd_capture`'s appended script writes the ending `$PWD` to.
pub(crate) const CWD_STATE_ENV: &str = "AGENTMUX_BASHWRAP_CWD_STATE";

/// Optional override of the state file's own path, read from bashwrap's own
/// process env (distinct from `CWD_STATE_ENV`, which bashwrap sets rather
/// than reads). Exists so tests can point persistence at a tempdir instead
/// of a real agent's `~/.agentmux` state, and as an escape hatch for manual
/// debugging.
pub(crate) const CWD_STATE_FILE_OVERRIDE_ENV: &str = "AGENTMUX_BASHWRAP_CWD_STATE_FILE";

/// Resolves the cwd-persistence file location and any previously-restored
/// starting directory once per `exec` invocation.
pub(crate) struct CwdState {
    /// Where the ending `$PWD` gets written after this command runs, for the
    /// *next* `exec` invocation to pick up. `None` if we couldn't resolve a
    /// location or create its parent dir — persistence is then simply
    /// skipped for this call (never a hard failure; falls back to today's
    /// pre-fix behavior).
    pub(crate) path: Option<PathBuf>,
    /// The directory to actually start this command's shell in: the
    /// previously-persisted directory if one was found and still exists,
    /// otherwise this process's own (fixed, inherited) cwd — the same
    /// default used before this fix existed.
    pub(crate) start_dir: Option<PathBuf>,
}

impl CwdState {
    /// [`CwdState::load`], or, when the call's agent can't be told
    /// (`Args::agent_unknown`), the directory the shell was started in and
    /// nothing written: any agent's would be a guess.
    pub(crate) fn for_call(agent: Option<&str>, agent_unknown: bool) -> Self {
        if agent_unknown {
            return Self { path: None, start_dir: std::env::current_dir().ok() };
        }
        Self::load(agent)
    }

    /// `agent`: the subagent that made the call, if any. A subagent keeps
    /// its own state file next to the instance's: it starts where it last
    /// left off, or, on its first call, where the main agent is; and its
    /// `cd` is written only to its own file. Before this, the main agent and
    /// every subagent shared the instance's file, so a subagent's `cd` moved
    /// the main agent's next command into the subagent's checkout, where a
    /// `git commit` committed the subagent's work
    /// (docs/specs/SPEC_BASHWRAP_PER_AGENT_CWD_2026_10_18.md).
    pub(crate) fn load(agent: Option<&str>) -> Self {
        let mut path = cwd_state_path();
        if let Some(dir) = path.as_deref().and_then(Path::parent) {
            if let Err(e) = std::fs::create_dir_all(dir) {
                tracing::warn!(
                    target: "bashwrap",
                    error = %e,
                    dir = %dir.display(),
                    "failed to create cwd-state dir; cwd persistence disabled for this call",
                );
                // reagentx P2 (re-review): the log line above claims
                // persistence is disabled, but `path` used to stay `Some`
                // here regardless — restore_cwd below would still attempt
                // (and silently fail) a read, and the child would still get
                // CWD_STATE_ENV pointed at a directory that doesn't exist,
                // so append_cwd_capture's write would silently fail on
                // every single call forever instead of being skipped once,
                // as documented. Actually clear it so both sides honor the
                // "disabled" contract `path`'s own doc comment promises.
                path = None;
            }
        }
        let agent = agent.map(str::trim).filter(|a| !a.is_empty());
        let own = path.as_deref().zip(agent).map(|(p, a)| agent_state_path(p, a));
        let start_dir = own
            .as_deref()
            .and_then(restore_cwd)
            .or_else(|| path.as_deref().and_then(restore_cwd))
            .or_else(|| std::env::current_dir().ok());
        // Subagents come and go; the main agent's calls clear out the files of
        // ones long gone.
        if agent.is_none() {
            if let Some(dir) = path.as_deref().and_then(Path::parent) {
                sweep_agent_state_files(dir, AGENT_STATE_MAX_AGE);
            }
        }
        Self { path: own.or(path), start_dir }
    }
}

/// A subagent's state file: the instance's, with the agent in its name
/// (`<instance>.agent-<agent>.cwd`), so it sorts and sweeps with it.
fn agent_state_path(instance_file: &Path, agent: &str) -> PathBuf {
    let stem = instance_file.file_stem().and_then(|s| s.to_str()).unwrap_or("bashwrap");
    instance_file.with_file_name(format!("{stem}{AGENT_STATE_MARK}{}.cwd", sanitize_state_key(agent)))
}

const AGENT_STATE_MARK: &str = ".agent-";

/// A subagent's state file untouched this long belongs to one that ended.
const AGENT_STATE_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// Delete subagents' state files in `dir` not written for `max_age`. Never
/// an instance's own file (no agent mark), and never fatal.
fn sweep_agent_state_files(dir: &Path, max_age: Duration) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.contains(AGENT_STATE_MARK) || !name.ends_with(".cwd") {
            continue;
        }
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age >= max_age);
        if old {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Per-*instance* state file path. Keyed by `AGENTMUX_INSTANCE_SLUG` — set by
/// `agent-model.ts` to a per-launch slug that also seeds that instance's own
/// unique working directory (`${agentmuxHome()}/agents/${instanceSlug}`) —
/// NOT `AGENTMUX_AGENT_ID`, which is the definition-wide slug shared by every
/// named instance launched from the same agent definition (codex P1 on this
/// PR: keying by `AGENTMUX_AGENT_ID` made two concurrently-running instances
/// of the same definition silently share one cwd file, so one instance's
/// `cd` could redirect the other's next command into the wrong worktree).
/// Falls back to `AGENTMUX_AGENT_ID` for older callers that only set that,
/// then to a sanitized form of this process's own cwd (always the instance's
/// fixed home directory either way) for the rare case neither is set, e.g. a
/// manual invocation outside AgentMux.
fn cwd_state_path() -> Option<PathBuf> {
    if let Ok(over) = std::env::var(CWD_STATE_FILE_OVERRIDE_ENV) {
        if !over.is_empty() {
            return Some(PathBuf::from(over));
        }
    }
    let dir = dirs::home_dir()?
        .join(".agentmux")
        .join("state")
        .join("bashwrap-cwd");
    let non_empty = |s: String| (!s.is_empty()).then_some(s);
    let key = std::env::var("AGENTMUX_INSTANCE_SLUG")
        .ok()
        .and_then(non_empty)
        .or_else(|| std::env::var("AGENTMUX_AGENT_ID").ok().and_then(non_empty))
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .map(|p| p.to_string_lossy().into_owned())
        })?;
    Some(dir.join(format!("{}.cwd", sanitize_state_key(&key))))
}

/// Filesystem-safe form of an arbitrary key for use as a filename component.
fn sanitize_state_key(raw: &str) -> String {
    raw.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Read back a previously-persisted cwd, if the file exists, is non-empty,
/// and still names a real directory (it may not — the directory could have
/// been deleted since, or this is the first call for this agent).
fn restore_cwd(state_path: &Path) -> Option<PathBuf> {
    let contents = std::fs::read_to_string(state_path).ok()?;
    let trimmed = contents.trim();
    if trimmed.is_empty() {
        return None;
    }
    let path = PathBuf::from(trimmed);
    path.is_dir().then_some(path)
}

/// Appends bookkeeping to `command_block` that (1) captures its real exit
/// code before running anything else, (2) persists the shell's ending
/// `$PWD` to `$AGENTMUX_BASHWRAP_CWD_STATE` if that env var is set (skipped
/// entirely otherwise — e.g. when `CwdState::path` was `None`), written via
/// a temp-file-then-rename so a mid-write crash can't leave a half-written
/// state file, and (3) re-exits with the captured code so this bookkeeping
/// never changes the command's real exit status. `command_block` must be a
/// brace group or bare command, not something that already ends in its own
/// `exit`.
///
/// The temp file is suffixed with `$$` (this bash's own PID), not a fixed
/// name — reagentx P2 on this PR: two Bash tool calls for the same
/// agent/instance can run concurrently (Claude issues parallel tool calls),
/// and a shared fixed temp name meant two concurrent writers could
/// interleave writes to the *same* temp file before either renamed it,
/// corrupting the persisted cwd. Per-PID temp names make each writer's
/// temp-then-rename independent; the only remaining race is which
/// completed rename lands last, which is an inherent, expected ambiguity
/// for genuinely concurrent `cd`s (no different from two parallel shells
/// racing to `cd` a shared session) — not data corruption.
pub(crate) fn append_cwd_capture(command_block: &str) -> String {
    // `pwd` alone prints MSYS-style paths under Git Bash (`/c/Users/...`),
    // which Rust's `std::fs`/`Path::is_dir()` on native Windows can't
    // resolve (it looks for a literal `\c\Users\...` under the current
    // drive root). `-W` is Git Bash's own flag for "print the Windows-
    // native form instead" (`C:/Users/...`), which both `restore_cwd`'s
    // `is_dir()` check and `cmd.cwd()`/`cmd.current_dir()` understand
    // correctly. Not a POSIX `pwd` flag — Unix has no such mismatch to
    // work around, so plain `pwd` is correct there.
    let pwd_cmd = if cfg!(windows) { "pwd -W" } else { "pwd" };
    format!(
        "{command_block}\n\
__agentmux_bashwrap_rc=$?\n\
if [ -n \"${CWD_STATE_ENV}\" ]; then\n\
  __agentmux_bashwrap_cwd_tmp=\"${CWD_STATE_ENV}.tmp.$$\"\n\
  {pwd_cmd} > \"$__agentmux_bashwrap_cwd_tmp\" 2>/dev/null && mv -f \"$__agentmux_bashwrap_cwd_tmp\" \"${CWD_STATE_ENV}\" 2>/dev/null\n\
fi\n\
exit $__agentmux_bashwrap_rc"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_env_lock::{unique_temp_dir, EnvVarGuard, ENV_LOCK};

    // ── cwd persistence tests ───────────────────────────────────────────────
    // See docs/retro/RETRO_BASH_CWD_RESET_NOTICE_WINDOWS_2026_08_02.md for
    // the bug this exists to fix: each `exec` invocation used to always seed
    // its inner bash's cwd from this process's own (fixed) cwd, silently
    // discarding whatever directory a previous call's `cd` left the agent
    // in.

    #[test]
    fn sanitize_state_key_replaces_unsafe_chars() {
        assert_eq!(sanitize_state_key("agent3-0630k"), "agent3-0630k");
        assert_eq!(sanitize_state_key("Agent With Spaces"), "Agent_With_Spaces");
        assert_eq!(sanitize_state_key(r"C:\Users\user"), "C__Users_user");
    }

    /// SPEC_BASHWRAP_PER_AGENT_CWD_2026_10_18.md: the main agent and its
    /// subagents share one instance, and used to share one state file, so a
    /// subagent's `cd` into its own checkout moved the main agent's next
    /// command there. Each subagent now has its own file; its first call
    /// starts where the main agent is.
    #[test]
    fn a_subagents_cd_never_moves_the_main_agent() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let scratch = unique_temp_dir("per-agent-cwd");
        let (main_dir, sub_dir) = (scratch.join("main"), scratch.join("sub"));
        std::fs::create_dir_all(&main_dir).unwrap();
        std::fs::create_dir_all(&sub_dir).unwrap();
        let main_file = scratch.join("inst.cwd");
        std::fs::write(&main_file, main_dir.to_string_lossy().as_bytes()).unwrap();
        let _override = EnvVarGuard::set(CWD_STATE_FILE_OVERRIDE_ENV, main_file.to_str().unwrap());

        // A new subagent starts where the main agent is, and writes its own file.
        let first = CwdState::load(Some("sub-1"));
        assert_eq!(first.start_dir.as_deref(), Some(main_dir.as_path()));
        let sub_file = first.path.clone().unwrap();
        assert_ne!(sub_file, main_file);
        assert!(sub_file.to_string_lossy().ends_with("inst.agent-sub-1.cwd"), "{sub_file:?}");

        // It cds (its command's bookkeeping writes its own file).
        std::fs::write(&sub_file, sub_dir.to_string_lossy().as_bytes()).unwrap();
        assert_eq!(CwdState::load(Some("sub-1")).start_dir.as_deref(), Some(sub_dir.as_path()));
        // The main agent, and another subagent, are where they were.
        let main = CwdState::load(None);
        assert_eq!(main.start_dir.as_deref(), Some(main_dir.as_path()));
        assert_eq!(main.path.as_deref(), Some(main_file.as_path()));
        assert_eq!(CwdState::load(Some("sub-2")).start_dir.as_deref(), Some(main_dir.as_path()));
        // An empty agent id is the main agent.
        assert_eq!(CwdState::load(Some("  ")).path.as_deref(), Some(main_file.as_path()));
        drop(_override);
        let _ = std::fs::remove_dir_all(&scratch);
    }

    /// A call whose agent can't be told uses no agent's file, and writes none.
    #[test]
    fn a_call_of_unknown_agent_uses_and_writes_no_state() {
        let state = CwdState::for_call(Some("sub-1"), true);
        assert_eq!(state.path, None);
        assert_eq!(state.start_dir, std::env::current_dir().ok());
    }

    #[test]
    fn subagent_state_files_are_swept_but_an_instances_own_never_is() {
        let scratch = unique_temp_dir("sweep-agent-cwd");
        for name in ["inst.cwd", "inst.agent-a.cwd", "other.agent-b.cwd", "notes.txt"] {
            std::fs::write(scratch.join(name), b"x").unwrap();
        }
        sweep_agent_state_files(&scratch, Duration::from_secs(3600));
        assert!(scratch.join("inst.agent-a.cwd").exists(), "a recent subagent file stays");
        sweep_agent_state_files(&scratch, Duration::ZERO);
        let mut left: Vec<String> =
            std::fs::read_dir(&scratch).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        assert_eq!(left, vec!["inst.cwd", "notes.txt"]);
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn cwd_state_path_honors_explicit_override() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let scratch = unique_temp_dir("override");
        let target = scratch.join("explicit.cwd");
        unsafe {
            std::env::set_var(CWD_STATE_FILE_OVERRIDE_ENV, &target);
        }
        let resolved = cwd_state_path();
        unsafe {
            std::env::remove_var(CWD_STATE_FILE_OVERRIDE_ENV);
        }
        let _ = std::fs::remove_dir_all(&scratch);
        assert_eq!(resolved, Some(target));
    }

    #[test]
    fn cwd_state_path_derives_from_agent_id_when_no_override_or_instance_slug() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _override = EnvVarGuard::unset(CWD_STATE_FILE_OVERRIDE_ENV);
        let _instance = EnvVarGuard::unset("AGENTMUX_INSTANCE_SLUG");
        let _agent = EnvVarGuard::set("AGENTMUX_AGENT_ID", "Test-Agent-42");

        let resolved = cwd_state_path().expect("home dir should resolve in any test environment");
        assert!(resolved.ends_with("Test-Agent-42.cwd"), "{resolved:?}");
        assert!(resolved.to_string_lossy().contains(".agentmux"));
    }

    /// codex P1 on this PR: keying solely by `AGENTMUX_AGENT_ID` (the
    /// definition-wide slug) made two concurrently-running *instances* of
    /// the same agent definition share one cwd file — a `cd` in instance A
    /// would redirect instance B's next command into A's directory, even
    /// though each instance has its own distinct working directory. The
    /// fix: prefer `AGENTMUX_INSTANCE_SLUG` (per-launch, unique per running
    /// instance — see `agent-model.ts`) whenever it's set.
    #[test]
    fn cwd_state_path_prefers_instance_slug_over_agent_id() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _override = EnvVarGuard::unset(CWD_STATE_FILE_OVERRIDE_ENV);
        let _agent = EnvVarGuard::set("AGENTMUX_AGENT_ID", "shared-definition-slug");
        let _instance_a = EnvVarGuard::set("AGENTMUX_INSTANCE_SLUG", "shared-definition-slug-20260101-000000");

        let resolved_a =
            cwd_state_path().expect("home dir should resolve in any test environment");
        assert!(
            resolved_a.ends_with("shared-definition-slug-20260101-000000.cwd"),
            "{resolved_a:?}"
        );

        // A second, concurrently-running instance of the SAME definition
        // (same AGENTMUX_AGENT_ID, different AGENTMUX_INSTANCE_SLUG) must
        // resolve to a DIFFERENT file.
        let _instance_b = EnvVarGuard::set("AGENTMUX_INSTANCE_SLUG", "shared-definition-slug-20260101-000001");
        let resolved_b =
            cwd_state_path().expect("home dir should resolve in any test environment");
        assert_ne!(resolved_a, resolved_b);
        assert!(
            resolved_b.ends_with("shared-definition-slug-20260101-000001.cwd"),
            "{resolved_b:?}"
        );
    }

    /// reagentx re-review finding: when the state dir can't be created,
    /// `CwdState::load()` used to leave `path` as `Some` anyway, so
    /// `restore_cwd` and the child's write would both silently keep failing
    /// against a directory that doesn't exist on every subsequent call,
    /// instead of cleanly disabling persistence once as the doc comment
    /// promises. Forces the failure by pointing the override at a path
    /// whose *parent* is a plain file, not a directory — `create_dir_all`
    /// cannot create a directory there.
    #[test]
    fn cwd_state_load_clears_path_when_state_dir_cannot_be_created() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let scratch = unique_temp_dir("uncreatable-parent");
        let blocking_file = scratch.join("not-a-directory");
        std::fs::write(&blocking_file, b"blocks create_dir_all").unwrap();
        let unreachable_state_file = blocking_file.join("state.cwd");

        let _override = EnvVarGuard::set(
            CWD_STATE_FILE_OVERRIDE_ENV,
            unreachable_state_file.to_str().unwrap(),
        );
        let state = CwdState::load(None);
        let _ = std::fs::remove_dir_all(&scratch);

        assert_eq!(
            state.path, None,
            "path must be cleared (not just logged-as-disabled) when its parent dir can't be created"
        );
        assert!(state.start_dir.is_some(), "must still fall back to a start dir");
    }

    #[test]
    fn restore_cwd_none_when_file_missing() {
        let scratch = unique_temp_dir("missing");
        let missing = scratch.join("does-not-exist.cwd");
        assert_eq!(restore_cwd(&missing), None);
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn restore_cwd_none_when_persisted_dir_no_longer_exists() {
        let scratch = unique_temp_dir("stale");
        let state_file = scratch.join("state.cwd");
        let gone = scratch.join("deleted-dir");
        std::fs::create_dir_all(&gone).unwrap();
        std::fs::write(&state_file, gone.to_string_lossy().as_bytes()).unwrap();
        std::fs::remove_dir_all(&gone).unwrap();
        assert_eq!(restore_cwd(&state_file), None);
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn restore_cwd_some_when_persisted_dir_exists() {
        let scratch = unique_temp_dir("valid");
        let state_file = scratch.join("state.cwd");
        std::fs::write(&state_file, scratch.to_string_lossy().as_bytes()).unwrap();
        assert_eq!(restore_cwd(&state_file), Some(scratch.clone()));
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn restore_cwd_ignores_blank_file() {
        let scratch = unique_temp_dir("blank");
        let state_file = scratch.join("state.cwd");
        std::fs::write(&state_file, b"   \n").unwrap();
        assert_eq!(restore_cwd(&state_file), None);
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn append_cwd_capture_preserves_exit_code_and_gates_on_env() {
        let script = append_cwd_capture("false");
        // The real command's exit code must survive the appended
        // bookkeeping regardless of whether the state env var is set.
        assert!(script.contains("__agentmux_bashwrap_rc=$?"));
        assert!(script.trim_end().ends_with("exit $__agentmux_bashwrap_rc"));
        // The write is gated on the env var being non-empty, and uses a
        // per-PID temp-then-rename so a crash mid-write can't corrupt the
        // file, and two concurrent invocations never write the same temp
        // file (reagentx P2 on this PR).
        assert!(script.contains(&format!("-n \"${CWD_STATE_ENV}\"")));
        assert!(script.contains(&format!("${CWD_STATE_ENV}.tmp.$$")));
        assert!(script.contains("mv -f"));
    }
}
