// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The launcher's half of upgrading a `.deb` or `.rpm` install from the app
//! (`docs/specs/SPEC_LINUX_PACKAGE_UPGRADE_AND_REPOSITORY_2026_10_24.md` §3.1,
//! phase 3g-1).
//!
//! Installing a package takes root, and AgentMux never runs as root. So instead
//! of the usual "quit, then run the installer", the user is asked for permission
//! first, while AgentMux is still open: `pkexec` starts the package's own helper
//! (`/opt/agentmux/bin/agentmux-install-update`, `scripts/agentmux-install-update.sh`),
//! the desktop shows its password prompt, and the helper checks the file itself
//! and prints `READY`. Only then does the launcher quit; the helper waits for it
//! to be gone, installs, and a small waiter, started before any of that and kept
//! running after the launcher exits, starts AgentMux again whether the install
//! worked or not, so the old version runs rather than none.
//!
//! If the password is refused, can't be asked for, or the file fails its checks,
//! nothing quits: the reason is left for srv (`INSTALL_RESULT_FILE`) and the
//! panel says it.

use std::path::{Path, PathBuf};

/// Where a package puts AgentMux's programs.
pub const INSTALL_BIN: &str = "/opt/agentmux/bin";
/// The helper's name in [`INSTALL_BIN`].
pub const HELPER_NAME: &str = "agentmux-install-update";
/// The command that starts the installed AgentMux (the package's wrapper).
pub const START_COMMAND: &str = "/usr/bin/agentmux";

/// The helper's record of the last upgrade it did (`from=` and `to=` lines), readable
/// by anyone: what "go back" is offered from.
pub const STATE_DIR: &str = "/var/lib/agentmux";
/// How long after an upgrade going back is still offered: after that, what the new
/// version did to its data is too much to leave behind.
const GO_BACK_WITHIN: std::time::Duration = std::time::Duration::from_secs(7 * 24 * 60 * 60);

/// How long going back waits for the helper's `READY`: it prints that only once it has downloaded
/// the old release (its download may take up to 900 s) and checked it, and the user's password
/// prompt comes before all of that. Stopping the wait early would leave the helper without the waiter
/// that starts AgentMux again.
const GO_BACK_AUTHORIZE_WITHIN: std::time::Duration = std::time::Duration::from_secs(900 + 300);

/// What to hand the helper in place of a file when it is to install the version
/// the last upgrade replaced.
const GO_BACK_ARG: &str = "--go-back";

/// A package that the user's password can install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageUpgrade {
    /// The release's `.deb` or `.rpm` in `updates/<version>/` of the data folder;
    /// for going back, [`GO_BACK_ARG`] (the helper downloads the package itself).
    pub installer: PathBuf,
    pub version: String,
    /// This version's data folder: where the result is left for srv.
    pub data_dir: PathBuf,
}

/// The release's file for `version`, by package kind.
pub fn package_file_name(version: &str, deb: bool) -> String {
    if deb {
        format!("AgentMux_{version}_amd64.deb")
    } else {
        format!("AgentMux-{version}-1.x86_64.rpm")
    }
}

/// Check what srv names to install against this install: `installer` is the release's
/// package for `version`, as downloaded into this version's `updates/<version>/`
/// and whole by `sha256`, and this launcher is the installed one (`own_dir` is
/// `bin_dir`) with the install helper beside it.
pub fn check(installer: &Path, sha256: &str, version: &str, own_dir: &Path, data_dir: &Path, bin_dir: &Path) -> Result<PackageUpgrade, String> {
    if own_dir != bin_dir || !bin_dir.join(HELPER_NAME).is_file() {
        return Err("this AgentMux isn't a .deb or .rpm install that can install updates itself".into());
    }
    let named_right = installer
        .file_name()
        .is_some_and(|n| [true, false].iter().any(|deb| n.to_string_lossy() == package_file_name(version, *deb)));
    let in_updates = installer.parent() == Some(data_dir.join("updates").join(version).as_path());
    if !installer.is_absolute() || !named_right || !in_updates || !installer.is_file() {
        return Err(format!("{} isn't the AgentMux {version} package AgentMux downloaded", installer.display()));
    }
    if !crate::upgrade_restart::is_sha256_hex(sha256) || crate::upgrade_restart::file_sha256(installer).as_deref() != Some(sha256) {
        return Err(format!("{} isn't the AgentMux {version} release; download it again", installer.display()));
    }
    Ok(PackageUpgrade { installer: installer.to_path_buf(), version: version.to_string(), data_dir: data_dir.to_path_buf() })
}

/// Whether the last upgrade the helper did replaced `version` with `own_version` (the
/// running one) not long before `now`: then going back to it is on offer.
/// `record` is the helper's `last-upgrade` file: `from=` and `to=` lines.
pub fn go_back_version(record: &Path, own_version: &str, now: std::time::SystemTime) -> Option<String> {
    let age = now.duration_since(std::fs::metadata(record).and_then(|m| m.modified()).ok()?).unwrap_or_default();
    let text = std::fs::read_to_string(record).ok()?;
    let field = |key: &str| text.lines().find_map(|l| l.strip_prefix(key)).map(|v| v.trim().to_string());
    let (from, to) = (field("from=")?, field("to=")?);
    let plain = |v: &str| !v.is_empty() && v.bytes().all(|b| b.is_ascii_alphanumeric() || b"+~.-".contains(&b));
    (to == own_version && plain(&from) && plain(&to) && from != to && age < GO_BACK_WITHIN).then_some(from)
}

impl PackageUpgrade {
    /// Going back to `version`: the helper fetches and checks the package itself.
    pub fn go_back(version: &str, data_dir: &Path) -> PackageUpgrade {
        let up = PackageUpgrade { installer: PathBuf::from(GO_BACK_ARG), version: version.to_string(), data_dir: data_dir.to_path_buf() };
        let _ = std::fs::create_dir_all(up.files().0.parent().unwrap_or(data_dir));
        up
    }

    fn is_go_back(&self) -> bool {
        self.installer.as_os_str() == GO_BACK_ARG
    }

    fn deb(&self) -> bool {
        self.installer.extension().is_some_and(|e| e == "deb")
    }

    /// Where the waiter's output goes, and where the result is left.
    fn files(&self) -> (PathBuf, PathBuf, PathBuf) {
        let dir = self.data_dir.join("updates").join(&self.version);
        (dir.join("install.out"), dir.join("install.err"), dir.join(agentmux_common::srv_stderr::INSTALL_RESULT_FILE))
    }

    /// The command that installs this file by hand: what the panel offers when no
    /// password prompt can be shown.
    pub fn manual_command(&self) -> String {
        if self.is_go_back() {
            return format!("download AgentMux {} from https://github.com/agentmuxai/agentmux/releases and install it with your package manager", self.version);
        }
        format!("sudo {} {}", if self.deb() { "apt install" } else { "dnf install" }, self.installer.display())
    }
}

/// The reason an earlier try left (see [`leave_result`]).
fn read_result(up: &PackageUpgrade) -> Option<String> {
    let text = std::fs::read_to_string(up.files().2).ok()?;
    serde_json::from_str::<serde_json::Value>(&text).ok()?.get("error")?.as_str().map(String::from)
}

/// Leave `error` for srv, which shows it in the version panel.
fn leave_result(up: &PackageUpgrade, error: &str) {
    let (_, _, result) = up.files();
    let json = serde_json::json!({ "version": up.version, "error": error });
    if let Err(e) = std::fs::write(&result, format!("{json}\n")) {
        crate::log(&format!("upgrade: couldn't leave {} for srv: {e}", result.display()));
    }
}

/// What the waiter runs: `pkexec <helper> <file> <version> <launcher pid>`, with the
/// helper's output in files the launcher and the waiter read (the files are opened
/// here, as the user, so root never needs a path in the user's folders).
/// Arguments: pkexec, helper, file, version, launcher pid, out, err, result, the
/// command that starts AgentMux.
///
/// Before the helper printed `READY` (password refused or unavailable, or the file
/// failed a check) AgentMux is still running: the exit status tells the launcher
/// what happened, and nothing else is done. After it, the waiter outlives the
/// launcher: it leaves why the install failed (a success leaves nothing), waits for
/// the launcher to be gone and starts AgentMux again, the old version or the new.
const WAITER: &str = r#"pkexec="$1"; helper="$2"; file="$3"; ver="$4"; lpid="$5"; out="$6"; err="$7"; result="$8"; start="$9"
"$pkexec" "$helper" "$file" "$ver" "$lpid" >"$out" 2>"$err" </dev/null
rc=$?
grep -qx READY "$out" 2>/dev/null || exit "$rc"
if [ "$rc" -eq 0 ]; then
    rm -f "$result"
else
    reason=$(sed -n 's/^agentmux-install-update: //p' "$err" | tail -n 1 | tr -d '"\\' | cut -c1-300)
    [ -n "$reason" ] || reason="the installer stopped with status $rc"
    printf '{"version":"%s","error":"%s"}\n' "$ver" "$reason" >"$result"
fi
# The launcher is on its way out (it was told to quit when the helper said READY):
# start AgentMux once it has gone, whether or not the install worked.
n=0
while kill -0 "$lpid" 2>/dev/null; do
    [ "$n" -lt 180 ] || exit "$rc"
    sleep 1
    n=$((n + 1))
done
exec "$start"
"#;

/// pkexec exits with this when the user dismissed the password prompt, and with
/// [`NOT_AUTHORIZED`] when it couldn't authorise the command (no prompt to show, or
/// not allowed).
const DISMISSED: i32 = 126;
const NOT_AUTHORIZED: i32 = 127;

/// The waiter's process group while the password prompt is open (0 when none).
static WAITER_GROUP: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// What the dialog asks before the password is: the user agrees to quit, then
/// the desktop asks for the password.
pub fn question(up: &PackageUpgrade, stopping_paragraph: &str) -> String {
    format!(
        "Restart AgentMux to upgrade to {}?\n\n\
         AgentMux asks your computer for permission to install the new {} package (you may be asked for your password), \
         then every window closes and every agent stops, the new version is installed and AgentMux opens again.\n\n\
         Your tabs come back and agents resume their conversations.{stopping_paragraph}",
        up.version,
        if up.deb() { ".deb" } else { ".rpm" },
    )
}

/// What the dialog asks when the new version could not be kept running.
pub fn go_back_question(from: &str, to: &str, why: &str) -> String {
    format!(
        "AgentMux {to} {why}.\n\n\
         Go back to AgentMux {from}, the version before the last upgrade? Your computer asks for your password \
         to install it, then AgentMux {from} opens. Your tabs from before the upgrade come back, but what you \
         changed in {to} stays in {to}'s own data."
    )
}

/// The new version could not be kept running (`why` finishes "AgentMux X ___"): offer to
/// go back to the version the last upgrade replaced. `true` once the helper has
/// accepted and is waiting for this launcher to exit: the caller exits, and the waiter
/// starts the old version. Only for a `.deb`/`.rpm` install, one upgrade from the app
/// ago (the helper's record), and the user's yes.
pub fn offer_go_back(own_version: &str, own_dir: &Path, data_dir: &Path, why: &str) -> bool {
    if own_dir != Path::new(INSTALL_BIN) || !own_dir.join(HELPER_NAME).is_file() {
        return false;
    }
    let record = Path::new(STATE_DIR).join("last-upgrade");
    let Some(from) = go_back_version(&record, own_version, std::time::SystemTime::now()) else { return false };
    crate::log(&format!("upgrade: {own_version} {why}; offering to go back to {from}"));
    if !crate::upgrade_linux::confirm_labelled("Go back to the previous AgentMux?", &go_back_question(&from, own_version, why), "Go back") {
        return false;
    }
    let up = PackageUpgrade::go_back(&from, data_dir);
    if authorize(&up, std::process::id(), GO_BACK_AUTHORIZE_WITHIN) {
        return true;
    }
    // Nothing was installed: the reason is the result file's, if there is one (a cancelled prompt has none).
    if let Some(error) = read_result(&up) {
        crate::upgrade_linux::notify("AgentMux couldn't go back", &format!("The install didn't finish: {error}"));
    }
    false
}

/// Close the password prompt (and with it the waiter), as "Not now".
pub fn dismiss() {
    let group = WAITER_GROUP.swap(0, std::sync::atomic::Ordering::SeqCst);
    if group != 0 {
        let _ = agentmux_common::process::signal_process_group(group, libc::SIGTERM);
    }
}

/// The last line the helper left on stderr, without its prefix.
fn last_reason(err: &Path) -> Option<String> {
    let text = std::fs::read_to_string(err).ok()?;
    let line = text.lines().rev().map(str::trim).find(|l| !l.is_empty())?;
    Some(line.strip_prefix("agentmux-install-update: ").unwrap_or(line).chars().take(300).collect())
}

/// Ask for permission to install `up` (the desktop's password prompt, through
/// polkit) and have its helper check the file. `true` once the helper is waiting
/// for AgentMux to quit: the waiter then carries on without the launcher.
/// `false` (with the reason left for srv, unless the user simply said no) when
/// nothing will be installed, so AgentMux keeps running.
pub fn authorize(up: &PackageUpgrade, launcher_pid: u32, within: std::time::Duration) -> bool {
    authorize_with(Path::new("/usr/bin/pkexec"), &Path::new(INSTALL_BIN).join(HELPER_NAME), Path::new(START_COMMAND), up, launcher_pid, within)
}

fn authorize_with(pkexec: &Path, helper: &Path, start: &Path, up: &PackageUpgrade, launcher_pid: u32, within: std::time::Duration) -> bool {
    use std::os::unix::process::CommandExt;
    let (out, err, result) = up.files();
    let _ = std::fs::remove_file(&result);
    if !pkexec.is_file() {
        leave_result(up, &format!("this computer has no pkexec to ask for permission. Install it yourself: {}", up.manual_command()));
        return false;
    }
    let _ = std::fs::remove_file(&out);
    let mut cmd = std::process::Command::new("/bin/sh");
    cmd.arg("-c")
        .arg(WAITER)
        .arg("agentmux-install-waiter")
        .arg(pkexec)
        .arg(helper)
        .arg(&up.installer)
        .arg(&up.version)
        .arg(launcher_pid.to_string())
        .arg(&out)
        .arg(&err)
        .arg(&result)
        .arg(start)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        // Its own session: it outlives this launcher, and a signal to this one's group never reaches it.
        .process_group(0);
    crate::upgrade_restart::restart_env(&mut cmd, std::env::vars_os());
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            leave_result(up, &format!("couldn't start the installer: {e}"));
            return false;
        }
    };
    let group = child.id();
    WAITER_GROUP.store(group, std::sync::atomic::Ordering::SeqCst);
    let started = std::time::Instant::now();
    let ready = loop {
        if std::fs::read_to_string(&out).is_ok_and(|t| t.lines().any(|l| l == "READY")) {
            break true;
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                // It stopped before the helper was ready: the exit status says why.
                match status.code() {
                    Some(DISMISSED) | None => crate::log("upgrade: the password prompt was dismissed; nothing is installed"),
                    Some(NOT_AUTHORIZED) => {
                        crate::log("upgrade: polkit couldn't authorise the install (no password prompt here, or not allowed)");
                        leave_result(up, &format!("no password prompt could be shown here. Install it yourself: {}", up.manual_command()));
                    }
                    Some(code) => {
                        let why = last_reason(&err).unwrap_or_else(|| format!("the installer stopped with status {code}"));
                        crate::log(&format!("upgrade: the package install didn't start: {why}"));
                        leave_result(up, &why);
                    }
                }
                break false;
            }
            Ok(None) if started.elapsed() > within => {
                let _ = agentmux_common::process::signal_process_group(group, libc::SIGTERM);
                crate::log("upgrade: no answer to the password prompt in time");
                break false;
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(150)),
            Err(e) => {
                crate::log(&format!("upgrade: lost the installer's waiter: {e}"));
                break false;
            }
        }
    };
    // Still the waiter's only while this prompt is open; a dismiss after this can't reach it.
    let _ = WAITER_GROUP.compare_exchange(group, 0, std::sync::atomic::Ordering::SeqCst, std::sync::atomic::Ordering::SeqCst);
    if ready {
        // Leave it running: it is the waiter that starts AgentMux again. Dropping the
        // handle doesn't stop it.
        drop(child);
    }
    ready
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::time::Duration;

    const SHA: &str = "2c26b46b68ffc68ff99b453c1d30413413422d706483bfa0f98a5e886266e7ae"; // "foo"

    /// A data folder with `updates/0.60.2/<name>` holding "foo", and a bin folder with the helper.
    fn layout(name: &str) -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        let updates = data.join("updates/0.60.2");
        std::fs::create_dir_all(&updates).unwrap();
        let installer = updates.join(name);
        std::fs::write(&installer, "foo").unwrap();
        let bin = dir.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join(HELPER_NAME), "#!/bin/sh\n").unwrap();
        (dir, installer, data, bin)
    }

    #[test]
    fn only_the_releases_package_in_this_versions_updates_folder_passes() {
        for name in ["AgentMux_0.60.2_amd64.deb", "AgentMux-0.60.2-1.x86_64.rpm"] {
            let (_dir, installer, data, bin) = layout(name);
            let up = check(&installer, SHA, "0.60.2", &bin, &data, &bin).unwrap();
            assert_eq!(up.installer, installer);
            assert_eq!(up.deb(), name.ends_with(".deb"));
        }
    }

    #[test]
    fn a_package_that_isnt_what_srv_downloaded_is_refused() {
        let (dir, installer, data, bin) = layout("AgentMux_0.60.2_amd64.deb");
        assert!(check(&installer, SHA, "0.60.3", &bin, &data, &bin).is_err(), "another version");
        assert!(check(&installer, &SHA.replace('2', "3"), "0.60.2", &bin, &data, &bin).is_err(), "a changed file");
        assert!(check(&installer, "zz", "0.60.2", &bin, &data, &bin).is_err(), "not a digest");
        assert!(check(&installer, SHA, "0.60.2", &dir.path().join("elsewhere"), &data, &bin).is_err(), "not the installed launcher");
        let wrong_name = installer.with_file_name("agentmux.deb");
        std::fs::copy(&installer, &wrong_name).unwrap();
        assert!(check(&wrong_name, SHA, "0.60.2", &bin, &data, &bin).is_err(), "the release's name only");
        let outside = dir.path().join("AgentMux_0.60.2_amd64.deb");
        std::fs::copy(&installer, &outside).unwrap();
        assert!(check(&outside, SHA, "0.60.2", &bin, &data, &bin).is_err(), "only from this version's updates folder");
        std::fs::remove_file(bin.join(HELPER_NAME)).unwrap();
        assert!(check(&installer, SHA, "0.60.2", &bin, &data, &bin).is_err(), "a package from before the helper shipped");
    }

    #[test]
    fn the_manual_command_names_the_package_manager_and_the_file() {
        let (_dir, installer, data, bin) = layout("AgentMux_0.60.2_amd64.deb");
        let up = check(&installer, SHA, "0.60.2", &bin, &data, &bin).unwrap();
        assert_eq!(up.manual_command(), format!("sudo apt install {}", installer.display()));
        let (_dir, installer, data, bin) = layout("AgentMux-0.60.2-1.x86_64.rpm");
        let up = check(&installer, SHA, "0.60.2", &bin, &data, &bin).unwrap();
        assert_eq!(up.manual_command(), format!("sudo dnf install {}", installer.display()));
    }

    /// An executable that stands in for pkexec: runs `body`, given the helper and its arguments.
    fn fake_pkexec(dir: &Path, body: &str) -> PathBuf {
        let path = dir.join("pkexec");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn result_error(up: &PackageUpgrade) -> Option<String> {
        let text = std::fs::read_to_string(up.files().2).ok()?;
        serde_json::from_str::<serde_json::Value>(&text).ok()?.get("error")?.as_str().map(String::from)
    }

    /// A script standing in for the command that starts AgentMux: it records that it ran.
    fn start_script(dir: &Path) -> (PathBuf, PathBuf) {
        let marker = dir.join("started");
        let start = dir.join("start.sh");
        std::fs::write(&start, format!("#!/bin/sh\necho started >> {}\n", marker.display())).unwrap();
        std::fs::set_permissions(&start, std::fs::Permissions::from_mode(0o755)).unwrap();
        (start, marker)
    }

    /// For the tests that never get as far as starting AgentMux.
    fn no_start(dir: &Path) -> PathBuf {
        start_script(dir).0
    }

    fn wait_for(what: impl Fn() -> bool) -> bool {
        let started = std::time::Instant::now();
        while !what() && started.elapsed() < Duration::from_secs(15) {
            std::thread::sleep(Duration::from_millis(50));
        }
        what()
    }

    #[test]
    fn a_failed_install_leaves_its_reason_and_the_waiter_starts_agentmux_once_the_launcher_is_gone() {
        let (dir, installer, data, bin) = layout("AgentMux_0.60.2_amd64.deb");
        let up = check(&installer, SHA, "0.60.2", &bin, &data, &bin).unwrap();
        // Authorised: READY, then an install that fails with a reason on stderr.
        let pkexec = fake_pkexec(dir.path(), "echo READY; echo 'agentmux-install-update: apt-get couldn'\\''t install it' >&2; exit 1");
        let (start, marker) = start_script(dir.path());
        // A process standing in for the launcher, which is still quitting.
        let mut launcher = std::process::Command::new("sleep").arg("60").spawn().unwrap();
        assert!(authorize_with(&pkexec, &bin.join(HELPER_NAME), &start, &up, launcher.id(), Duration::from_secs(10)));
        assert!(wait_for(|| result_error(&up).is_some()));
        assert_eq!(result_error(&up).as_deref(), Some("apt-get couldn't install it"));
        std::thread::sleep(Duration::from_millis(1500));
        assert!(!marker.exists(), "not while the launcher is still there");
        launcher.kill().unwrap();
        launcher.wait().unwrap();
        assert!(wait_for(|| marker.exists()), "AgentMux is started again after a failed install");
    }

    #[test]
    fn a_successful_install_leaves_no_failure_and_starts_agentmux() {
        let (dir, installer, data, bin) = layout("AgentMux_0.60.2_amd64.deb");
        let up = check(&installer, SHA, "0.60.2", &bin, &data, &bin).unwrap();
        let (start, marker) = start_script(dir.path());
        let pkexec = fake_pkexec(dir.path(), "echo READY; exit 0");
        let (out, err, result) = up.files();
        std::fs::write(&result, "old").unwrap();
        // Run the waiter itself, with a launcher that is already gone.
        let status = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(WAITER)
            .arg("waiter")
            .arg(&pkexec)
            .arg(bin.join(HELPER_NAME))
            .arg(&installer)
            .arg("0.60.2")
            .arg("999999")
            .arg(&out)
            .arg(&err)
            .arg(&result)
            .arg(&start)
            .status()
            .unwrap();
        assert!(status.success());
        assert!(marker.exists(), "AgentMux is started once the install is done");
        assert!(!result.exists(), "a successful install leaves no failure behind");
    }

    #[test]
    fn going_back_is_offered_only_for_the_version_the_last_upgrade_installed() {
        let dir = tempfile::tempdir().unwrap();
        let record = dir.path().join("last-upgrade");
        let now = std::time::SystemTime::now();
        assert_eq!(go_back_version(&record, "0.60.2", now), None, "no upgrade done from the app");
        std::fs::write(&record, "from=0.60.1\nto=0.60.2\n").unwrap();
        assert_eq!(go_back_version(&record, "0.60.2", now).as_deref(), Some("0.60.1"));
        assert_eq!(go_back_version(&record, "0.60.3", now), None, "a version the upgrade didn't install");
        assert_eq!(go_back_version(&record, "0.60.2", now + GO_BACK_WITHIN + Duration::from_secs(60)), None, "too long ago");
        for bad in ["from=\nto=0.60.2\n", "from=0.60.1 x\nto=0.60.2\n", "from=../x\nto=0.60.2\n", "from=0.60.2\nto=0.60.2\n", "to=0.60.2\n"] {
            std::fs::write(&record, bad).unwrap();
            assert_eq!(go_back_version(&record, "0.60.2", now), None, "{bad:?}");
        }
    }

    #[test]
    fn going_back_hands_the_helper_no_file_and_restarts_the_old_version() {
        let (dir, _installer, data, _bin) = layout("AgentMux_0.60.2_amd64.deb");
        let up = PackageUpgrade::go_back("0.60.1", &data);
        assert!(up.is_go_back());
        let args = dir.path().join("args");
        let pkexec = fake_pkexec(dir.path(), &format!("printf '%s\\n' \"$@\" > {}; echo READY; exit 0", args.display()));
        let (start, marker) = start_script(dir.path());
        // A process standing in for the launcher, which exits once the helper says READY.
        let mut launcher = std::process::Command::new("sleep").arg("60").spawn().unwrap();
        let pid = launcher.id();
        assert!(authorize_with(&pkexec, Path::new("/opt/agentmux/bin/agentmux-install-update"), &start, &up, pid, Duration::from_secs(10)));
        launcher.kill().unwrap();
        launcher.wait().unwrap();
        assert!(wait_for(|| marker.exists()), "the old version is started once the launcher is gone");
        let got = std::fs::read_to_string(&args).unwrap();
        assert_eq!(got, format!("/opt/agentmux/bin/agentmux-install-update\n--go-back\n0.60.1\n{pid}\n"));
        assert!(up.manual_command().contains("0.60.1"));
    }

    #[test]
    fn a_dismissed_prompt_installs_nothing_and_says_nothing() {
        let (dir, installer, data, bin) = layout("AgentMux_0.60.2_amd64.deb");
        let up = check(&installer, SHA, "0.60.2", &bin, &data, &bin).unwrap();
        let pkexec = fake_pkexec(dir.path(), "exit 126");
        assert!(!authorize_with(&pkexec, &bin.join(HELPER_NAME), &no_start(dir.path()), &up, 1, Duration::from_secs(10)));
        assert_eq!(result_error(&up), None, "the user said no: not an error");
    }

    #[test]
    fn no_password_prompt_leaves_the_command_to_run_by_hand() {
        let (dir, installer, data, bin) = layout("AgentMux_0.60.2_amd64.deb");
        let up = check(&installer, SHA, "0.60.2", &bin, &data, &bin).unwrap();
        let pkexec = fake_pkexec(dir.path(), "exit 127");
        assert!(!authorize_with(&pkexec, &bin.join(HELPER_NAME), &no_start(dir.path()), &up, 1, Duration::from_secs(10)));
        let error = result_error(&up).unwrap();
        assert!(error.contains(&format!("sudo apt install {}", installer.display())), "{error}");
        // No pkexec at all says the same.
        assert!(!authorize_with(&dir.path().join("none"), &bin.join(HELPER_NAME), &no_start(dir.path()), &up, 1, Duration::from_secs(10)));
        assert!(result_error(&up).unwrap().contains("no pkexec"));
    }

    #[test]
    fn a_file_the_helper_refuses_leaves_its_reason_and_nothing_quits() {
        let (dir, installer, data, bin) = layout("AgentMux_0.60.2_amd64.deb");
        let up = check(&installer, SHA, "0.60.2", &bin, &data, &bin).unwrap();
        let pkexec = fake_pkexec(dir.path(), "echo 'agentmux-install-update: the file doesn'\\''t match the checksum GitHub lists for it' >&2; exit 1");
        assert!(!authorize_with(&pkexec, &bin.join(HELPER_NAME), &no_start(dir.path()), &up, 1, Duration::from_secs(10)));
        assert_eq!(result_error(&up).as_deref(), Some("the file doesn't match the checksum GitHub lists for it"));
    }

    #[test]
    fn a_prompt_left_unanswered_is_closed_when_time_is_up() {
        let (dir, installer, data, bin) = layout("AgentMux_0.60.2_amd64.deb");
        let up = check(&installer, SHA, "0.60.2", &bin, &data, &bin).unwrap();
        let pkexec = fake_pkexec(dir.path(), "exec sleep 30");
        let started = std::time::Instant::now();
        assert!(!authorize_with(&pkexec, &bin.join(HELPER_NAME), &no_start(dir.path()), &up, 1, Duration::from_millis(600)));
        assert!(started.elapsed() < Duration::from_secs(10));
        assert_eq!(result_error(&up), None);
    }
}
