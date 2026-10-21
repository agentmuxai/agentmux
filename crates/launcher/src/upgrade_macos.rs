// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The macOS side of "Upgrade & restart"
//! (`docs/specs/SPEC_UPGRADE_FOR_USERS_AND_AGENTS_2026_10_15.md` §4.2, phase
//! 3c), for `upgrade_restart`: checking the `.dmg` srv downloaded and verified,
//! asking the user with a native dialog (`osascript`), and the script that
//! replaces `AgentMux.app` once this launcher has exited.
//!
//! The app can't replace itself while it runs (its own launcher is inside
//! it), so the swap is a small `/bin/sh` script started detached just before
//! the launcher exits ([`apply_command`]). It works on a private copy of the
//! `.dmg`, checks it again against the release's SHA-256, mounts it read-only,
//! checks the new app's code signature (and, when this app passes Gatekeeper,
//! that the new one does too, from the same team), copies it into a private
//! folder it creates beside the running one and swaps the two, holding a
//! lock on the app so two launchers (two channels can run one `.app`) never
//! swap it at once. This version goes to the Trash, so it can still be put
//! back. Whatever fails, the script opens an AgentMux again: the
//! new one, or this one, still in place.
//!
//! The pure parts (paths, checks, the dialog's answer, the script's
//! arguments) build and are tested on every platform; only showing the
//! dialog is macOS-only.

use std::path::{Path, PathBuf};

/// The release file a macOS install upgrades with, as
/// `.github/workflows/build-macos.yml` names it (srv's `update_install`).
pub fn dmg_name(version: &str) -> String {
    format!("AgentMux_{version}_arm64.dmg")
}

/// The outermost `.app` bundle holding `path`: the launcher runs in
/// `AgentMux.app/Contents/MacOS/`, and srv in a helper app nested inside it.
pub fn outermost_app(path: &Path) -> Option<PathBuf> {
    agentmux_common::runtime_mode::outermost_app_bundle(path)
}

/// Why `app` can't be replaced where it is, or `None` when it can: macOS runs
/// a quarantined app from a read-only copy (App Translocation), an app opened
/// straight from its mounted `.dmg` is on a read-only volume, and the folder
/// that holds it must be one this user can write.
pub fn not_replaceable(app: &Path) -> Option<String> {
    if app.to_string_lossy().contains("/AppTranslocation/") {
        return Some(format!(
            "macOS runs this AgentMux from a temporary read-only copy (App Translocation); move {} to Applications and open it from there",
            app.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
        ));
    }
    let Some(parent) = app.parent() else {
        return Some(format!("{} has no folder", app.display()));
    };
    if world_writable(parent) {
        return Some(format!(
            "{} can be changed by every account on this Mac; move AgentMux to Applications to upgrade it in place",
            parent.display()
        ));
    }
    if !can_write_in(parent) {
        return Some(format!(
            "{} can't be written (a mounted .dmg, or a folder this user can't change); move AgentMux to Applications to upgrade it in place",
            parent.display()
        ));
    }
    None
}

/// Whether every account can create files in `dir` (`/Users/Shared`, `/tmp`):
/// another account could then plant links where the swap writes.
#[cfg(unix)]
fn world_writable(dir: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(dir).is_ok_and(|m| m.permissions().mode() & 0o002 != 0)
}

#[cfg(not(unix))]
fn world_writable(_dir: &Path) -> bool {
    false
}

/// Whether this user can create a file in `dir` (a read-only volume can't).
fn can_write_in(dir: &Path) -> bool {
    let probe = dir.join(format!(".agentmux-write-check-{}", std::process::id()));
    match std::fs::File::create(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// What a confirmed macOS upgrade needs: the verified `.dmg`, its digest, the
/// app it replaces, the data channel to keep, and where the script logs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MacUpgrade {
    pub dmg: PathBuf,
    pub sha256: String,
    pub version: String,
    pub app: PathBuf,
    pub channel: String,
}

impl MacUpgrade {
    /// `updates/<v>/apply.log` beside the `.dmg`: what the script did.
    pub fn log(&self) -> PathBuf {
        self.dmg.with_file_name("apply.log")
    }
}

/// Check what srv names for a macOS upgrade to `version`: that version's
/// `.dmg`, in this version's own `updates/<v>/`, whole by the release's
/// `sha256`, and an app here that can be replaced. `own_dir` is the
/// launcher's folder (`AgentMux.app/Contents/MacOS`), `channel_dir` this
/// channel's data folder, whose name is the channel the new version keeps.
pub fn check_dmg(
    dmg: &Path,
    sha256: &str,
    version: &str,
    own_dir: &Path,
    data_dir: &Path,
    channel_dir: &Path,
) -> Result<MacUpgrade, String> {
    let named_right = dmg.file_name().is_some_and(|n| n == dmg_name(version).as_str());
    let in_updates = dmg.parent() == Some(data_dir.join("updates").join(version).as_path());
    if !dmg.is_absolute() || !named_right || !in_updates || !dmg.is_file() {
        return Err(format!("{} isn't the AgentMux {version} disk image AgentMux downloaded", dmg.display()));
    }
    let app = outermost_app(own_dir).ok_or("this AgentMux isn't running from an .app")?;
    if let Some(why) = not_replaceable(&app) {
        return Err(why);
    }
    // Before asking: a file changed since srv checked it fails here. The
    // check that binds is the script's, on its own private copy.
    if !crate::upgrade_restart::is_sha256_hex(sha256) || crate::upgrade_restart::file_sha256(dmg).as_deref() != Some(sha256) {
        return Err(format!("{} isn't the AgentMux {version} release; download it again", dmg.display()));
    }
    let channel = channel_dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    Ok(MacUpgrade { dmg: dmg.to_path_buf(), sha256: sha256.to_string(), version: version.to_string(), app, channel })
}

/// The dialog's question.
pub fn question(up: &MacUpgrade, stopping_paragraph: &str) -> String {
    format!(
        "Restart AgentMux to upgrade to {v}?\n\n\
         Every window closes and every agent stops, then AgentMux {v} replaces\n{}\nand opens. \
         This version goes to the Trash.\n\n\
         Your tabs come back and agents resume their conversations. macOS may ask again for \
         permissions it gave this version.{stopping_paragraph}",
        up.app.display(),
        v = up.version
    )
}

/// The label of the dialog's Yes.
const YES: &str = "Upgrade & restart";

/// Whether `osascript`'s output for the dialog is an explicit Yes: the button,
/// and not given up (the dialog closes itself, as No, when its time is up).
pub fn said_yes(stdout: &str) -> bool {
    stdout.contains(&format!("button returned:{YES}")) && !stdout.contains("gave up:true")
}

/// The dialog's open `osascript`, 0 when none.
#[cfg(target_os = "macos")]
static DIALOG_PID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Ask with a native dialog, which no page or agent tool in AgentMux can
/// click. "Not now" is the default button. The text goes to `osascript` as
/// arguments, never into the script, so nothing in it is parsed. It closes
/// itself after `upgrade_restart::ANSWER_WITHIN`.
#[cfg(target_os = "macos")]
pub fn confirm(title: &str, body: &str) -> bool {
    use std::sync::atomic::Ordering;
    let within = crate::upgrade_restart::ANSWER_WITHIN.as_secs();
    let script = format!(
        "on run argv\n\
         display dialog (item 1 of argv) with title (item 2 of argv) buttons {{\"Not now\", \"{YES}\"}} \
         default button \"Not now\" cancel button \"Not now\" with icon note giving up after {within}\n\
         end run"
    );
    let child = std::process::Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg(script)
        .arg(body)
        .arg(title)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn();
    let child = match child {
        Ok(c) => c,
        Err(e) => {
            crate::log(&format!("upgrade: couldn't open osascript to ask: {e}"));
            return false;
        }
    };
    let pid = child.id();
    DIALOG_PID.store(pid, Ordering::SeqCst);
    let answer = child.wait_with_output();
    let _ = DIALOG_PID.compare_exchange(pid, 0, Ordering::SeqCst, Ordering::SeqCst);
    answer.is_ok_and(|out| out.status.success() && said_yes(&String::from_utf8_lossy(&out.stdout)))
}

/// Close the open dialog (as "Not now").
#[cfg(target_os = "macos")]
pub fn dismiss() {
    let pid = DIALOG_PID.swap(0, std::sync::atomic::Ordering::SeqCst);
    if pid != 0 {
        // SAFETY: signals the dialog this launcher started and still waits on.
        unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
    }
}

/// The swap, run by `/bin/sh` once this launcher (`$1`) has exited. Every
/// path is an argument, never part of the script, so nothing in one is
/// parsed. Arguments: launcher pid, `.dmg`, SHA-256, version, the app to
/// replace, the channel to record beside it (may be empty), the log.
const APPLY_SCRIPT: &str = r#"
pid="$1"; dmg="$2"; sha="$3"; ver="$4"; app="$5"; channel="$6"; log="$7"
exec >>"$log" 2>&1
say() { echo "$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ) $*"; }
work=""; mnt=""; staging=""
cleanup() {
  [ -n "$mnt" ] && { /usr/bin/hdiutil detach -quiet "$mnt" || /usr/bin/hdiutil detach -quiet -force "$mnt"; }
  [ -n "$staging" ] && /bin/rm -rf "$staging"
  [ -n "$work" ] && /bin/rm -rf "$work"
  [ -n "$locked" ] && /bin/rm -rf "$lock"
}
fail() { say "failed: $*; opening the AgentMux that is in place"; cleanup; /usr/bin/open "$app"; exit 1; }
team_of() { /usr/bin/codesign -dv "$1" 2>&1 | /usr/bin/sed -n 's/^TeamIdentifier=//p'; }
parent=$(/usr/bin/dirname "$app"); base=$(/usr/bin/basename "$app")
lock="$parent/.$base.upgrade-lock"; locked=""
say "upgrade to $ver: waiting for the launcher ($pid) to exit"
i=0
while /bin/kill -0 "$pid" 2>/dev/null; do
  i=$((i+1)); [ "$i" -gt 600 ] && { say "the launcher is still running after 60 s; nothing changed"; exit 1; }
  /bin/sleep 0.1
done
# One swap of this app at a time: two launchers on different channels can
# run the same .app. A lock left by a script that is gone is taken over.
if ! /bin/mkdir "$lock" 2>/dev/null; then
  other=$(/bin/cat "$lock/pid" 2>/dev/null)
  if [ -n "$other" ] && /bin/kill -0 "$other" 2>/dev/null; then
    say "another upgrade of $app is running ($other); nothing changed"; exit 1
  fi
  /bin/rm -rf "$lock"; /bin/mkdir "$lock" 2>/dev/null || { say "couldn't take the upgrade lock; nothing changed"; exit 1; }
fi
locked=1; echo "$$" > "$lock/pid"
work=$(/usr/bin/mktemp -d "${TMPDIR:-/tmp}/agentmux-upgrade.XXXXXX") || fail "no temporary folder"
/bin/cp "$dmg" "$work/u.dmg" || fail "couldn't copy the disk image"
got=$(/usr/bin/shasum -a 256 "$work/u.dmg" | /usr/bin/awk '{print $1}')
[ "$got" = "$sha" ] || fail "the disk image isn't the release (sha256 $got)"
mnt="$work/mnt"; /bin/mkdir "$mnt" || fail "no mount point"
/usr/bin/hdiutil attach -quiet -nobrowse -noautoopen -readonly -mountpoint "$mnt" "$work/u.dmg" || { mnt=""; fail "couldn't mount the disk image"; }
new="$mnt/AgentMux.app"
[ -d "$new" ] || new=$(/bin/ls -d "$mnt"/*.app 2>/dev/null | /usr/bin/head -n 1)
[ -n "$new" ] && [ -d "$new" ] || fail "no app in the disk image"
got_ver=$(/usr/libexec/PlistBuddy -c "Print :CFBundleShortVersionString" "$new/Contents/Info.plist" 2>/dev/null)
[ "$got_ver" = "$ver" ] || fail "the disk image holds version '$got_ver', not $ver"
/usr/bin/codesign --verify --deep --strict "$new" || fail "the new app's code signature doesn't verify"
old_team=$(team_of "$app")
if [ -n "$old_team" ] && [ "$old_team" != "not set" ]; then
  [ "$(team_of "$new")" = "$old_team" ] || fail "the new app isn't signed by this app's team ($old_team)"
fi
if /usr/sbin/spctl -a -t exec "$app" 2>/dev/null; then
  /usr/sbin/spctl -a -t exec "$new" || fail "Gatekeeper doesn't accept the new app"
fi
# A folder of our own beside the app (created new, unpredictable name), so
# nothing another account placed there is followed or overwritten.
staging=$(/usr/bin/mktemp -d "$parent/.$base.upgrading.XXXXXX") || fail "couldn't make a folder beside the app"
/usr/bin/ditto "$new" "$staging/$base" || fail "couldn't copy the new app beside this one"
/usr/bin/codesign --verify --deep --strict "$staging/$base" || fail "the copied app's code signature doesn't verify"
/usr/bin/hdiutil detach -quiet "$mnt" || /usr/bin/hdiutil detach -quiet -force "$mnt"; mnt=""
old_ver=$(/usr/libexec/PlistBuddy -c "Print :CFBundleShortVersionString" "$app/Contents/Info.plist" 2>/dev/null)
trash="$HOME/.Trash/${base%.app} $old_ver $(/bin/date +%Y%m%d-%H%M%S).app"
/bin/mv "$app" "$trash" || fail "couldn't move this version to the Trash"
if ! /bin/mv "$staging/$base" "$app"; then
  /bin/mv "$trash" "$app"
  fail "couldn't put the new app in place; this version is back"
fi
if [ -n "$channel" ]; then
  # Written to a new file of our own, then renamed over the record: a rename
  # replaces whatever is at that name and never follows it.
  record="$parent/.$base.agentmux-channel"
  tmp=$(/usr/bin/mktemp "$parent/.$base.agentmux-channel.XXXXXX") &&
    printf '%s\n' "$channel" > "$tmp" && /bin/mv "$tmp" "$record" || { [ -n "$tmp" ] && /bin/rm -f "$tmp"; say "couldn't record the channel beside the app"; }
fi
cleanup
say "replaced $app with $ver; this version is in the Trash as $trash"
/usr/bin/open "$app" || say "couldn't open the new app"
"#;

/// The detached `/bin/sh` that swaps the app once launcher `launcher_pid` is
/// gone. `upgrade_restart::relaunch` adds the environment and process group.
pub fn apply_command(up: &MacUpgrade, launcher_pid: u32) -> std::process::Command {
    let mut cmd = std::process::Command::new("/bin/sh");
    cmd.arg("-c")
        .arg(APPLY_SCRIPT)
        .arg("agentmux-upgrade")
        .arg(launcher_pid.to_string())
        .arg(&up.dmg)
        .arg(&up.sha256)
        .arg(&up.version)
        .arg(&up.app)
        .arg(&up.channel)
        .arg(up.log());
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_outermost_app_is_the_one_replaced() {
        let srv = Path::new("/Applications/AgentMux.app/Contents/Helpers/AgentMux Server.app/Contents/MacOS/srv");
        assert_eq!(outermost_app(srv).as_deref(), Some(Path::new("/Applications/AgentMux.app")));
        let launcher = Path::new("/Users/u/Apps/AgentMux 2.app/Contents/MacOS");
        assert_eq!(outermost_app(launcher).as_deref(), Some(Path::new("/Users/u/Apps/AgentMux 2.app")));
        assert_eq!(outermost_app(Path::new("/opt/agentmux/bin")), None);
    }

    #[test]
    fn a_translocated_or_unwritable_app_is_not_replaced() {
        let translocated = Path::new("/private/var/folders/x/T/AppTranslocation/ABC/d/AgentMux.app");
        assert!(not_replaceable(translocated).unwrap().contains("App Translocation"));
        assert!(not_replaceable(Path::new("/no/such/folder/AgentMux.app")).unwrap().contains("can't be written"));
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(not_replaceable(&dir.path().join("AgentMux.app")), None);
    }

    #[cfg(unix)]
    #[test]
    fn an_app_in_a_folder_every_account_can_write_is_not_replaced() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o1777)).unwrap();
        assert!(not_replaceable(&dir.path().join("AgentMux.app")).unwrap().contains("every account"));
    }

    fn layout(root: &Path, version: &str) -> (PathBuf, PathBuf, PathBuf, PathBuf) {
        let own_dir = root.join("Apps/AgentMux.app/Contents/MacOS");
        std::fs::create_dir_all(&own_dir).unwrap();
        let channel_dir = root.join("channels/stable");
        let data_dir = channel_dir.join("versions/0.59.35");
        let updates = data_dir.join("updates").join(version);
        std::fs::create_dir_all(&updates).unwrap();
        let dmg = updates.join(dmg_name(version));
        std::fs::write(&dmg, b"dmg").unwrap();
        (own_dir, data_dir, channel_dir, dmg)
    }

    #[test]
    fn only_the_downloaded_release_dmg_is_accepted() {
        let root = tempfile::tempdir().unwrap();
        let (own_dir, data_dir, channel_dir, dmg) = layout(root.path(), "0.59.36");
        let sha = crate::upgrade_restart::file_sha256(&dmg).unwrap();
        let up = check_dmg(&dmg, &sha, "0.59.36", &own_dir, &data_dir, &channel_dir).unwrap();
        assert_eq!(up.app, root.path().join("Apps/AgentMux.app"));
        assert_eq!(up.channel, "stable");
        assert_eq!(up.log(), dmg.with_file_name("apply.log"));

        // Another version's name, a wrong digest, or a file outside updates/<v>/.
        assert!(check_dmg(&dmg, &sha, "0.59.37", &own_dir, &data_dir, &channel_dir).is_err());
        assert!(check_dmg(&dmg, &"0".repeat(64), "0.59.36", &own_dir, &data_dir, &channel_dir).is_err());
        let elsewhere = root.path().join(dmg_name("0.59.36"));
        std::fs::write(&elsewhere, b"dmg").unwrap();
        assert!(check_dmg(&elsewhere, &sha, "0.59.36", &own_dir, &data_dir, &channel_dir).is_err());
        // Not running from an .app.
        assert!(check_dmg(&dmg, &sha, "0.59.36", root.path(), &data_dir, &channel_dir).is_err());
    }

    #[test]
    fn only_an_explicit_yes_in_time_counts() {
        assert!(said_yes("button returned:Upgrade & restart, gave up:false\n"));
        assert!(!said_yes("button returned:, gave up:true\n"));
        assert!(!said_yes("button returned:Not now, gave up:false\n"));
        assert!(!said_yes(""));
    }

    #[test]
    fn the_script_gets_every_path_as_an_argument() {
        let up = MacUpgrade {
            dmg: PathBuf::from("/Users/u/.agentmux/channels/stable/versions/0.59.35/updates/0.59.36/AgentMux_0.59.36_arm64.dmg"),
            sha256: "a".repeat(64),
            version: "0.59.36".into(),
            app: PathBuf::from("/Users/u/My Apps/AgentMux.app"),
            channel: "stable".into(),
        };
        let cmd = apply_command(&up, 4242);
        let args: Vec<String> = cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(cmd.get_program(), "/bin/sh");
        assert_eq!(args[0], "-c");
        assert!(!args[1].contains("My Apps"), "no path is inside the script");
        // The record the script writes is the one the next version reads.
        let record = agentmux_common::runtime_mode::mac_app_channel_file(&up.app);
        assert_eq!(record, Path::new("/Users/u/My Apps/.AgentMux.app.agentmux-channel"));
        assert!(args[1].contains(r#"record="$parent/.$base.agentmux-channel""#));
        assert!(args[1].contains("mkdir \"$lock\""), "the swap holds a lock on the app");
        assert_eq!(
            &args[2..],
            &[
                "agentmux-upgrade",
                "4242",
                &up.dmg.display().to_string(),
                &up.sha256,
                "0.59.36",
                "/Users/u/My Apps/AgentMux.app",
                "stable",
                &up.log().display().to_string(),
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_script_parses() {
        let out = std::process::Command::new("/bin/sh").arg("-n").arg("-c").arg(APPLY_SCRIPT).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    }
}
