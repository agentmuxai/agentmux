// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Puts the build slot where agents' shells find it, so only one heavy build or test runs
//! at a time on a machine: `docs/specs/SPEC_BUILD_SLOT_ENFORCEMENT_2026_10_20.md`.
//!
//! Two kinds of file are installed, both taken from `scripts/build-slot/`:
//!
//! - `~/.agentmux/shared/locks/build-slot.sh`, the lock itself. Every AgentMux channel on the
//!   machine shares it, so it is only replaced by a newer version (`# build-slot-version: N`),
//!   never by an older channel's copy.
//! - `cargo` and `cargo.cmd` in `~/.agentmux/tools/bin`, the user tools directory that agents'
//!   shells put ahead of the system PATH (`tool_store::tools_path`). They send the heavy cargo
//!   subcommands through the lock. A file at that path that does not carry the shim's marker is
//!   somebody's own and is left alone.
//!
//! Installing never fails startup: the caller logs an error and goes on. The shims fail open
//! too, so a missing or broken lock script costs nothing but the protection.

use std::io;
use std::path::{Path, PathBuf};

const SCRIPT: &str = include_str!("../../../scripts/build-slot/build-slot.sh");
const SHIM_SH: &str = include_str!("../../../scripts/build-slot/cargo");
const SHIM_CMD: &str = include_str!("../../../scripts/build-slot/cargo.cmd");

const VERSION_PREFIX: &str = "# build-slot-version:";
/// Both shims carry this text; a file without it is not ours to overwrite.
const SHIM_MARKER: &str = "agentmux-build-slot-shim";

/// `~/.agentmux/shared/locks/build-slot.sh` under `home`.
pub fn script_path(home: &Path) -> PathBuf {
    home.join(".agentmux").join("shared").join("locks").join("build-slot.sh")
}

/// `~/.agentmux/tools/bin` under `home`: the shims' directory.
pub fn shim_dir(home: &Path) -> PathBuf {
    home.join(".agentmux").join("tools").join("bin")
}

/// What one `install` call did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Installed {
    /// Files written, by name.
    pub written: Vec<String>,
    /// Files left alone because they are not ours, by name.
    pub foreign: Vec<String>,
}

/// Installs for the current user's home directory. `Ok` with nothing written when there is none.
pub fn install_for_current_user() -> io::Result<Installed> {
    match dirs::home_dir() {
        Some(home) => install(&home),
        None => Ok(Installed::default()),
    }
}

/// Installs the lock script and both shims under `home`, writing only what is missing or older.
pub fn install(home: &Path) -> io::Result<Installed> {
    let mut done = Installed::default();

    let script = lf(SCRIPT);
    let script_dest = script_path(home);
    let installed_version = std::fs::read_to_string(&script_dest).ok().and_then(|t| version_of(&t));
    let embedded_version = version_of(&script);
    if !script_dest.exists() || installed_version.is_none() || embedded_version > installed_version {
        write_executable(&script_dest, &script)?;
        done.written.push("build-slot.sh".to_string());
    }

    let dir = shim_dir(home);
    for (name, text) in [("cargo", lf(SHIM_SH)), ("cargo.cmd", crlf(SHIM_CMD))] {
        let dest = dir.join(name);
        match std::fs::read_to_string(&dest) {
            Ok(existing) if existing == text => {}
            Ok(existing) if existing.contains(SHIM_MARKER) => {
                write_executable(&dest, &text)?;
                done.written.push(name.to_string());
            }
            Ok(_) => done.foreign.push(name.to_string()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                write_executable(&dest, &text)?;
                done.written.push(name.to_string());
            }
            // Not valid UTF-8 (a real binary named `cargo`): somebody else's.
            Err(e) if e.kind() == io::ErrorKind::InvalidData => done.foreign.push(name.to_string()),
            Err(e) => return Err(e),
        }
    }
    Ok(done)
}

/// The `# build-slot-version: N` line's number.
fn version_of(text: &str) -> Option<u32> {
    text.lines()
        .find_map(|l| l.strip_prefix(VERSION_PREFIX))
        .and_then(|v| v.trim().parse().ok())
}

/// A shell script must be LF whatever the checkout's line endings were.
fn lf(text: &str) -> String {
    text.replace("\r\n", "\n")
}

/// A batch file is CRLF whatever the checkout's line endings were.
fn crlf(text: &str) -> String {
    lf(text).replace('\n', "\r\n")
}

/// Writes through a temporary name in the same directory, then renames over `dest`, so a
/// shell starting a shim at that moment sees the old file or the new one, never half of one.
fn write_executable(dest: &Path, text: &str) -> io::Result<()> {
    let dir = dest.parent().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no parent"))?;
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(".{}.{}.tmp", dest.file_name().and_then(|n| n.to_str()).unwrap_or("f"), std::process::id()));
    std::fs::write(&tmp, text)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
    }
    std::fs::rename(&tmp, dest).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn the_embedded_files_are_what_the_installer_expects() {
        assert!(version_of(SCRIPT).is_some(), "build-slot.sh needs a `# build-slot-version: N` line");
        assert!(SHIM_SH.contains(SHIM_MARKER), "the cargo shim must carry its marker");
        assert!(SHIM_CMD.contains(SHIM_MARKER), "the cargo.cmd shim must carry its marker");
        assert!(!lf(SCRIPT).contains('\r') && !lf(SHIM_SH).contains('\r'));
        assert!(crlf(SHIM_CMD).contains("\r\n") && !crlf(SHIM_CMD).replace("\r\n", "").contains('\n'));
    }

    #[test]
    fn an_empty_home_gets_the_script_and_both_shims_once() {
        let h = home();
        let first = install(h.path()).unwrap();
        assert_eq!(first.written, vec!["build-slot.sh", "cargo", "cargo.cmd"]);
        assert!(first.foreign.is_empty());
        assert_eq!(install(h.path()).unwrap(), Installed::default(), "the second call finds nothing to do");
        assert_eq!(std::fs::read_to_string(shim_dir(h.path()).join("cargo.cmd")).unwrap(), crlf(SHIM_CMD));
    }

    #[test]
    fn a_cargo_that_is_not_the_shim_is_left_alone() {
        let h = home();
        let dir = shim_dir(h.path());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("cargo"), "#!/bin/sh\necho mine\n").unwrap();
        let done = install(h.path()).unwrap();
        assert_eq!(done.foreign, vec!["cargo"]);
        assert_eq!(std::fs::read_to_string(dir.join("cargo")).unwrap(), "#!/bin/sh\necho mine\n");
        assert!(done.written.contains(&"cargo.cmd".to_string()), "the other shim is still installed");
    }

    #[test]
    fn an_older_shim_is_replaced_and_a_newer_script_is_kept() {
        let h = home();
        install(h.path()).unwrap();
        let dir = shim_dir(h.path());
        std::fs::write(dir.join("cargo"), "#!/bin/sh\n# agentmux-build-slot-shim v0\n").unwrap();
        let newer = "#!/bin/bash\n# build-slot-version: 999\necho newer channel\n";
        std::fs::write(script_path(h.path()), newer).unwrap();
        let done = install(h.path()).unwrap();
        assert_eq!(done.written, vec!["cargo"]);
        assert_eq!(std::fs::read_to_string(script_path(h.path())).unwrap(), newer, "no downgrade");
    }

    #[test]
    fn an_unversioned_or_older_script_is_replaced() {
        let h = home();
        install(h.path()).unwrap();
        std::fs::write(script_path(h.path()), "#!/bin/bash\n# written by hand, no version\n").unwrap();
        assert_eq!(install(h.path()).unwrap().written, vec!["build-slot.sh"]);
        std::fs::write(script_path(h.path()), "#!/bin/bash\n# build-slot-version: 0\n").unwrap();
        assert_eq!(install(h.path()).unwrap().written, vec!["build-slot.sh"]);
    }

    // The shim's own behaviour, run with a stand-in cargo and a stand-in lock script.
    // Skipped where there is no bash (a Windows machine without Git for Windows).
    fn bash() -> Option<PathBuf> {
        if cfg!(windows) {
            let git = std::env::var_os("ProgramFiles").map(|p| PathBuf::from(p).join("Git").join("bin").join("bash.exe"))?;
            git.exists().then_some(git)
        } else {
            Some(PathBuf::from("bash"))
        }
    }

    /// What the shim ran, given `args`, for a home with or without the lock script.
    fn run_shim(with_slot: bool, off: bool, args: &[&str]) -> Option<String> {
        let bash = bash()?;
        let h = home();
        let real_dir = h.path().join("real");
        let shim = shim_dir(h.path());
        std::fs::create_dir_all(&real_dir).unwrap();
        std::fs::create_dir_all(&shim).unwrap();
        let real = real_dir.join("cargo");
        std::fs::write(&real, "#!/usr/bin/env bash\necho \"REAL $*\"\n").unwrap();
        std::fs::write(shim.join("cargo"), lf(SHIM_SH)).unwrap();
        if with_slot {
            std::fs::create_dir_all(script_path(h.path()).parent().unwrap()).unwrap();
            std::fs::write(script_path(h.path()), "#!/usr/bin/env bash\necho \"SLOT $*\"\n").unwrap();
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let sep = if cfg!(windows) { ";" } else { ":" };
        let path = format!("{}{sep}{}{sep}{}", shim.display(), real_dir.display(), std::env::var("PATH").unwrap_or_default());
        let mut cmd = std::process::Command::new(bash);
        cmd.arg(shim.join("cargo")).args(args).env("PATH", path).env("HOME", h.path());
        if off {
            cmd.env("AGENTMUX_BUILD_SLOT_OFF", "1");
        }
        #[cfg(windows)]
        {
            use crate::win32::NoWindow;
            cmd.no_window();
        }
        let out = cmd.output().unwrap();
        Some(String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n").trim().to_string())
    }

    fn contains_all(out: &str, parts: &[&str]) -> bool {
        parts.iter().all(|p| out.contains(p))
    }

    #[test]
    fn a_heavy_subcommand_goes_through_the_slot_and_a_light_one_does_not() {
        let Some(test) = run_shim(true, false, &["test", "-p", "x"]) else { return };
        assert!(test.starts_with("SLOT --reason cargo test -- "), "got {test:?}");
        assert!(contains_all(&test, &["real", "cargo test -p x"]), "got {test:?}");
        let nightly = run_shim(true, false, &["+nightly", "--locked", "check"]).unwrap();
        assert!(nightly.starts_with("SLOT --reason cargo check -- "), "got {nightly:?}");
        for args in [
            &["-C", "dir", "test"][..],
            &["-Z", "unstable-options", "test"][..],
            &["--config", "build.jobs=2", "test"][..],
            &["--color", "never", "test"][..],
            &["--config=build.jobs=2", "-Zunstable-options", "test"][..],
        ] {
            let out = run_shim(true, false, args).unwrap();
            assert!(out.starts_with("SLOT --reason cargo test -- "), "{args:?} should be slotted, got {out:?}");
        }
        assert_eq!(run_shim(true, false, &["metadata"]).unwrap(), "REAL metadata");
        assert_eq!(run_shim(true, false, &["-C", "dir", "metadata"]).unwrap(), "REAL -C dir metadata");
        assert_eq!(run_shim(true, false, &["run", "-p", "srv"]).unwrap(), "REAL run -p srv", "run keeps running, so it is not slotted");
        assert_eq!(run_shim(true, false, &[]).unwrap(), "REAL");
    }

    #[test]
    fn the_shim_fails_open() {
        if bash().is_none() {
            return;
        }
        assert_eq!(run_shim(false, false, &["test"]).unwrap(), "REAL test", "no lock script: cargo still runs");
        assert_eq!(run_shim(true, true, &["test"]).unwrap(), "REAL test", "AGENTMUX_BUILD_SLOT_OFF skips the slot");
    }
}
