// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! "Restart now" for a Microsoft Store update that is downloaded and staged (task #2514): close AgentMux and
//! open it again on the new version.
//!
//! The Store does not close a running packaged desktop app to swap its package: it stages the new package, marks
//! the install "blocked on the user" and waits until nothing of the old package runs
//! (Area54, 2026-11-12: `Working -> BlockedOnUser`, the old package still registered). Windows then registers the
//! new package, but does not open the app again: `RegisterApplicationRestart` (the launcher's) only fires when
//! Windows itself closes the app. So something outside the package has to open it, once the swap is done.
//!
//! That something is a one-shot Scheduled Task for the person, run by the Task Scheduler and so outside the
//! package: a process started by AgentMux, wherever its exe lives, belongs to the package and would keep the old one
//! in use, which is the one thing that must not happen. The task
//! - takes nothing from anyone: the script is built here from two checked values (this package's family name,
//!   which Windows gives the running app, and this build's version), and the task is registered with it as its
//!   one fixed action, where only the person can change it;
//! - waits up to ten minutes for a newer AgentMux package to be registered, then opens AgentMux by its package
//!   identity (`shell:AppsFolder\<family>!AgentMux`), never by an exe path;
//! - starts AgentMux exactly once, whether or not the package changed (a swap that did not happen is told at the
//!   next start by the marker, `store_restart`), removes itself, and ends. Its limit is fifteen minutes.
//!
//! NOT run against a packaged install by whoever wrote it: the script text and its checks are unit-tested, the
//! Windows calls are type-checked only.

use std::time::Duration;

/// The Scheduled Task's name.
const TASK: &str = "AgentMux Store reopen";
/// The application id in the package's manifest (`<Application Id="AgentMux">`).
const APP_ID: &str = "AgentMux";
/// How long the task waits for the new package to be registered.
const WAIT_MINUTES: u32 = 10;
/// The task is stopped by the Task Scheduler after this long whatever it is doing.
const LIMIT_MINUTES: u32 = 15;

/// A package family name: `Name_<13 characters>`, where the name is the manifest's (`Publisher.App`).
fn valid_family(family: &str) -> bool {
    let Some((name, id)) = family.split_once('_') else { return false };
    !name.is_empty()
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
        && id.len() == 13
        && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
}

/// `major.minor.build`.
fn valid_version(v: &str) -> bool {
    let parts: Vec<&str> = v.split('.').collect();
    parts.len() == 3 && parts.iter().all(|p| !p.is_empty() && p.len() <= 6 && p.chars().all(|c| c.is_ascii_digit()))
}

/// What the task runs, outside the package.
fn wait_script(family: &str, from: &str) -> String {
    let name = family.split('_').next().unwrap_or_default();
    format!(
        "$ErrorActionPreference = 'SilentlyContinue'\n\
         $from = [version]'{from}.0'\n\
         $until = (Get-Date).AddMinutes({WAIT_MINUTES})\n\
         while ((Get-Date) -lt $until) {{\n\
         \x20 $p = Get-AppxPackage -Name '{name}' | Where-Object {{ [version]$_.Version -gt $from }} | Select-Object -First 1\n\
         \x20 if ($p) {{ break }}\n\
         \x20 Start-Sleep -Seconds 3\n\
         }}\n\
         Start-Process 'shell:AppsFolder\\{family}!{APP_ID}'\n\
         Unregister-ScheduledTask -TaskName '{TASK}' -Confirm:$false\n"
    )
}

/// UTF-16LE base64, as PowerShell's `-EncodedCommand` takes it.
fn encoded(script: &str) -> String {
    use base64::Engine;
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// What registers the task and starts it now, run for a moment by AgentMux itself.
fn register_script(family: &str, from: &str) -> String {
    let wait = encoded(&wait_script(family, from));
    format!(
        "$ErrorActionPreference = 'Stop'\n\
         $ps = Join-Path $env:SystemRoot 'System32\\WindowsPowerShell\\v1.0\\powershell.exe'\n\
         $act = New-ScheduledTaskAction -Execute $ps -Argument '-NoProfile -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -EncodedCommand {wait}'\n\
         $who = [Security.Principal.WindowsIdentity]::GetCurrent().Name\n\
         $prin = New-ScheduledTaskPrincipal -UserId $who -LogonType Interactive -RunLevel Limited\n\
         $set = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -StartWhenAvailable -ExecutionTimeLimit (New-TimeSpan -Minutes {LIMIT_MINUTES})\n\
         Register-ScheduledTask -TaskName '{TASK}' -Action $act -Principal $prin -Settings $set -Force | Out-Null\n\
         Start-ScheduledTask -TaskName '{TASK}'\n"
    )
}

/// The command line that registers and starts the task, for these two values; an error when either is not what
/// Windows gives (nothing else reaches the script).
pub fn registration(family: &str, from: &str) -> Result<String, String> {
    if !valid_family(family) {
        return Err("the package's family name is not one Windows gives".into());
    }
    if !valid_version(from) {
        return Err("this build's version is not major.minor.build".into());
    }
    Ok(encoded(&register_script(family, from)))
}

/// This package's family name, from Windows.
#[cfg(target_os = "windows")]
fn family_name() -> Result<String, String> {
    let family = windows::ApplicationModel::Package::Current()
        .and_then(|p| p.Id())
        .and_then(|id| id.FamilyName())
        .map_err(|e| format!("this is not a packaged install ({e})"))?;
    Ok(family.to_string())
}

/// Register the reopen. Blocking (a PowerShell runs for a moment): call it off the async workers.
#[cfg(target_os = "windows")]
pub fn schedule(from: &str) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    use std::time::Instant;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let family = super::store_update::on_winrt_thread("store-family", family_name)??;
    let command = registration(&family, from)?;
    let exe = std::path::Path::new(&std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into()))
        .join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
    let mut child = std::process::Command::new(exe)
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-EncodedCommand", &command])
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("couldn't start PowerShell: {e}"))?;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => return Err(format!("scheduling it failed (exit {})", status.code().unwrap_or(-1))),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            Ok(None) => {
                let _ = child.kill();
                return Err("scheduling it took too long".into());
            }
            Err(e) => return Err(format!("couldn't wait for PowerShell: {e}")),
        }
    }
}

/// Only Windows has a Store.
#[cfg(not(target_os = "windows"))]
pub fn schedule(_from: &str) -> Result<(), String> {
    Err("the Microsoft Store is Windows only".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAMILY: &str = "AgentMux.AgentMux_vqr1k32tkfk4y";

    fn decode(b64: &str) -> String {
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD.decode(b64).unwrap();
        let units: Vec<u16> = bytes.chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        String::from_utf16(&units).unwrap()
    }

    #[test]
    fn only_values_windows_gives_get_into_the_script() {
        assert!(valid_family(FAMILY));
        assert!(valid_version("0.60.24"));
        for bad in ["", "AgentMux", "AgentMux_short", "A'gent_vqr1k32tkfk4y", "Agent Mux_vqr1k32tkfk4y", "Agent;Mux_vqr1k32tkfk4y", "AgentMux_VQR1K32TKFK4Y", "AgentMux.AgentMux_vqr1k32tkfk4y'; calc; '"] {
            assert!(!valid_family(bad), "{bad}");
            assert!(registration(bad, "0.60.24").is_err(), "{bad}");
        }
        for bad in ["", "0.60", "0.60.24.1", "0.60.x", "0.60.24'; calc; '", "0.60.-1"] {
            assert!(!valid_version(bad), "{bad}");
            assert!(registration(FAMILY, bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_task_opens_agentmux_by_its_package_identity_and_never_by_a_path() {
        let script = wait_script(FAMILY, "0.60.24");
        assert!(script.contains("Start-Process 'shell:AppsFolder\\AgentMux.AgentMux_vqr1k32tkfk4y!AgentMux'"), "{script}");
        assert!(!script.to_lowercase().contains(".exe"), "no exe path: {script}");
        assert!(!script.contains("WindowsApps"), "{script}");
    }

    #[test]
    fn the_task_waits_for_a_newer_package_then_starts_once_and_removes_itself() {
        let script = wait_script(FAMILY, "0.60.24");
        assert!(script.contains("[version]'0.60.24.0'"), "{script}");
        assert!(script.contains("Get-AppxPackage -Name 'AgentMux.AgentMux'"), "{script}");
        assert!(script.contains("AddMinutes(10)"), "{script}");
        assert_eq!(script.matches("Start-Process").count(), 1, "one start, newer package or not");
        assert!(script.contains("Unregister-ScheduledTask -TaskName 'AgentMux Store reopen'"), "{script}");
    }

    #[test]
    fn the_registration_carries_the_wait_script_as_the_tasks_one_fixed_action() {
        let command = registration(FAMILY, "0.60.24").unwrap();
        let register = decode(&command);
        assert!(register.contains("Register-ScheduledTask -TaskName 'AgentMux Store reopen'"), "{register}");
        assert!(register.contains("Start-ScheduledTask -TaskName 'AgentMux Store reopen'"), "{register}");
        assert!(register.contains("-LogonType Interactive -RunLevel Limited"), "the person's own session, no elevation: {register}");
        assert!(register.contains("New-TimeSpan -Minutes 15"), "{register}");
        let wait = register.split("-EncodedCommand ").nth(1).and_then(|r| r.split('\'').next()).unwrap();
        assert_eq!(decode(wait), wait_script(FAMILY, "0.60.24"));
    }

    #[test]
    fn the_encoding_is_utf16_little_endian_base64() {
        assert_eq!(encoded("A"), "QQA=");
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn off_windows_nothing_is_scheduled() {
        assert!(schedule("0.60.24").is_err());
    }
}
