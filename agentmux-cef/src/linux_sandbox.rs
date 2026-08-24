//! Linux-only: detect and recover from Ubuntu's AppArmor restriction on
//! unprivileged user-namespace creation, which blocks the kernel-namespace
//! sandbox CEF relies on when `--disable-setuid-sandbox` is set (see
//! `app/mod.rs`'s Linux sandbox branch). Originally landed in Ubuntu 23.10,
//! later security-patched into 22.04/20.04 LTS too — not an AgentMux
//! regression, but AgentMux still owns making the failure visible and
//! recoverable instead of a silent no-op window.
//!
//! See docs/specs/SPEC_LINUX_SANDBOX_APPARMOR_USERNS_2026_08_23.md for the
//! full design, including why this uses an AppArmor profile exception
//! rather than reviving the classic SUID `chrome-sandbox` binary (the
//! extract-once-cache's version-scoped paths mean a permissions-based fix
//! wouldn't survive an AgentMux update; a glob-scoped AppArmor profile
//! does).
//!
//! Split deliberately into OS-agnostic pure functions (profile text
//! generation, dialog-choice parsing) that are unit-tested on any host,
//! and `#[cfg(target_os = "linux")]`-gated functions that actually touch
//! the OS (spawn `unshare`, `pkexec`, `zenity`/`kdialog`) which can only be
//! exercised on real Linux.
//!
//! The pure items below are only ever called from `imp` (Linux-only) or
//! `#[cfg(test)]`, so a non-Linux build sees them as genuinely unused —
//! silence that expected noise without weakening dead_code detection on
//! the platform where it matters.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

/// Where the one-time fix installs the AppArmor profile.
pub const APPARMOR_PROFILE_INSTALL_PATH: &str = "/etc/apparmor.d/agentmux-userns";

/// Build the AppArmor profile text granting the `userns` rule to AgentMux's
/// CEF host binary, covering both the common case (AppRun's extract-once
/// cache) and the FUSE-mount fallback (extraction failed / not yet run).
///
/// The version-segment wildcard (`extracted/*/`) is the load-bearing part —
/// the extract-once cache path embeds the running AgentMux version
/// (`$HOME/.local/share/agentmux/extracted/<VERSION>/...`), which changes on
/// every update. Without the wildcard, this profile would need reinstalling
/// after every single AgentMux upgrade, defeating the entire point of a
/// one-time fix. See spec §1.3.
pub fn build_apparmor_profile() -> String {
    format!(
        "# Installed by AgentMux's one-time sandbox-fix helper.\n\
         # See docs/linux.md \"Sandbox blocked by system policy\".\n\
         abi <abi/4.0>,\n\
         include <tunables/global>\n\
         \n\
         profile agentmux-userns-extracted /home/*/.local/share/agentmux/extracted/*/usr/bin/agentmux-cef flags=(unconfined) {{\n\
         \x20 userns,\n\
         }}\n\
         \n\
         # FUSE-mount fallback (extract-once cache unavailable — disk full,\n\
         # $HOME unwritable, or first launch before extraction completes).\n\
         # appimagetool's default FUSE mountpoint naming; see spec OQ3 —\n\
         # verify this matches the pinned appimagetool version's actual\n\
         # naming before relying on this stanza alone.\n\
         profile agentmux-userns-fuse /tmp/.mount_AgentMu*/usr/bin/agentmux-cef flags=(unconfined) {{\n\
         \x20 userns,\n\
         }}\n"
    )
}

/// Outcome of the "sandbox blocked" dialog (§5.3 of the spec).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxDialogChoice {
    /// Run the pkexec-elevated one-time AppArmor fix.
    FixNow,
    /// Proceed with `AGENTMUX_UNSAFE_NOSANDBOX=1` for this session only.
    ContinueWithoutSandbox,
    /// Exit without launching.
    Cancel,
    /// No dialog tool (zenity/kdialog) was available to ask at all.
    /// Callers must NOT interpret this as consent — see doc comment on
    /// `show_userns_blocked_dialog`.
    NoDialogToolAvailable,
}

/// Parse zenity's `--question --extra-button="Continue without sandbox"`
/// result. zenity exits 0 for the OK button ("Fix it now" here, via
/// `--ok-label`), and exits 1 for BOTH Cancel and the extra button — the
/// only way to tell them apart is that the extra button also prints its
/// own label text to stdout before exiting, while Cancel prints nothing.
fn parse_zenity_result(exit_success: bool, stdout: &str) -> SandboxDialogChoice {
    if exit_success {
        SandboxDialogChoice::FixNow
    } else if stdout.trim() == CONTINUE_WITHOUT_SANDBOX_LABEL {
        SandboxDialogChoice::ContinueWithoutSandbox
    } else {
        SandboxDialogChoice::Cancel
    }
}

/// Parse kdialog's `--yesnocancel` exit code. Unlike zenity, kdialog gives
/// three distinct exit codes natively (0=Yes, 1=No, 2=Cancel) — no stdout
/// parsing needed.
fn parse_kdialog_result(exit_code: Option<i32>) -> SandboxDialogChoice {
    match exit_code {
        Some(0) => SandboxDialogChoice::FixNow,
        Some(1) => SandboxDialogChoice::ContinueWithoutSandbox,
        _ => SandboxDialogChoice::Cancel,
    }
}

const CONTINUE_WITHOUT_SANDBOX_LABEL: &str = "Continue without sandbox this time";
const FIX_NOW_LABEL: &str = "Fix it now (one-time, needs your password)";
const DIALOG_TITLE: &str = "AgentMux — sandbox blocked by system policy";
const DIALOG_BODY: &str = "Your system's security policy (AppArmor) is blocking the sandbox \
AgentMux uses to isolate its browser engine. This is a known Ubuntu policy change \
(unprivileged user namespaces restricted, ~2024+), not an AgentMux bug — see docs/linux.md.\n\n\
Install a one-time fix (needs your password) so future launches load with the sandbox enabled?";

/// CLI flag recognized as an internal, undocumented self-probe mode — see
/// `run_internal_userns_probe_and_exit()`'s doc comment for why this exists
/// and why it must be the exact same binary path the AppArmor profile
/// confines (reagent P0: probing via the generic `unshare` utility tests a
/// completely different, unconfined security context and would report
/// "blocked" forever regardless of whether the fix actually worked).
pub const INTERNAL_PROBE_USERNS_FLAG: &str = "--internal-probe-userns";

#[cfg(target_os = "linux")]
mod imp {
    use super::*;
    use std::process::Command;

    /// Run the actual `unshare(CLONE_NEWUSER)` syscall and exit — called
    /// from the very top of `lib.rs::run()`, before any other startup work,
    /// when this process was re-exec'd with `INTERNAL_PROBE_USERNS_FLAG`.
    ///
    /// Safe to call `libc::unshare()` directly in-process here (unlike the
    /// general fork()-in-a-multithreaded-process hazard noted elsewhere in
    /// this module) because a probe-mode process does nothing else at all
    /// — it never spawns the async runtime, CEF, or any additional thread;
    /// this function is the entire lifetime of the process.
    pub fn run_internal_userns_probe_and_exit() -> ! {
        // SAFETY: unshare() takes a single integer flag argument and
        // touches no Rust-managed memory; this process has no other
        // threads (see doc comment) so there is no concurrent state for
        // the namespace change to race with.
        let rc = unsafe { libc::unshare(libc::CLONE_NEWUSER) };
        std::process::exit(if rc == 0 { 0 } else { 1 });
    }

    /// Deterministically test whether unprivileged user-namespace creation
    /// works **for AgentMux's own confined binary path** — the exact
    /// condition CEF's namespace sandbox needs. This is the primary
    /// detection path (spec §4): it answers "will the sandbox work"
    /// directly, without needing to know *why* it might not (AppArmor
    /// policy, a container's own restrictions, an old kernel) or guessing
    /// at CEF's internal failure signature.
    ///
    /// Re-execs the CURRENT binary (`current_exe()`) with
    /// `INTERNAL_PROBE_USERNS_FLAG` rather than spawning the generic
    /// `unshare` utility: `build_apparmor_profile()` grants the `userns`
    /// rule to a path-matched profile scoped to AgentMux's own binary path
    /// specifically — `/usr/bin/unshare` is a different binary with no such
    /// grant, so probing it would test an entirely different, permanently
    /// unconfined security context and report "blocked" identically before
    /// and after the fix, on every launch, forever (reagent P0 on
    /// PR #2783). Re-execing `current_exe()` ensures the probe subprocess
    /// is matched by the exact same AppArmor profile rule CEF's own
    /// renderer/GPU subprocess spawns will be.
    ///
    /// Spawning a subprocess (rather than calling `libc::unshare()`
    /// directly in THIS process) sidesteps the fork()-in-a-multithreaded-
    /// process hazard: by the time this runs, this process is very likely
    /// multithreaded (async runtime, CEF's own internal threads), and
    /// `fork()` only duplicates the calling thread — other threads' lock
    /// state is undefined in the child. The re-exec'd child, in contrast,
    /// is a fresh, single-purpose, single-threaded process (see
    /// `run_internal_userns_probe_and_exit()`), so calling `unshare()`
    /// directly there is sound.
    pub fn probe_userns_available() -> bool {
        let exe = match std::env::current_exe() {
            Ok(p) => p,
            Err(_) => return true, // can't even locate ourselves; don't block startup on this
        };
        match Command::new(exe).arg(INTERNAL_PROBE_USERNS_FLAG).status() {
            Ok(status) => status.success(),
            Err(_) => true,
        }
    }

    fn have_on_path(bin: &str) -> bool {
        Command::new("which")
            .arg(bin)
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    fn try_zenity() -> Option<SandboxDialogChoice> {
        if !have_on_path("zenity") {
            return None;
        }
        let output = Command::new("zenity")
            .args([
                "--question",
                "--title",
                DIALOG_TITLE,
                "--text",
                DIALOG_BODY,
                "--ok-label",
                FIX_NOW_LABEL,
                "--cancel-label",
                "Cancel",
                "--extra-button",
                CONTINUE_WITHOUT_SANDBOX_LABEL,
            ])
            .output()
            .ok()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        Some(parse_zenity_result(output.status.success(), &stdout))
    }

    fn try_kdialog() -> Option<SandboxDialogChoice> {
        if !have_on_path("kdialog") {
            return None;
        }
        let status = Command::new("kdialog")
            .args([
                "--title",
                DIALOG_TITLE,
                "--yesnocancel",
                DIALOG_BODY,
                "--yes-label",
                FIX_NOW_LABEL,
                "--no-label",
                CONTINUE_WITHOUT_SANDBOX_LABEL,
                "--cancel-label",
                "Cancel",
            ])
            .status()
            .ok()?;
        Some(parse_kdialog_result(status.code()))
    }

    /// Show the "sandbox blocked" dialog and return the user's choice.
    /// Tries zenity then kdialog (whichever desktop toolkit is present);
    /// if neither is on PATH (headless / minimal window manager), returns
    /// `NoDialogToolAvailable` — callers must treat this the same as
    /// `Cancel` (never silently proceed unsandboxed without being able to
    /// ask), and should log the equivalent text to stderr so a user
    /// running from a terminal still sees it.
    pub fn show_userns_blocked_dialog() -> SandboxDialogChoice {
        try_zenity()
            .or_else(try_kdialog)
            .unwrap_or(SandboxDialogChoice::NoDialogToolAvailable)
    }

    /// Resolve the absolute path to the bundled helper script
    /// (`install-userns-apparmor-fix.sh`, copied to the AppDir root
    /// alongside `install-linux-desktop.sh` by `build-appimage-linux.sh` —
    /// same top-level convention, not a subdirectory). Prefers `$APPDIR`
    /// (set by `AppRun` before exec'ing the launcher, inherited down to
    /// this process — see lib.rs); falls back to a path relative to the
    /// current binary for `task dev`/non-AppImage runs where `APPDIR`
    /// isn't set.
    pub fn resolve_helper_script_path() -> std::path::PathBuf {
        if let Ok(appdir) = std::env::var("APPDIR") {
            return std::path::PathBuf::from(appdir).join("install-userns-apparmor-fix.sh");
        }
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .unwrap_or_default()
            .join("install-userns-apparmor-fix.sh")
    }

    /// Create the temp file holding the generated AppArmor profile that
    /// `run_pkexec_fix` hands to the privileged helper, using a random
    /// (not predictable), owner-only-readable, exclusively-created path.
    ///
    /// A fixed, predictable filename in the shared world-writable temp
    /// directory (e.g. `agentmux-userns.profile`) is a real local
    /// privilege-escalation vector (reagent P1 on PR #2783, missed in the
    /// first fix pass): a local attacker who can predict the path can
    /// pre-position a symlink or file there and win the race between this
    /// unprivileged write and pkexec's privileged read, causing root to
    /// install attacker-controlled content at
    /// `/etc/apparmor.d/agentmux-userns`. Three independent mitigations,
    /// not just one:
    ///   1. Random suffix (kernel CSPRNG via `/dev/urandom`, falling back
    ///      to PID+nanosecond-timestamp only if that's somehow
    ///      unavailable) — an attacker can't pre-position anything at a
    ///      path they can't predict in advance.
    ///   2. `O_CREAT | O_EXCL` (`create_new(true)`) — atomically fails if
    ///      the path already exists (e.g. a pre-positioned symlink),
    ///      rather than following it.
    ///   3. Mode `0o600` (owner read/write only) at creation time, so even
    ///      within the narrow window before pkexec reads it, no other
    ///      unprivileged user on the system can read or substitute it.
    fn create_secure_temp_profile(contents: &str) -> Result<std::path::PathBuf, String> {
        use std::io::{Read, Write};
        use std::os::unix::fs::OpenOptionsExt;

        let mut random_bytes = [0u8; 16];
        if std::fs::File::open("/dev/urandom")
            .and_then(|mut f| f.read_exact(&mut random_bytes))
            .is_err()
        {
            // /dev/urandom is present on every real Linux system; this
            // fallback only guards against something like a minimal
            // container image missing it. Weaker (PID + timestamp are far
            // less entropy than a CSPRNG), but combined with O_EXCL + 0600
            // below, still closes the "attacker pre-positions a file at a
            // KNOWN path" attack this exists to prevent — it's the
            // *predictability* that's the vulnerability, and neither
            // source is predictable to an attacker in advance.
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos())
                .unwrap_or(0);
            let pid = std::process::id();
            random_bytes[..4].copy_from_slice(&pid.to_ne_bytes());
            random_bytes[4..8].copy_from_slice(&nanos.to_ne_bytes());
        }
        let suffix: String = random_bytes.iter().map(|b| format!("{b:02x}")).collect();
        let profile_path =
            std::env::temp_dir().join(format!("agentmux-userns-{suffix}.profile"));

        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true) // O_EXCL — fail rather than follow a pre-existing path/symlink
            .mode(0o600) // owner-only
            .open(&profile_path)
            .map_err(|e| format!("failed to create temp profile: {e}"))?;
        file.write_all(contents.as_bytes())
            .map_err(|e| format!("failed to write temp profile: {e}"))?;

        Ok(profile_path)
    }

    /// Run the pkexec-elevated one-time fix: writes `build_apparmor_profile()`'s
    /// output to a securely-created temp file (`create_secure_temp_profile`),
    /// then invokes the bundled helper script (resolved via
    /// `resolve_helper_script_path()`) via `pkexec` to copy it into place
    /// and reload AppArmor. The profile TEXT lives only in
    /// `build_apparmor_profile()` (tested, §see module docs) — the
    /// privileged helper script only does the mechanical copy+reload, so
    /// there's a single source of truth for what the profile actually
    /// grants rather than the same text duplicated in a bundled template
    /// that could drift from what Rust generates.
    ///
    /// Returns Ok(()) only if the helper itself reported success; a
    /// missing `pkexec` (minimal/non-graphical setups) or a user
    /// cancelling the polkit password prompt both surface as Err with a
    /// message suitable for direct display.
    pub fn run_pkexec_fix(helper_script: &std::path::Path) -> Result<(), String> {
        let profile_path = create_secure_temp_profile(&build_apparmor_profile())?;

        if !have_on_path("pkexec") {
            return Err(format!(
                "pkexec not found. Run this manually in a terminal:\n    sudo bash {} {}",
                helper_script.display(),
                profile_path.display()
            ));
        }

        let output = Command::new("pkexec")
            .arg(helper_script)
            .arg(&profile_path)
            .output()
            .map_err(|e| format!("failed to launch pkexec: {e}"))?;

        // Best-effort cleanup — leaving a stray temp file behind isn't
        // worth failing the whole operation over.
        let _ = std::fs::remove_file(&profile_path);

        if output.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(format!(
                "sandbox fix failed (exit {:?}) trying to install {}: {}",
                output.status.code(),
                APPARMOR_PROFILE_INSTALL_PATH,
                stderr.trim()
            ))
        }
    }
}

#[cfg(target_os = "linux")]
pub use imp::{
    probe_userns_available, resolve_helper_script_path, run_internal_userns_probe_and_exit,
    run_pkexec_fix, show_userns_blocked_dialog,
};

/// Set (by lib.rs, once) when the current session is running with the
/// sandbox explicitly disabled after the user chose "Continue without
/// sandbox this time" — read by `client::display::on_title_change` to keep
/// this security downgrade visible in the window title for the rest of the
/// session (reagent/codex P2 on PR #2783: §5.3 of the spec requires a
/// persistent, not-easily-missed indicator, not just a log line). A plain
/// `AtomicBool` rather than a full flash-notification/banner plumb-through
/// — the window title is the one UI surface the host can update directly
/// without new frontend/IPC work, and is visible for the entire session by
/// construction (can't be dismissed/scrolled away like a toast could).
pub static RUNNING_UNSANDBOXED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apparmor_profile_covers_extracted_cache_path_with_version_wildcard() {
        let profile = build_apparmor_profile();
        assert!(profile.contains("/home/*/.local/share/agentmux/extracted/*/usr/bin/agentmux-cef"));
    }

    #[test]
    fn apparmor_profile_covers_fuse_mount_fallback() {
        let profile = build_apparmor_profile();
        assert!(profile.contains("/tmp/.mount_AgentMu*/usr/bin/agentmux-cef"));
    }

    #[test]
    fn apparmor_profile_grants_userns_in_both_stanzas() {
        let profile = build_apparmor_profile();
        assert_eq!(profile.matches("userns,").count(), 2);
    }

    #[test]
    fn apparmor_profile_is_valid_utf8_and_nonempty() {
        let profile = build_apparmor_profile();
        assert!(!profile.is_empty());
    }

    #[test]
    fn zenity_ok_button_means_fix_now() {
        assert_eq!(parse_zenity_result(true, ""), SandboxDialogChoice::FixNow);
    }

    #[test]
    fn zenity_extra_button_label_on_stdout_means_continue_without_sandbox() {
        assert_eq!(
            parse_zenity_result(false, CONTINUE_WITHOUT_SANDBOX_LABEL),
            SandboxDialogChoice::ContinueWithoutSandbox
        );
    }

    #[test]
    fn zenity_extra_button_label_with_trailing_newline_still_matches() {
        // zenity's stdout typically includes a trailing newline after the label.
        let stdout = format!("{CONTINUE_WITHOUT_SANDBOX_LABEL}\n");
        assert_eq!(
            parse_zenity_result(false, &stdout),
            SandboxDialogChoice::ContinueWithoutSandbox
        );
    }

    #[test]
    fn zenity_empty_stdout_on_failure_means_cancel() {
        assert_eq!(parse_zenity_result(false, ""), SandboxDialogChoice::Cancel);
    }

    #[test]
    fn kdialog_exit_0_means_fix_now() {
        assert_eq!(parse_kdialog_result(Some(0)), SandboxDialogChoice::FixNow);
    }

    #[test]
    fn kdialog_exit_1_means_continue_without_sandbox() {
        assert_eq!(
            parse_kdialog_result(Some(1)),
            SandboxDialogChoice::ContinueWithoutSandbox
        );
    }

    #[test]
    fn kdialog_exit_2_means_cancel() {
        assert_eq!(parse_kdialog_result(Some(2)), SandboxDialogChoice::Cancel);
    }

    #[test]
    fn kdialog_missing_exit_code_defaults_to_cancel_not_fix_now() {
        // A process killed by a signal (no exit code) must never be
        // interpreted as consent to elevate privileges.
        assert_eq!(parse_kdialog_result(None), SandboxDialogChoice::Cancel);
    }

    #[test]
    fn kdialog_unexpected_exit_code_defaults_to_cancel() {
        assert_eq!(parse_kdialog_result(Some(99)), SandboxDialogChoice::Cancel);
    }
}
