// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! "Upgrade & restart" (`docs/specs/SPEC_UPGRADE_FOR_USERS_AND_AGENTS_2026_10_15.md`
//! §4.1, phases 3b and 3d): restart a Windows portable into the newer
//! release srv downloaded, verified and unpacked next to it, or run the newer
//! release's installer on a `setup.exe` install (`backend/update_download.rs`).
//!
//! srv asks with an `AGENTMUXSRV-UPGRADE` line on its stderr
//! (`agentmux_common::srv_stderr`); the reader in `srv_spawner` hands it here.
//! The supervisor then:
//! 1. checks what srv names again ([`staged`]): an AgentMux portable of
//!    exactly that version, newer than this one, beside a portable; or that
//!    version's installer, with the release's digest, in this version's own
//!    `updates/<v>/`, on a `setup.exe` install;
//! 2. asks the user with a native dialog ([`confirm`]), which no page or agent
//!    tool in AgentMux can click: a restart stops every agent, so it takes the
//!    user's own answer here, whoever asked srv;
//! 3. quits the host the way the tray's Quit does, which closes the windows and
//!    saves the session;
//! 4. after srv has stopped (so the new version copies complete data,
//!    `srv/src/bootstrap/carry_forward.rs`), starts the new launcher, or a
//!    hidden PowerShell that waits for this launcher to be gone, runs the
//!    installer silently and starts AgentMux again ([`relaunch`]), and exits.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use agentmux_common::srv_stderr::{UpgradeRequest, UpgradeTarget};
use tokio::sync::mpsc;

/// A confirmed restart that hasn't happened within this long (the host
/// never quit) no longer restarts into the new version on a later quit.
pub const CONFIRMED_FOR: Duration = Duration::from_secs(5 * 60);

static REQUESTS: Mutex<Option<mpsc::UnboundedSender<UpgradeRequest>>> = Mutex::new(None);
/// A Yes counts only this long after its dialog opened (the dialog itself
/// can't time out). Shorter than srv's wait (`RESTART_GRACE`, 3 minutes in
/// `srv/src/backend/update_restart.rs`), so by the time the panel says the
/// restart didn't happen, no open dialog can still start one.
pub const ANSWER_WITHIN: Duration = Duration::from_secs(150);

/// The latest request asked about: only its dialog's Yes counts.
static LATEST_ASK: AtomicU64 = AtomicU64::new(0);
/// The thread showing the open dialog, 0 when none (Windows).
static DIALOG_THREAD: AtomicU32 = AtomicU32::new(0);

const DIALOG_TITLE: &str = "Upgrade AgentMux";

/// The supervisor's end of srv's upgrade requests. One per process: it lasts
/// across srv respawns, since the reader of each new srv sends here too.
pub fn requests() -> mpsc::UnboundedReceiver<UpgradeRequest> {
    let (tx, rx) = mpsc::unbounded_channel();
    *REQUESTS.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx);
    rx
}

/// From the srv stderr reader.
pub fn forward(req: UpgradeRequest) {
    if let Some(tx) = REQUESTS.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        let _ = tx.send(req);
    }
}

/// `major.minor.patch`, or `None`.
fn version_core(v: &str) -> Option<(u64, u64, u64)> {
    let core = v.trim().trim_start_matches('v').split(['+', '-']).next()?;
    let mut parts = core.split('.').map(|p| p.parse::<u64>().ok());
    let version = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(version)
}

/// Whether `dir` holds Inno Setup's uninstaller (`unins000.exe`, …): a
/// `setup.exe` install.
fn has_uninstaller(dir: &Path) -> bool {
    std::fs::read_dir(dir).is_ok_and(|entries| {
        entries.filter_map(|e| e.ok()).any(|e| {
            let name = e.file_name().to_string_lossy().to_ascii_lowercase();
            name.starts_with("unins") && name.ends_with(".exe")
        })
    })
}

/// A portable folder. `setup.exe` installs the same `runtime/`, marker and
/// all, so a folder with Inno Setup's uninstaller isn't one.
fn is_portable(folder: &Path) -> bool {
    folder.join("runtime").join("agentmux-portable.marker").is_file() && !has_uninstaller(folder)
}

/// What a confirmed upgrade restarts into.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Staged {
    /// A portable's new folder: start its launcher.
    Launcher(PathBuf),
    /// A `setup.exe` install: run this installer, then start `own_dir`
    /// again. The installer keeps the install's own mode (all users or the
    /// current one): Inno Setup's default `UsePreviousPrivileges` reads it
    /// from the previous install, which the folder alone can't tell.
    /// `sha256` is the release's digest, checked again as it runs.
    Installer { installer: PathBuf, sha256: String, own_dir: PathBuf },
}

impl std::fmt::Display for Staged {
    /// The path the user would recognise: the new launcher, or the installer.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Staged::Launcher(launcher) => write!(f, "{}", launcher.display()),
            Staged::Installer { installer, .. } => write!(f, "{}", installer.display()),
        }
    }
}

/// 64 lowercase hex digits: all that is ever put in the script.
fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// The file's SHA-256 in lowercase hex, or `None` if it can't be read.
fn file_sha256(path: &Path) -> Option<String> {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    std::io::copy(&mut std::fs::File::open(path).ok()?, &mut hasher).ok()?;
    Some(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// What to restart into, if `req` names a newer release than `own_version`
/// that fits this install (in `own_dir`, with its data in `data_dir`): a
/// portable of exactly that version beside a portable, or that version's
/// installer in this version's own `updates/<v>/` on a `setup.exe` install.
pub fn staged(req: &UpgradeRequest, own_dir: &Path, data_dir: &Path, own_version: &str) -> Result<Staged, String> {
    match (version_core(&req.version), version_core(own_version)) {
        (Some(new), Some(own)) if new > own => {}
        _ => return Err(format!("{} isn't newer than {own_version}", req.version)),
    }
    match &req.target {
        UpgradeTarget::Launcher(launcher) => {
            let named_right = launcher.file_name().is_some_and(|n| n.eq_ignore_ascii_case("agentmux.exe"));
            if !launcher.is_absolute() || !named_right || !launcher.is_file() {
                return Err(format!("{} isn't an AgentMux launcher", launcher.display()));
            }
            let folder = launcher.parent().ok_or("the launcher has no folder")?;
            if !is_portable(own_dir) {
                return Err("this AgentMux isn't a portable install".into());
            }
            if !is_portable(folder) || !folder.join("runtime").join(format!("agentmux-{}.exe", req.version)).is_file() {
                return Err(format!("{} isn't an AgentMux {} portable", folder.display(), req.version));
            }
            if folder == own_dir {
                return Err("that is the running AgentMux".into());
            }
            Ok(Staged::Launcher(launcher.clone()))
        }
        UpgradeTarget::Installer { installer, sha256 } => {
            let name = format!("AgentMux-{}-x64-setup.exe", req.version);
            let named_right = installer.file_name().is_some_and(|n| n.eq_ignore_ascii_case(name.as_str()));
            let in_updates = installer.parent() == Some(data_dir.join("updates").join(&req.version).as_path());
            if !installer.is_absolute() || !named_right || !in_updates || !installer.is_file() {
                return Err(format!("{} isn't the AgentMux {} installer AgentMux downloaded", installer.display(), req.version));
            }
            if !has_uninstaller(own_dir) || !own_dir.join("agentmux.exe").is_file() {
                return Err("this AgentMux isn't a setup.exe install".into());
            }
            // Before asking: a file changed since srv checked it fails here,
            // with a reason. The check that binds is the script's
            // ([`installer_script`]), on the file as it runs.
            if !is_sha256_hex(sha256) || file_sha256(installer).as_deref() != Some(sha256.as_str()) {
                return Err(format!("{} isn't the AgentMux {} release; download it again", installer.display(), req.version));
            }
            Ok(Staged::Installer { installer: installer.clone(), sha256: sha256.clone(), own_dir: own_dir.to_path_buf() })
        }
    }
}

/// Ask the user on a thread of its own, and call `on_yes` only for an
/// explicit Yes that still counts ([`answer_counts`]): one given within
/// [`ANSWER_WITHIN`], to the latest request. One dialog at a time: a newer
/// request closes the open one, and a dialog closes itself (as No) once its
/// time is up. A Yes that slips past either is told it expired.
pub fn ask_then(version: String, staged: Staged, on_yes: impl FnOnce(Staged) + Send + 'static) {
    let ask = LATEST_ASK.fetch_add(1, Ordering::SeqCst) + 1;
    dismiss(DIALOG_THREAD.load(Ordering::SeqCst));
    std::thread::spawn(move || {
        let opened = Instant::now();
        let thread = current_thread();
        DIALOG_THREAD.store(thread, Ordering::SeqCst);
        std::thread::spawn(move || {
            std::thread::sleep(ANSWER_WITHIN);
            if DIALOG_THREAD.load(Ordering::SeqCst) == thread {
                dismiss(thread);
            }
        });
        let yes = confirm(&version, &staged);
        let _ = DIALOG_THREAD.compare_exchange(thread, 0, Ordering::SeqCst, Ordering::SeqCst);
        if !yes {
            return;
        }
        if answer_counts(ask, LATEST_ASK.load(Ordering::SeqCst), opened.elapsed()) {
            on_yes(staged);
        } else {
            crate::log(&format!("upgrade: a Yes {}s after asking, or to an older request, was ignored", opened.elapsed().as_secs()));
            crate::show_fatal_dialog(
                "AgentMux upgrade",
                "This upgrade request expired, so AgentMux didn't restart.\n\nStart it again from the version panel.",
            );
        }
    });
}

/// Whether a Yes to request `ask`, given `waited` after its dialog opened,
/// still counts when `latest` is the newest request.
fn answer_counts(ask: u64, latest: u64, waited: Duration) -> bool {
    ask == latest && waited < ANSWER_WITHIN
}

#[cfg(target_os = "windows")]
fn current_thread() -> u32 {
    unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() }
}

#[cfg(not(target_os = "windows"))]
fn current_thread() -> u32 {
    0
}

/// Close the dialog shown on `thread` by clicking its No button.
#[cfg(target_os = "windows")]
fn dismiss(thread: u32) {
    use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumThreadWindows, GetDlgItem, GetWindowTextW, SendMessageW, BM_CLICK, IDNO,
    };
    unsafe extern "system" fn click_no(hwnd: HWND, _: LPARAM) -> BOOL {
        let mut title = [0u16; 64];
        let len = GetWindowTextW(hwnd, title.as_mut_ptr(), title.len() as i32).max(0) as usize;
        if String::from_utf16_lossy(&title[..len]) == DIALOG_TITLE {
            let no = GetDlgItem(hwnd, IDNO);
            if !no.is_null() {
                SendMessageW(no, BM_CLICK, 0, 0);
            }
            return 0;
        }
        1
    }
    if thread != 0 {
        unsafe { EnumThreadWindows(thread, Some(click_no), 0) };
    }
}

#[cfg(not(target_os = "windows"))]
fn dismiss(_thread: u32) {}

#[cfg(target_os = "windows")]
fn confirm(version: &str, staged: &Staged) -> bool {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, IDYES, MB_DEFBUTTON2, MB_ICONQUESTION, MB_SETFOREGROUND, MB_TOPMOST, MB_YESNO,
    };
    let body = match staged {
        Staged::Launcher(launcher) => format!(
            "Restart AgentMux to upgrade to {version}?\n\n\
             Every window closes and every agent stops, then AgentMux {version} opens \
             from:\n{}\n\nYour tabs come back and agents resume their conversations.",
            launcher.parent().unwrap_or(launcher).display()
        ),
        Staged::Installer { .. } => format!(
            "Restart AgentMux to upgrade to {version}?\n\n\
             Every window closes and every agent stops, then the AgentMux {version} installer \
             runs (Windows may ask for permission, if AgentMux is installed for all users) \
             and AgentMux opens again.\n\nYour tabs come back and agents resume their conversations."
        ),
    };
    let wide = |s: &str| std::ffi::OsStr::new(s).encode_wide().chain(Some(0)).collect::<Vec<u16>>();
    let (title, body) = (wide(DIALOG_TITLE), wide(&body));
    let answer = unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            body.as_ptr(),
            title.as_ptr(),
            MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2 | MB_SETFOREGROUND | MB_TOPMOST,
        )
    };
    answer == IDYES
}

#[cfg(not(target_os = "windows"))]
fn confirm(_version: &str, _staged: &Staged) -> bool {
    // Only the Windows portable restarts itself so far (phase 3b).
    false
}

/// Whether a restart confirmed at `at` still stands.
pub fn still_confirmed(at: Instant) -> bool {
    at.elapsed() < CONFIRMED_FOR
}

/// `path` as a PowerShell single-quoted string: nothing in it is expanded
/// (no `$`, no `%VAR%`). Every character PowerShell takes as a single quote
/// is written twice: `'` and the typographic ones (U+2018 to U+201B), which
/// a user name can hold.
fn ps_quote(path: &Path) -> String {
    let mut out = String::from("'");
    for c in path.display().to_string().chars() {
        if matches!(c, '\'' | '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}') {
            out.push(c);
        }
        out.push(c);
    }
    out.push('\'');
    out
}

/// The hidden PowerShell that upgrades a `setup.exe` install: wait a few
/// seconds for this launcher to be gone (its files are replaced), run the
/// installer silently, then start AgentMux again whatever the installer did,
/// so a failed or declined install leaves the old version running, not none.
///
/// The installer sits in the user's own data folder, which any process of
/// theirs can write, and on an all-users install Windows runs it elevated.
/// So the script opens it allowing others only to read, which blocks any
/// write, rename or delete; hashes it through that handle; and runs it only
/// if it is `sha256`, holding the handle until the installer exits. The file
/// that runs is the file that was checked.
fn installer_script(installer: &Path, sha256: &str, own_dir: &Path) -> String {
    debug_assert!(is_sha256_hex(sha256));
    let installer = ps_quote(installer);
    format!(
        "Start-Sleep -Seconds 3; \
         $f = $null; \
         try {{ \
         $f = [System.IO.File]::Open({installer}, 'Open', 'Read', 'Read'); \
         $h = [System.BitConverter]::ToString([System.Security.Cryptography.SHA256]::Create().ComputeHash($f)).Replace('-', ''); \
         if ($h -eq '{sha256}') {{ \
         Start-Process -FilePath {installer} -ArgumentList '/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART','/SP-' -Wait \
         }} \
         }} catch {{ }} finally {{ if ($f) {{ $f.Dispose() }} }}; \
         Start-Process -FilePath {}",
        ps_quote(&own_dir.join("agentmux.exe"))
    )
}

/// `-EncodedCommand`'s form of `script` (base64 of UTF-16LE): the paths in
/// it reach PowerShell as they are, through no shell's parsing.
fn encoded_command(script: &str) -> String {
    use base64::Engine;
    let utf16: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    base64::engine::general_purpose::STANDARD.encode(utf16)
}

/// Windows PowerShell by its full path, never whatever is first on `PATH`.
fn powershell_exe() -> PathBuf {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    PathBuf::from(root).join("System32").join("WindowsPowerShell").join("v1.0").join("powershell.exe")
}

/// Restart into `staged`, detached, without this AgentMux's own environment:
/// `AGENTMUX` would make the next launcher think it was started from a pane,
/// and the `AGENTMUX_*` this launcher set on itself (background service,
/// tray) would override the next one's settings.
pub fn relaunch(staged: &Staged) -> std::io::Result<()> {
    use agentmux_common::win32::NoWindow;
    let mut cmd = match staged {
        Staged::Launcher(launcher) => {
            let mut c = std::process::Command::new(launcher);
            if let Some(folder) = launcher.parent() {
                c.current_dir(folder);
            }
            c
        }
        Staged::Installer { installer, sha256, own_dir } => {
            let mut c = std::process::Command::new(powershell_exe());
            c.args(["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-EncodedCommand"])
                .arg(encoded_command(&installer_script(installer, sha256, own_dir)));
            c
        }
    };
    for (key, _) in std::env::vars_os() {
        let k = key.to_string_lossy();
        if k == "AGENTMUX" || k.starts_with("AGENTMUX_") {
            cmd.env_remove(&key);
        }
    }
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .no_window()
        .spawn()
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn portable(dir: &Path, version: &str) -> PathBuf {
        std::fs::create_dir_all(dir.join("runtime")).unwrap();
        std::fs::write(dir.join("runtime/agentmux-portable.marker"), b"").unwrap();
        std::fs::write(dir.join(format!("runtime/agentmux-{version}.exe")), b"").unwrap();
        std::fs::write(dir.join("agentmux.exe"), b"").unwrap();
        dir.join("agentmux.exe")
    }

    fn req(version: &str, launcher: &Path) -> UpgradeRequest {
        UpgradeRequest { version: version.into(), target: UpgradeTarget::Launcher(launcher.to_path_buf()) }
    }

    /// SHA-256 of the empty file, which the tests' installers are.
    const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    fn setup_req(version: &str, installer: &Path) -> UpgradeRequest {
        let target = UpgradeTarget::Installer { installer: installer.to_path_buf(), sha256: EMPTY_SHA256.into() };
        UpgradeRequest { version: version.into(), target }
    }

    #[test]
    fn only_a_newer_portable_beside_a_portable_is_restarted_into() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let own = root.path().join("agentmux-0.59.26-x64-portable");
        portable(&own, "0.59.26");
        let new = portable(&root.path().join("agentmux-0.59.27-x64-portable"), "0.59.27");
        assert_eq!(staged(&req("0.59.27", &new), &own, &data, "0.59.26").unwrap(), Staged::Launcher(new.clone()));

        // Not newer, or not the version it claims.
        assert!(staged(&req("0.59.27", &new), &own, &data, "0.59.27").is_err());
        assert!(staged(&req("0.59.28", &new), &own, &data, "0.59.26").is_err());
        // Not a launcher, or not absolute.
        let other = root.path().join("agentmux-0.59.27-x64-portable/runtime/agentmux-0.59.27.exe");
        assert!(staged(&req("0.59.27", &other), &own, &data, "0.59.26").is_err());
        assert!(staged(&req("0.59.27", Path::new("agentmux.exe")), &own, &data, "0.59.26").is_err());
        // This AgentMux isn't a portable, including a setup.exe install,
        // which has the portable's runtime/ and marker too.
        let installed = root.path().join("Programs/AgentMux");
        portable(&installed, "0.59.26");
        std::fs::write(installed.join("unins000.exe"), b"").unwrap();
        assert!(staged(&req("0.59.27", &new), &installed, &data, "0.59.26").is_err());
    }

    #[test]
    fn only_that_versions_installer_in_this_versions_updates_runs_on_a_setup_install() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let own = root.path().join("Programs/AgentMux");
        portable(&own, "0.59.26");
        std::fs::write(own.join("unins000.exe"), b"").unwrap();
        let updates = data.join("updates/0.59.27");
        std::fs::create_dir_all(&updates).unwrap();
        let installer = updates.join("AgentMux-0.59.27-x64-setup.exe");
        std::fs::write(&installer, b"").unwrap();
        let Staged::Installer { installer: got, sha256, own_dir } =
            staged(&setup_req("0.59.27", &installer), &own, &data, "0.59.26").unwrap()
        else {
            panic!("expected the installer")
        };
        assert_eq!((got, sha256.as_str(), own_dir), (installer.clone(), EMPTY_SHA256, own.clone()));

        // Changed since srv checked it, or no proper digest to check against.
        std::fs::write(&installer, b"not the release").unwrap();
        let err = staged(&setup_req("0.59.27", &installer), &own, &data, "0.59.26").unwrap_err();
        assert!(err.contains("download it again"), "{err}");
        std::fs::write(&installer, b"").unwrap();
        let mut upper = setup_req("0.59.27", &installer);
        upper.target = UpgradeTarget::Installer { installer: installer.clone(), sha256: EMPTY_SHA256.to_uppercase() };
        assert!(staged(&upper, &own, &data, "0.59.26").is_err());

        // Not that version's name, not in this version's updates, or older.
        let renamed = updates.join("setup.exe");
        std::fs::write(&renamed, b"").unwrap();
        assert!(staged(&setup_req("0.59.27", &renamed), &own, &data, "0.59.26").is_err());
        let elsewhere = root.path().join("AgentMux-0.59.27-x64-setup.exe");
        std::fs::write(&elsewhere, b"").unwrap();
        assert!(staged(&setup_req("0.59.27", &elsewhere), &own, &data, "0.59.26").is_err());
        assert!(staged(&setup_req("0.59.27", &installer), &own, &data, "0.59.27").is_err());
        // A portable never runs an installer.
        let portable_dir = root.path().join("agentmux-0.59.26-x64-portable");
        portable(&portable_dir, "0.59.26");
        assert!(staged(&setup_req("0.59.27", &installer), &portable_dir, &data, "0.59.26").is_err());
    }

    #[test]
    fn the_installer_script_runs_silently_then_always_starts_agentmux_again() {
        let path = "C:/Users/A %TEMP% $x/AgentMux-0.59.27-x64-setup.exe";
        let script = installer_script(Path::new(path), EMPTY_SHA256, Path::new("C:/Apps/O'Neil"));
        assert!(script.starts_with("Start-Sleep -Seconds 3; "), "{script}");
        // Opened so nothing else can change it, hashed through that handle,
        // and run only on a match, before the handle is let go.
        let open = script.find(&format!("[System.IO.File]::Open('{path}', 'Open', 'Read', 'Read')")).expect(&script);
        let check = script.find(&format!("if ($h -eq '{EMPTY_SHA256}')")).expect(&script);
        // Single-quoted: neither %TEMP% nor $x is expanded; a ' is doubled.
        let run = script
            .find(&format!("Start-Process -FilePath '{path}' -ArgumentList '/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART','/SP-' -Wait"))
            .expect(&script);
        let release = script.find("$f.Dispose()").expect(&script);
        assert!(open < check && check < run && run < release, "{script}");
        // AgentMux starts again whatever happened, last.
        let restart = script.rfind("Start-Process -FilePath 'C:/Apps/O''Neil").expect(&script);
        assert!(restart > release, "{script}");
        // Typographic quotes close a PowerShell string too: doubled as well.
        assert_eq!(ps_quote(Path::new("C:/Users/O\u{2019}Neil")), "'C:/Users/O\u{2019}\u{2019}Neil'");
        assert_eq!(ps_quote(Path::new("a\u{2018}b\u{201A}c\u{201B}")), "'a\u{2018}\u{2018}b\u{201A}\u{201A}c\u{201B}\u{201B}'");
        // The install keeps its own mode (UsePreviousPrivileges).
        assert!(!script.contains("ALLUSERS") && !script.contains("CURRENTUSER"));
    }

    #[test]
    fn the_script_reaches_powershell_as_utf16_base64() {
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD.decode(encoded_command("Start-Process 'é'")).unwrap();
        let units: Vec<u16> = bytes.chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        assert_eq!(String::from_utf16(&units).unwrap(), "Start-Process 'é'");
    }

    #[test]
    fn a_late_or_superseded_yes_does_not_count() {
        assert!(answer_counts(3, 3, Duration::from_secs(10)));
        assert!(!answer_counts(3, 3, ANSWER_WITHIN));
        assert!(!answer_counts(2, 3, Duration::from_secs(10)));
        // The dialog must stop counting before srv gives up (3 minutes).
        assert!(ANSWER_WITHIN < Duration::from_secs(3 * 60));
    }

    #[test]
    fn a_confirmation_lapses() {
        assert!(still_confirmed(Instant::now()));
        if let Some(long_ago) = Instant::now().checked_sub(CONFIRMED_FOR + Duration::from_secs(1)) {
            assert!(!still_confirmed(long_ago));
        }
    }
}
