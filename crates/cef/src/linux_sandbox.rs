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

/// The binary paths the profile names, as AppArmor globs (`*` is any run of characters except `/`, `?` one
/// character except `/`). One list, so the profile text and [`exe_covered_by_profile`] cannot drift apart.
const EXTRACTED_GLOB: &str = "/home/*/.local/share/agentmux/extracted/*/usr/bin/agentmux-cef";
/// The AppImage runtime mounts at `/tmp/.mount_` + the first 6 characters of the AppImage's file name
/// (`AgentM` for `AgentMux_<version>_amd64.AppImage`) + 6 random characters, e.g. `/tmp/.mount_AgentMDCbFgh`.
const FUSE_GLOB: &str = "/tmp/.mount_AgentM??????/usr/bin/agentmux-cef";
const INSTALLED_GLOB: &str = "/opt/agentmux/bin/agentmux-cef";

/// Build the AppArmor profile text granting the `userns` rule to AgentMux's
/// CEF host binary, covering the common case (AppRun's extract-once
/// cache), the FUSE-mount fallback (extraction failed / not yet run), and
/// (as of docs/specs/SPEC_LINUX_DISTRO_TARGETS_AND_DOWNLOADS_PAGE_2026_09_15.md
/// Phase 1) the fixed install path used by the `.deb`/`.rpm` packages.
///
/// The version-segment wildcard (`extracted/*/`) is the load-bearing part —
/// the extract-once cache path embeds the running AgentMux version
/// (`$HOME/.local/share/agentmux/extracted/<VERSION>/...`), which changes on
/// every update. Without the wildcard, this profile would need reinstalling
/// after every single AgentMux upgrade, defeating the entire point of a
/// one-time fix. See spec §1.3.
///
/// **Does NOT cover the portable `.tar.xz`** — that format has no fixed
/// install path by design (extract anywhere), so a static AppArmor profile
/// fundamentally cannot name its binary's location in advance. A user on
/// an AppArmor-restricted system who hits this with the tarball needs to
/// use the AppImage or `.deb`/`.rpm` instead, or fall back to
/// `AGENTMUX_UNSAFE_NOSANDBOX=1` — documented in docs/linux.md rather than
/// silently unhandled.
pub fn build_apparmor_profile() -> String {
    format!(
        "# Installed by AgentMux's one-time sandbox-fix helper.\n\
         # See docs/linux.md \"Sandbox blocked by system policy\".\n\
         abi <abi/4.0>,\n\
         include <tunables/global>\n\
         \n\
         profile agentmux-userns-extracted {extracted} flags=(unconfined) {{\n\
         \x20 userns,\n\
         }}\n\
         \n\
         # FUSE-mount fallback (extract-once cache unavailable — disk full,\n\
         # $HOME unwritable, or first launch before extraction completes).\n\
         # The AppImage runtime names the mount after the first 6 characters\n\
         # of the AppImage's file name, then 6 random ones.\n\
         profile agentmux-userns-fuse {fuse} flags=(unconfined) {{\n\
         \x20 userns,\n\
         }}\n\
         \n\
         # .deb/.rpm install path (fixed, not user-chosen — see\n\
         # scripts/build-deb-linux.sh / build-rpm-linux.sh's /opt/agentmux\n\
         # layout). No version-wildcard needed: package upgrades replace\n\
         # the files at this same fixed path rather than adding a new\n\
         # version-suffixed directory the way the AppImage's extract cache\n\
         # does.\n\
         profile agentmux-userns-installed {installed} flags=(unconfined) {{\n\
         \x20 userns,\n\
         }}\n",
        extracted = EXTRACTED_GLOB,
        fuse = FUSE_GLOB,
        installed = INSTALLED_GLOB,
    )
}

/// Whether AppArmor glob `glob` matches `path` (`*`: any run of characters except `/`; `?`: one such character).
fn glob_matches(glob: &str, path: &str) -> bool {
    fn go(g: &[u8], p: &[u8]) -> bool {
        match g.split_first() {
            None => p.is_empty(),
            Some((b'*', rest)) => {
                let run = p.iter().position(|&c| c == b'/').unwrap_or(p.len());
                (0..=run).any(|n| go(rest, &p[n..]))
            }
            Some((b'?', rest)) => matches!(p.split_first(), Some((&c, tail)) if c != b'/' && go(rest, tail)),
            Some((c, rest)) => matches!(p.split_first(), Some((d, tail)) if d == c && go(rest, tail)),
        }
    }
    go(glob.as_bytes(), path.as_bytes())
}

/// Whether a stanza of [`build_apparmor_profile`] names the binary at `exe` (`current_exe()`, which is the resolved
/// path AppArmor matches). A build anywhere else (a dev `dist/cef-dev` build, the portable `.tar.xz`) is not
/// covered, so installing the profile cannot help it.
pub fn exe_covered_by_profile(exe: &std::path::Path) -> bool {
    exe.to_str().is_some_and(|e| [EXTRACTED_GLOB, FUSE_GLOB, INSTALLED_GLOB].iter().any(|g| glob_matches(g, e)))
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

/// The dialog for a binary no stanza covers: only "Continue without sandbox" and Cancel, because installing the
/// profile cannot help and clicking "Fix it now" would just ask again.
fn uncoverable_dialog_body(exe: &str) -> String {
    format!(
        "Your system's security policy (AppArmor) is blocking the sandbox AgentMux uses to isolate its browser engine.\n\n\
         The one-time fix only covers AgentMux when it runs from the AppImage or a .deb/.rpm install, but this copy is running from:\n\
         {exe}\n\n\
         Install AgentMux that way to get the fix, or continue without the sandbox for this session \
         (AGENTMUX_UNSAFE_NOSANDBOX=1 does the same without asking) — see docs/linux.md."
    )
}

/// zenity's answer to the dialog above: the OK button is "Continue without sandbox this time".
fn parse_continue_only_zenity(exit_success: bool) -> SandboxDialogChoice {
    if exit_success {
        SandboxDialogChoice::ContinueWithoutSandbox
    } else {
        SandboxDialogChoice::Cancel
    }
}

/// kdialog's `--yesno` answer to the same: 0 is "Continue without sandbox this time", anything else cancels.
fn parse_continue_only_kdialog(exit_code: Option<i32>) -> SandboxDialogChoice {
    match exit_code {
        Some(0) => SandboxDialogChoice::ContinueWithoutSandbox,
        _ => SandboxDialogChoice::Cancel,
    }
}

/// Whether this start was made from an agent's environment, so nobody is at the session to answer the dialog.
/// `agent_id` is `AGENTMUX_AGENT_ID`: AgentMux sets it in every agent's shell and a person's terminal never has it,
/// so a `task dev` or an installed app that an agent starts carries it into the host. This is the signal and not
/// "a dev build" (a person's own `task dev` should still get the dialog) or "no terminal" (an installed app started
/// from a launcher has none either, and a person is there to click).
pub fn nobody_to_answer(agent_id: Option<&str>) -> bool {
    agent_id.is_some_and(|v| !v.trim().is_empty())
}

/// What an unattended start prints, on stderr and in the launcher's log, instead of showing the dialog: why it
/// stops, the one explicit way to run on, and what that costs. `exe` is this host binary; `fixable` is whether
/// the one-time profile fix could cover it (an AppImage or a .deb/.rpm install).
pub fn unattended_message(exe: &str, fixable: bool) -> String {
    let keep = if fixable {
        "To keep the sandbox, install the one-time fix from a terminal of your own: start AgentMux there and choose \"Fix it now\" (docs/linux.md)."
    } else {
        "To keep the sandbox, run AgentMux from the AppImage or a .deb/.rpm install after the one-time fix (docs/linux.md); this copy is not covered by it."
    };
    format!(
        "AgentMux: stopping. The browser sandbox is blocked by a system AppArmor policy (common on Ubuntu), and this start \
         came from an agent's environment (AGENTMUX_AGENT_ID is set), so nobody can answer the dialog that would ask what to do.\n\
         This copy is running from: {exe}\n\
         To run anyway WITHOUT the sandbox, start it again with AGENTMUX_UNSAFE_NOSANDBOX=1. The browser engine then runs web \
         pages with no operating-system sandbox, so a flaw in a page could act as you: use it for a check of your own pane layout \
         or of pages you wrote, not for browsing sites you do not trust.\n\
         {keep}"
    )
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

    /// Run `unshare(CLONE_NEWUSER)` followed by a `uid_map` write, and
    /// exit reflecting whether both succeeded — called from the very top
    /// of `lib.rs::run()`, before any other startup work, when this
    /// process was re-exec'd with `INTERNAL_PROBE_USERNS_FLAG`. The
    /// `uid_map` write is load-bearing, not defensive: `unshare()` alone
    /// always succeeds under Ubuntu's AppArmor userns restriction, which
    /// instead denies CAP_SYS_ADMIN inside the new namespace — the
    /// capability `uid_map` (and CEF's own sandbox setup) actually needs.
    /// See the exit-code comment below for the audit-log evidence.
    ///
    /// Safe to call `libc::unshare()` directly in-process here (unlike the
    /// general fork()-in-a-multithreaded-process hazard noted elsewhere in
    /// this module) because a probe-mode process does nothing else at all
    /// — it never spawns the async runtime, CEF, or any additional thread;
    /// this function is the entire lifetime of the process.
    pub fn run_internal_userns_probe_and_exit() -> ! {
        // Must be read BEFORE unshare(): once inside the new namespace,
        // getuid() returns the overflow UID (65534, "nobody") until
        // uid_map is written — the new namespace's mapping starts empty,
        // so the kernel has no UID to report yet (user_namespaces(7)).
        // An unprivileged process may only map its own real (parent-ns)
        // UID into uid_map; writing the overflow UID instead would always
        // be rejected, permanently flipping this probe from "always false
        // positive" to "always false negative" — even after the AppArmor
        // fix is installed, every launch would loop back to the recovery
        // dialog (codex P1 on this same PR).
        let real_uid = unsafe { libc::getuid() };
        // SAFETY: unshare() takes a single integer flag argument and
        // touches no Rust-managed memory; this process has no other
        // threads (see doc comment) so there is no concurrent state for
        // the namespace change to race with.
        let rc = unsafe { libc::unshare(libc::CLONE_NEWUSER) };
        if rc != 0 {
            std::process::exit(1);
        }
        // A bare unshare(CLONE_NEWUSER) succeeds unconditionally under
        // Ubuntu's AppArmor userns restriction — confirmed via the audit
        // log: `apparmor="AUDIT" operation="userns_create" ...
        // profile="unconfined"` (not a denial). The restriction instead
        // transitions the process into a synthetic `unprivileged_userns`
        // profile *inside* the new namespace that denies CAP_SYS_ADMIN
        // (`apparmor="DENIED" ... capability=21 capname="sys_admin"`),
        // which is what writing uid_map/gid_map — and CEF's own
        // subsequent mount/pivot_root calls — needs. Stopping at
        // unshare() alone is a false positive: it reports "available"
        // even when policy blocks the sandbox, so the recovery dialog
        // never fires and CEF's real sandbox init hits Chromium's hard
        // FATAL check instead (zygote_host_impl_linux.cc "No usable
        // sandbox!"). Writing uid_map exercises the exact capability the
        // policy actually gates. gid_map is left untouched: per
        // user_namespaces(7) it additionally requires
        // /proc/self/setgroups=deny first, but uid_map alone already
        // proves whether CAP_SYS_ADMIN is available in the new ns.
        let mapping = format!("0 {} 1\n", real_uid);
        let ok = std::fs::write("/proc/self/uid_map", mapping).is_ok();
        std::process::exit(if ok { 0 } else { 1 });
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

    /// `fixable`: the dialog offers "Fix it now"; otherwise only "Continue without sandbox" and Cancel.
    fn try_zenity(body: &str, fixable: bool) -> Option<SandboxDialogChoice> {
        if !have_on_path("zenity") {
            return None;
        }
        let ok_label = if fixable { FIX_NOW_LABEL } else { CONTINUE_WITHOUT_SANDBOX_LABEL };
        let mut args = vec!["--question", "--title", DIALOG_TITLE, "--text", body, "--ok-label", ok_label, "--cancel-label", "Cancel"];
        if fixable {
            args.extend(["--extra-button", CONTINUE_WITHOUT_SANDBOX_LABEL]);
        }
        let output = Command::new("zenity").args(args).output().ok()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        Some(if fixable {
            parse_zenity_result(output.status.success(), &stdout)
        } else {
            parse_continue_only_zenity(output.status.success())
        })
    }

    fn try_kdialog(body: &str, fixable: bool) -> Option<SandboxDialogChoice> {
        if !have_on_path("kdialog") {
            return None;
        }
        let mut args = vec!["--title", DIALOG_TITLE];
        if fixable {
            args.extend(["--yesnocancel", body, "--yes-label", FIX_NOW_LABEL, "--no-label", CONTINUE_WITHOUT_SANDBOX_LABEL]);
            args.extend(["--cancel-label", "Cancel"]);
        } else {
            args.extend(["--yesno", body, "--yes-label", CONTINUE_WITHOUT_SANDBOX_LABEL, "--no-label", "Cancel"]);
        }
        let status = Command::new("kdialog").args(args).status().ok()?;
        Some(if fixable { parse_kdialog_result(status.code()) } else { parse_continue_only_kdialog(status.code()) })
    }

    /// Show the "sandbox blocked" dialog and return the user's choice.
    /// Tries zenity then kdialog (whichever desktop toolkit is present);
    /// if neither is on PATH (headless / minimal window manager), returns
    /// `NoDialogToolAvailable` — callers must treat this the same as
    /// `Cancel` (never silently proceed unsandboxed without being able to
    /// ask), and should log the equivalent text to stderr so a user
    /// running from a terminal still sees it.
    pub fn show_userns_blocked_dialog() -> SandboxDialogChoice {
        // A binary no stanza names (a dev build, the portable tarball): offering the fix would loop forever.
        let uncovered = std::env::current_exe().ok().filter(|exe| !exe_covered_by_profile(exe));
        let body = uncovered.as_ref().map_or_else(|| DIALOG_BODY.to_string(), |exe| uncoverable_dialog_body(&exe.to_string_lossy()));
        let fixable = uncovered.is_none();
        try_zenity(&body, fixable)
            .or_else(|| try_kdialog(&body, fixable))
            .unwrap_or(SandboxDialogChoice::NoDialogToolAvailable)
    }

    /// The start of an agent has nobody to click the dialog (an agent's `task dev` build for a look check, an
    /// installed app under test): say why and how to go on, and stop with a code the launcher does not retry,
    /// instead of waiting behind the dialog for hours (#610). A person's start returns and gets the dialog.
    pub fn stop_if_nobody_can_answer() {
        if !nobody_to_answer(std::env::var("AGENTMUX_AGENT_ID").ok().as_deref()) {
            return;
        }
        let exe = std::env::current_exe().ok();
        let fixable = exe.as_deref().is_some_and(exe_covered_by_profile);
        let exe = exe.map_or_else(|| "agentmux-cef".to_string(), |e| e.to_string_lossy().into_owned());
        eprintln!("{}", unattended_message(&exe, fixable));
        std::process::exit(agentmux_common::process::HOST_EXIT_SANDBOX_UNAVAILABLE);
    }

    /// No `zenity` or `kdialog` to ask with: print the explanation and exit (never proceed unsandboxed unasked).
    pub fn exit_no_dialog_tool() -> ! {
        eprintln!(
            "AgentMux: the sandbox is blocked by a system AppArmor policy (common on newer Ubuntu), and no dialog tool \
             (zenity/kdialog) is available to ask how to proceed. Set AGENTMUX_UNSAFE_NOSANDBOX=1 to run without the \
             sandbox, or see docs/linux.md for how to install the proper fix."
        );
        std::process::exit(1);
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
    exit_no_dialog_tool, probe_userns_available, resolve_helper_script_path, run_internal_userns_probe_and_exit,
    run_pkexec_fix, show_userns_blocked_dialog, stop_if_nobody_can_answer,
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
        assert!(profile.contains("/tmp/.mount_AgentM??????/usr/bin/agentmux-cef"));
        assert!(!profile.contains(".mount_AgentMu"), "the mount name is the first 6 characters of the file name: AgentM");
    }

    #[test]
    fn profile_covers_the_real_paths() {
        let covered = |p: &str| exe_covered_by_profile(std::path::Path::new(p));
        assert!(covered("/tmp/.mount_AgentMDCbFgh/usr/bin/agentmux-cef"));
        assert!(covered("/home/parlo/.local/share/agentmux/extracted/0.60.20/usr/bin/agentmux-cef"));
        assert!(covered("/opt/agentmux/bin/agentmux-cef"));
    }

    #[test]
    fn profile_does_not_cover_other_builds() {
        let covered = |p: &str| exe_covered_by_profile(std::path::Path::new(p));
        // A dev build, which looped the dialog on charlie.
        assert!(!covered("/home/parlo/.agentmux/agents/parlo/agentmux/dist/cef-dev/agentmux-cef"));
        // The portable tarball, anywhere.
        assert!(!covered("/home/parlo/Downloads/AgentMux/usr/bin/agentmux-cef"));
        // A home that is not /home/<user>.
        assert!(!covered("/root/.local/share/agentmux/extracted/0.60.20/usr/bin/agentmux-cef"));
        // The mount name: 6 random characters after AgentM, and no `/` stands in for any of them.
        assert!(!covered("/tmp/.mount_AgentM/usr/bin/agentmux-cef"));
        assert!(!covered("/tmp/.mount_AgentMDCbFghX/usr/bin/agentmux-cef"));
        assert!(!covered("/tmp/.mount_AgentM/DCbFgh/usr/bin/agentmux-cef"));
        assert!(!covered("/tmp/.mount_Other00DCbFgh/usr/bin/agentmux-cef"));
        // A `*` does not cross a directory.
        assert!(!covered("/home/a/b/.local/share/agentmux/extracted/0.60.20/usr/bin/agentmux-cef"));
        assert!(!covered("/home/a/.local/share/agentmux/extracted/0.60.20/x/usr/bin/agentmux-cef"));
    }

    #[test]
    fn glob_star_and_question_mark() {
        assert!(glob_matches("/a/*/c", "/a/bb/c"));
        assert!(glob_matches("/a/*/c", "/a//c"));
        assert!(!glob_matches("/a/*/c", "/a/b/b/c"));
        assert!(glob_matches("/a/b?d", "/a/bcd"));
        assert!(!glob_matches("/a/b?d", "/a/b/d"));
        assert!(!glob_matches("/a/b?d", "/a/bd"));
    }

    #[test]
    fn uncoverable_dialog_names_the_binary_and_offers_no_fix() {
        let body = uncoverable_dialog_body("/home/p/agentmux/dist/cef-dev/agentmux-cef");
        assert!(body.contains("/home/p/agentmux/dist/cef-dev/agentmux-cef"));
        assert!(!body.contains(FIX_NOW_LABEL));
    }

    #[test]
    fn uncoverable_dialog_answers_are_continue_or_cancel_never_fix() {
        assert_eq!(parse_continue_only_zenity(true), SandboxDialogChoice::ContinueWithoutSandbox);
        assert_eq!(parse_continue_only_zenity(false), SandboxDialogChoice::Cancel);
        assert_eq!(parse_continue_only_kdialog(Some(0)), SandboxDialogChoice::ContinueWithoutSandbox);
        assert_eq!(parse_continue_only_kdialog(Some(1)), SandboxDialogChoice::Cancel);
        assert_eq!(parse_continue_only_kdialog(None), SandboxDialogChoice::Cancel);
    }

    #[test]
    fn apparmor_profile_covers_deb_rpm_install_path() {
        // docs/specs/SPEC_LINUX_DISTRO_TARGETS_AND_DOWNLOADS_PAGE_2026_09_15.md
        // Phase 1 — the .deb/.rpm install path (scripts/build-deb-linux.sh,
        // build-rpm-linux.sh). No version wildcard: package upgrades
        // replace files at this same fixed path.
        let profile = build_apparmor_profile();
        assert!(profile.contains("/opt/agentmux/bin/agentmux-cef"));
    }

    #[test]
    fn apparmor_profile_grants_userns_in_all_stanzas() {
        let profile = build_apparmor_profile();
        assert_eq!(profile.matches("userns,").count(), 3);
    }

    #[test]
    fn apparmor_profile_is_valid_utf8_and_nonempty() {
        let profile = build_apparmor_profile();
        assert!(!profile.is_empty());
    }

    #[test]
    fn only_an_agent_environment_means_nobody_can_answer() {
        assert!(nobody_to_answer(Some("opaz")));
        assert!(!nobody_to_answer(None));
        assert!(!nobody_to_answer(Some("")));
        assert!(!nobody_to_answer(Some("  ")));
    }

    #[test]
    fn unattended_message_names_the_way_on_the_cost_and_the_binary() {
        for fixable in [false, true] {
            let m = unattended_message("/work/dist/cef-dev/agentmux-cef", fixable);
            assert!(m.contains("AGENTMUX_UNSAFE_NOSANDBOX=1"), "{m}");
            assert!(m.contains("WITHOUT the sandbox"), "{m}");
            assert!(m.contains("/work/dist/cef-dev/agentmux-cef"), "{m}");
            assert!(m.contains("docs/linux.md"), "{m}");
        }
        assert!(unattended_message("x", true).contains("Fix it now"));
        assert!(!unattended_message("x", false).contains("Fix it now"));
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
