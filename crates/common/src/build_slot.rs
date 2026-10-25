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
//! - A shim per heavy tool (`cargo`, `task`, `npm`, `npx`, `docker`, each with a `.cmd` twin) in
//!   `~/.agentmux/tools/bin`, the user tools directory that agents' shells put ahead of the system
//!   PATH (`tool_store::tools_path`). All are the same two files under different names; they send
//!   the tool's heavy subcommands through the lock. A file at that path that does not carry the
//!   shim's marker is somebody's own and is left alone.
//!
//! Installing never fails startup: the caller logs an error and goes on. The shims fail open
//! too, so a missing or broken lock script costs nothing but the protection.

use std::io;
use std::path::{Path, PathBuf};

const SCRIPT: &str = include_str!("../../../scripts/build-slot/build-slot.sh");
const SHIM_SH: &str = include_str!("../../../scripts/build-slot/shim");
const SHIM_CMD: &str = include_str!("../../../scripts/build-slot/shim.cmd");

/// The tools that get a shim, by the name the shim is installed under.
const SHIMMED_TOOLS: [&str; 5] = ["cargo", "task", "npm", "npx", "docker"];

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

/// A slot with no heartbeat for this long belongs to a dead holder (the script's own rule).
const STALE_SECS: u64 = 90;

/// Who holds the build slot right now, read from the lock's owner file.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SlotHolder {
    /// The holder's `AGENTMUX_AGENT_ID` (else its user name), as the script recorded it.
    pub agent: String,
    /// When it took the slot, in epoch milliseconds.
    pub since_ms: u64,
    /// Why: the shim's reason (`cargo test`), else the command line.
    pub reason: String,
    /// The command line it runs, cut to 120 characters: a background subagent's build holds the slot under its leader's id, and this says what is really running.
    pub command: String,
}

/// `~/.agentmux/shared/locks/build`, the lock's own folder, or `AGENTMUX_BUILD_SLOT_DIR` when
/// that is set (the script honours it too).
fn lock_dir() -> Option<PathBuf> {
    match std::env::var_os("AGENTMUX_BUILD_SLOT_DIR").filter(|d| !d.is_empty()) {
        Some(dir) => Some(PathBuf::from(dir)),
        None => dirs::home_dir().map(|h| h.join(".agentmux").join("shared").join("locks").join("build")),
    }
}

/// The current holder of this machine's build slot, if there is a live one.
pub fn holder() -> Option<SlotHolder> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    read_holder(&lock_dir()?, now)
}

/// The holder recorded under `root` at `now_secs`: none when the slot is free, when its owner
/// file is not written yet, or when nobody has kept it alive for `STALE_SECS` (a dead holder).
pub fn read_holder(root: &Path, now_secs: u64) -> Option<SlotHolder> {
    let slot = root.join("slot");
    let text = std::fs::read_to_string(slot.join("owner")).ok()?;
    let alive_at = [slot.join("heartbeat"), slot.clone()]
        .iter()
        .find_map(|p| std::fs::metadata(p).ok()?.modified().ok())?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    if now_secs.saturating_sub(alive_at) > STALE_SECS {
        return None;
    }
    let field = |key: &str| text.lines().find_map(|l| l.strip_prefix(key)?.strip_prefix('=')).map(str::trim);
    let agent = field("agent").filter(|a| !a.is_empty())?.to_string();
    let start: u64 = field("start")?.parse().ok()?;
    let reason = field("reason").filter(|r| !r.is_empty()).or_else(|| field("cmd")).unwrap_or_default().to_string();
    let command: String = field("cmd").unwrap_or_default().chars().take(120).collect();
    Some(SlotHolder { agent, since_ms: start.saturating_mul(1000), reason, command })
}

/// Environment variables that make a bash started with them define `cargo`, `task`, `npm`, `npx`
/// and `docker` as functions calling the shims under `home` (`BASH_FUNC_cargo%%` and so on: bash
/// imports an exported function from its environment).
///
/// A function wins over any PATH lookup, so the shim is reached whatever order a shell profile
/// left the PATH in. On Windows, Git Bash's profile puts `~/bin` first, and a `cargo.exe` there
/// (a rustup proxy) made every agent's `cargo` skip the slot: five srv compiles ran at once
/// (2026-10-25). The shims put their own directory back first for the tools they run, which
/// covers what those start (`task` -> `cargo`).
///
/// Nothing for a tool whose shim is not installed (or is somebody's own), nor when the slot is
/// switched off (`off`, from `AGENTMUX_BUILD_SLOT_OFF`).
pub fn shim_function_env(home: &Path, off: bool) -> Vec<(String, String)> {
    if off {
        return Vec::new();
    }
    let dir = shim_dir(home).to_string_lossy().replace('\\', "/");
    // The path goes between double quotes in the function body.
    if dir.chars().any(|c| matches!(c, '"' | '$' | '`' | '\\' | '\n' | '\r')) {
        return Vec::new();
    }
    SHIMMED_TOOLS
        .iter()
        .filter(|tool| std::fs::read_to_string(shim_dir(home).join(tool)).is_ok_and(|t| t.contains(SHIM_MARKER)))
        .map(|tool| (format!("BASH_FUNC_{tool}%%"), format!("() {{  \"{dir}/{tool}\" \"$@\"\n}}")))
        .collect()
}

/// [`shim_function_env`] for the current user's home, with the slot's off switch read from the
/// environment.
pub fn shim_function_env_for_current_user() -> Vec<(String, String)> {
    let off = std::env::var_os("AGENTMUX_BUILD_SLOT_OFF").is_some_and(|v| !v.is_empty());
    dirs::home_dir().map(|h| shim_function_env(&h, off)).unwrap_or_default()
}

/// What one `install` call did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Installed {
    /// Files written, by name.
    pub written: Vec<String>,
    /// Files left alone because they are not ours, by name.
    pub foreign: Vec<String>,
}

/// Installs for the current user's home directory. `Ok` with nothing written when there is none,
/// and when this AgentMux is isolated (`AGENTMUX_HOME_OVERRIDE`): the lock and the shims are shared
/// by every AgentMux on the machine, so a test or capture instance must not change them.
pub fn install_for_current_user() -> io::Result<Installed> {
    let isolated = std::env::var_os("AGENTMUX_HOME_OVERRIDE").is_some_and(|v| !v.is_empty());
    install_unless_isolated(isolated, dirs::home_dir().as_deref())
}

/// [`install`] into `home`, unless `isolated`: then nothing is written, so the start of a test or
/// capture instance leaves the real home as it was.
fn install_unless_isolated(isolated: bool, home: Option<&Path>) -> io::Result<Installed> {
    match home {
        Some(home) if !isolated => install(home),
        _ => Ok(Installed::default()),
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
    let (sh, cmd) = (lf(SHIM_SH), crlf(SHIM_CMD));
    let files = SHIMMED_TOOLS.iter().flat_map(|t| [(t.to_string(), &sh), (format!("{t}.cmd"), &cmd)]);
    for (name, text) in files {
        let text = text.as_str();
        let dest = dir.join(&name);
        match std::fs::read_to_string(&dest) {
            Ok(existing) if existing == text => {}
            Ok(existing) if existing.contains(SHIM_MARKER) => {
                write_executable(&dest, text)?;
                done.written.push(name.clone());
            }
            Ok(_) => done.foreign.push(name.clone()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                write_executable(&dest, text)?;
                done.written.push(name.clone());
            }
            // Not valid UTF-8 (a real binary named `cargo`): somebody else's.
            Err(e) if e.kind() == io::ErrorKind::InvalidData => done.foreign.push(name.clone()),
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

    fn write_slot(root: &Path, owner: &str, heartbeat: bool) {
        let slot = root.join("slot");
        std::fs::create_dir_all(&slot).unwrap();
        std::fs::write(slot.join("owner"), owner).unwrap();
        if heartbeat {
            std::fs::write(slot.join("heartbeat"), "").unwrap();
        }
    }

    fn now_secs() -> u64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
    }

    #[test]
    fn the_holder_is_read_from_the_owner_file_and_a_dead_one_is_not_a_holder() {
        let h = home();
        let root = h.path();
        assert_eq!(read_holder(root, now_secs()), None, "no slot: free");
        write_slot(root, "agent=Lazo\npid=1\ntoken=t\nstart=1792830000\ncwd=/x\ncmd=/c/bin/cargo test -p x\nreason=cargo test\n", true);
        assert_eq!(
            read_holder(root, now_secs()),
            Some(SlotHolder { agent: "Lazo".into(), since_ms: 1_792_830_000_000, reason: "cargo test".into(), command: "/c/bin/cargo test -p x".into() })
        );
        assert_eq!(read_holder(root, now_secs() + 10 * 60), None, "no heartbeat for minutes: its holder died");
        write_slot(root, "agent=Manoz\nstart=1792830001\ncmd=npm run build\n", true);
        assert_eq!(read_holder(root, now_secs()).unwrap().reason, "npm run build", "no reason: the command line");
        write_slot(root, "agent=Manoz\n", true);
        assert_eq!(read_holder(root, now_secs()), None, "an owner file still being written names no start");
        write_slot(root, "", false);
        assert_eq!(read_holder(root, now_secs()), None);
    }

    #[test]
    fn the_embedded_files_are_what_the_installer_expects() {
        assert!(version_of(SCRIPT).is_some(), "build-slot.sh needs a `# build-slot-version: N` line");
        assert!(SHIM_SH.contains(SHIM_MARKER), "the cargo shim must carry its marker");
        assert!(SHIM_CMD.contains(SHIM_MARKER), "the cargo.cmd shim must carry its marker");
        assert!(!lf(SCRIPT).contains('\r') && !lf(SHIM_SH).contains('\r'));
        // One function, called on the last line: bash reads a script a piece at a time, so a
        // wait still running an older copy would break mid-line when the file is replaced.
        let lines: Vec<&str> = SCRIPT.lines().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(lines.last().copied(), Some("main \"$@\"; exit $?"), "the script must end by calling main");
        assert!(lines.contains(&"main() {"), "the script body must be one function");
        assert!(crlf(SHIM_CMD).contains("\r\n") && !crlf(SHIM_CMD).replace("\r\n", "").contains('\n'));
    }

    /// An isolated instance (a test or capture run with `AGENTMUX_HOME_OVERRIDE`) leaves the real
    /// home alone: the lock and the shims are shared by every AgentMux on the machine.
    #[test]
    fn an_isolated_instance_installs_nothing_in_the_real_home() {
        let real = home();
        assert_eq!(install_unless_isolated(true, Some(real.path())).unwrap(), Installed::default());
        assert!(!real.path().join(".agentmux").exists(), "nothing at all was created under the home");
        assert_eq!(install_unless_isolated(false, None).unwrap(), Installed::default(), "no home: nothing to install into");
        let done = install_unless_isolated(false, Some(real.path())).unwrap();
        assert!(done.written.contains(&"build-slot.sh".to_string()) && done.written.contains(&"npm".to_string()));
        assert!(script_path(real.path()).exists(), "a normal instance still installs");
    }

    #[test]
    fn an_empty_home_gets_the_script_and_both_shims_once() {
        let h = home();
        let first = install(h.path()).unwrap();
        assert_eq!(
            first.written,
            vec!["build-slot.sh", "cargo", "cargo.cmd", "task", "task.cmd", "npm", "npm.cmd", "npx", "npx.cmd", "docker", "docker.cmd"]
        );
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
        assert!(done.written.contains(&"cargo.cmd".to_string()), "the other shims are still installed");
        assert!(done.written.contains(&"task".to_string()), "the other tools' shims are still installed");
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

    /// What the `tool` shim ran, given `args`, for a home with or without the lock script.
    fn run_tool_env(tool: &str, with_slot: bool, off: bool, args: &[&str], env: &[(&str, &str)]) -> Option<String> {
        let bash = bash()?;
        let h = home();
        let real_dir = h.path().join("real");
        let shim = shim_dir(h.path());
        std::fs::create_dir_all(&real_dir).unwrap();
        std::fs::create_dir_all(&shim).unwrap();
        let real = real_dir.join(tool);
        std::fs::write(&real, "#!/usr/bin/env bash\necho \"REAL $*\"\n").unwrap();
        std::fs::write(shim.join(tool), lf(SHIM_SH)).unwrap();
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
        cmd.arg(shim.join(tool)).args(args).env("PATH", path).env("HOME", h.path());
        if off {
            cmd.env("AGENTMUX_BUILD_SLOT_OFF", "1");
        }
        cmd.envs(env.iter().copied());
        #[cfg(windows)]
        {
            use crate::win32::NoWindow;
            cmd.no_window();
        }
        let out = cmd.output().unwrap();
        Some(String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n").trim().to_string())
    }

    fn run_tool(tool: &str, with_slot: bool, off: bool, args: &[&str]) -> Option<String> {
        run_tool_env(tool, with_slot, off, args, &[])
    }

    fn run_shim(with_slot: bool, off: bool, args: &[&str]) -> Option<String> {
        run_tool("cargo", with_slot, off, args)
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

    /// Whether the `tool` shim sent `args` through the slot (the stand-in lock script says so).
    fn slotted(tool: &str, args: &[&str]) -> Option<bool> {
        run_tool(tool, true, false, args).map(|out| out.starts_with("SLOT "))
    }

    #[test]
    fn task_npm_npx_and_docker_slot_their_heavy_commands_only() {
        if bash().is_none() {
            return;
        }
        let heavy: &[(&str, &[&str])] = &[
            ("task", &["package"]),
            ("task", &["package:portable"]),
            ("task", &["--verbose", "build:backend"]),
            ("task", &["-d", "x", "bundle:windows"]),
            ("task", &["check:doc-status", "package"]),
            ("npm", &["run", "build"]),
            ("npm", &["run", "--silent", "package:portable"]),
            ("npm", &["run", "test:e2e"]),
            ("npm", &["test"]),
            ("npm", &["run", "coverage"]),
            ("npm", &["run", "--workspace", "child", "coverage"]),
            ("npm", &["run", "--prefix", "/p", "build"]),
            ("npm", &["--workspace", "child", "run", "-w", "x", "package"]),
            ("npx", &["vite", "build"]),
            ("npx", &["vitest", "run", "x.test.ts"]),
            ("npx", &["vitest"]),
            ("docker", &["build", "-t", "x", "."]),
            ("docker", &["buildx", "build", "."]),
            ("docker", &["compose", "up", "--build"]),
            ("docker", &["--context", "c", "build", "."]),
        ];
        for (tool, args) in heavy {
            assert_eq!(slotted(tool, args), Some(true), "{tool} {args:?} should be slotted");
        }
        let light: &[(&str, &[&str])] = &[
            ("task", &["dev"]),
            ("task", &["run"]),
            ("task", &["check:doc-status"]),
            ("task", &["changeset", "--", "patch", "package"]),
            ("task", &[]),
            ("npm", &["install"]),
            ("npm", &["ci"]),
            ("npm", &["run", "lint:scss"]),
            ("npm", &["run", "dev"]),
            ("npm", &["run", "--workspace", "build", "lint:scss"]),
            ("npm", &[]),
            ("npx", &["tsc", "--noEmit"]),
            ("npx", &["vite"]),
            ("docker", &["run", "--rm", "x"]),
            ("docker", &["ps"]),
            ("docker", &["compose", "up"]),
            ("docker", &["--version"]),
        ];
        for (tool, args) in light {
            assert_eq!(slotted(tool, args), Some(false), "{tool} {args:?} should run directly");
        }
        // The reason names the command, and the real tool gets the original arguments.
        let out = run_tool("task", true, false, &["package", "--force"]).unwrap();
        assert!(out.starts_with("SLOT --reason task package -- "), "got {out:?}");
        assert!(contains_all(&out, &["real", "task package --force"]), "got {out:?}");
        assert_eq!(run_tool("npm", true, true, &["run", "build"]).unwrap(), "REAL run build", "the opt-out works for every tool");
        assert_eq!(run_tool("docker", false, false, &["build", "."]).unwrap(), "REAL build .", "no lock script: docker still runs");
    }

    /// With a terminal, a bare vitest watches and keeps running, so it is not slotted unless the
    /// arguments ask for a single run (`npm test -- --run`, #5379).
    #[test]
    fn with_a_terminal_only_a_one_shot_test_run_is_slotted() {
        if bash().is_none() {
            return;
        }
        let with_tty = |tool: &str, args: &[&str]| {
            run_tool_env(tool, true, false, args, &[("BUILD_SLOT_ASSUME_TTY", "1")]).map(|o| o.starts_with("SLOT "))
        };
        for (tool, args) in [
            ("npm", &["test", "--", "--run"][..]),
            ("npm", &["run", "test", "--", "--run"][..]),
            ("npx", &["vitest", "--run"][..]),
            ("npx", &["vitest", "run"][..]),
            ("npm", &["run", "build"][..]),
        ] {
            assert_eq!(with_tty(tool, args), Some(true), "{tool} {args:?} should be slotted with a terminal");
        }
        for (tool, args) in [
            ("npm", &["test"][..]),
            ("npm", &["run", "test"][..]),
            ("npx", &["vitest"][..]),
            ("npm", &["test", "--", "--coverage"][..]),
            ("npx", &["vitest", "--coverage"][..]),
        ] {
            assert_eq!(with_tty(tool, args), Some(false), "{tool} {args:?} watches with a terminal, so it is not slotted");
        }
    }

    /// Runs the lock script under bash with its slot directory at `root`, as a fresh top-level call.
    fn run_slot(bash: &Path, script: &Path, root: &Path, args: &[&str]) -> std::process::Output {
        let mut cmd = std::process::Command::new(bash);
        cmd.arg(script).args(args).env("AGENTMUX_BUILD_SLOT_DIR", root).env_remove("BUILD_SLOT_HELD").env("BUILD_SLOT_POLL", "1");
        #[cfg(windows)]
        {
            use crate::win32::NoWindow;
            cmd.no_window();
        }
        cmd.output().unwrap()
    }

    /// Every run leaves one line in the history, and `history` summarises them by reason.
    #[test]
    fn a_run_is_recorded_in_the_history_and_summarised() {
        let Some(bash) = bash() else { return };
        let h = home();
        let script = h.path().join("build-slot.sh");
        std::fs::write(&script, lf(SCRIPT)).unwrap();
        let root = h.path().join("slot dir");
        let run = |args: &[&str]| run_slot(&bash, &script, &root, args);
        assert_eq!(run(&["--reason", "cargo test", "--", "true"]).status.code(), Some(0));
        assert_eq!(run(&["--reason", "task package", "--", "sh", "-c", "exit 3"]).status.code(), Some(3));
        let log = std::fs::read_to_string(root.join("history.log")).unwrap();
        let rows: Vec<Vec<&str>> = log.lines().map(|l| l.split('\t').collect()).collect();
        assert_eq!(rows.len(), 2, "one line per run: {log:?}");
        assert!(rows.iter().all(|r| r.len() == 6), "epoch, agent, reason, waited, held, exit: {log:?}");
        assert_eq!((rows[0][2], rows[0][5]), ("cargo test", "0"));
        assert_eq!((rows[1][2], rows[1][5]), ("task package", "3"), "the wrapped command's exit code is recorded");
        let summary = String::from_utf8_lossy(&run(&["history"]).stdout).to_string();
        assert!(summary.starts_with("runs: 2,"), "got {summary:?}");
        assert!(contains_all(&summary, &["cargo test", "task package"]), "got {summary:?}");
    }

    /// A run cut short by a signal is still recorded (once, exit 130), and the log rotates
    /// to `history.log.1` instead of being rewritten once it is long.
    #[test]
    fn an_interrupted_run_is_recorded_and_a_long_log_rotates() {
        let Some(bash) = bash() else { return };
        let h = home();
        let script = h.path().join("build-slot.sh");
        std::fs::write(&script, lf(SCRIPT)).unwrap();
        let root = h.path().join("slot dir");
        let run = |args: &[&str]| run_slot(&bash, &script, &root, args);
        // The wrapped command signals its parent, the wrapper, and then ends.
        let out = run(&["--reason", "cargo test", "--", "bash", "-c", "kill -TERM $PPID; sleep 1"]);
        assert_eq!(out.status.code(), Some(130));
        let log = std::fs::read_to_string(root.join("history.log")).unwrap();
        let rows: Vec<Vec<&str>> = log.lines().map(|l| l.split('\t').collect()).collect();
        assert_eq!(rows.len(), 1, "recorded once, not twice: {log:?}");
        assert_eq!((rows[0][2], rows[0][5]), ("cargo test", "130"));
        assert!(!root.join("slot").exists(), "the slot is freed");

        let filler = "1\tx\told\t0\t0\t0\n".repeat(2001);
        std::fs::write(root.join("history.log"), filler).unwrap();
        assert_eq!(run(&["--reason", "task package", "--", "true"]).status.code(), Some(0));
        assert!(root.join("history.log.1").exists(), "past 2000 lines the log is renamed aside");
        assert!(!root.join("history.log").exists(), "and the next append starts a new file");
        let summary = String::from_utf8_lossy(&run(&["history"]).stdout).to_string();
        assert!(summary.starts_with("runs: 2002,"), "history reads both generations: {summary:?}");
    }

    #[test]
    fn shim_functions_are_offered_for_every_installed_shim_and_nothing_else() {
        let h = home();
        assert!(shim_function_env(h.path(), false).is_empty(), "no shims installed: no functions");
        install(h.path()).unwrap();
        let env = shim_function_env(h.path(), false);
        let names: Vec<&str> = env.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(names, ["BASH_FUNC_cargo%%", "BASH_FUNC_task%%", "BASH_FUNC_npm%%", "BASH_FUNC_npx%%", "BASH_FUNC_docker%%"]);
        let dir = shim_dir(h.path()).to_string_lossy().replace('\\', "/");
        assert_eq!(env[0].1, format!("() {{  \"{dir}/cargo\" \"$@\"\n}}"));
        assert!(shim_function_env(h.path(), true).is_empty(), "the opt-out switches them off");
        // A `cargo` that is somebody's own is not ours to put a function in front of.
        std::fs::write(shim_dir(h.path()).join("cargo"), "#!/bin/sh\necho mine\n").unwrap();
        let env = shim_function_env(h.path(), false);
        assert!(env.iter().all(|(k, _)| k != "BASH_FUNC_cargo%%") && env.len() == 4);
    }

    /// The bug of 2026-10-25: a `cargo` earlier on the PATH than the shims' directory (Git Bash's
    /// profile puts `~/bin` first) skipped the slot. With the functions in the environment the shim
    /// is reached anyway; without them it is not.
    #[test]
    fn a_cargo_earlier_on_the_path_still_goes_through_the_slot_with_the_functions() {
        let Some(bash) = bash() else { return };
        let h = home();
        install(h.path()).unwrap();
        // Stand-ins: a lock script that says it ran, a "user bin" cargo that is first on the PATH,
        // and the real cargo after the shims.
        std::fs::write(script_path(h.path()), "#!/usr/bin/env bash\necho \"SLOT $*\"\n").unwrap();
        let (user_bin, real_dir) = (h.path().join("userbin"), h.path().join("real"));
        for (dir, text) in [(&user_bin, "BYPASS"), (&real_dir, "REAL")] {
            std::fs::create_dir_all(dir).unwrap();
            std::fs::write(dir.join("cargo"), format!("#!/usr/bin/env bash\necho {text}\n")).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(dir.join("cargo"), std::fs::Permissions::from_mode(0o755)).unwrap();
            }
        }
        let sep = if cfg!(windows) { ";" } else { ":" };
        let path = format!("{}{sep}{}{sep}{}{sep}{}", user_bin.display(), shim_dir(h.path()).display(), real_dir.display(), std::env::var("PATH").unwrap_or_default());
        let run = |with_functions: bool| {
            let mut cmd = std::process::Command::new(&bash);
            cmd.args(["-c", "cargo test -p x"]).env("PATH", &path).env("HOME", h.path()).env_remove("BUILD_SLOT_HELD").env_remove("AGENTMUX_BUILD_SLOT_OFF");
            if with_functions {
                cmd.envs(shim_function_env(h.path(), false));
            }
            #[cfg(windows)]
            {
                use crate::win32::NoWindow;
                cmd.no_window();
            }
            String::from_utf8_lossy(&cmd.output().unwrap().stdout).replace("\r\n", "\n").trim().to_string()
        };
        assert_eq!(run(false), "BYPASS", "without the functions the earlier cargo wins, which is the bug");
        let out = run(true);
        assert!(out.starts_with("SLOT --reason cargo test -- "), "got {out:?}");
        assert!(out.contains("userbin") && out.ends_with("cargo test -p x"), "the slot runs the cargo the shell would have found, the one in the user's bin: {out:?}");
    }

    /// What the shim runs starts with the shims' directory first on the PATH, so `task` ->
    /// `cargo` and `npm run build` -> `cargo` are slotted too.
    #[test]
    fn the_tool_a_shim_runs_finds_the_shims_first() {
        let Some(bash) = bash() else { return };
        let h = home();
        let (shim, real_dir) = (shim_dir(h.path()), h.path().join("real"));
        std::fs::create_dir_all(&shim).unwrap();
        std::fs::create_dir_all(&real_dir).unwrap();
        std::fs::write(shim.join("task"), lf(SHIM_SH)).unwrap();
        std::fs::write(real_dir.join("task"), "#!/usr/bin/env bash\necho \"FIRST ${PATH%%:*}\"\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(real_dir.join("task"), std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let sep = if cfg!(windows) { ";" } else { ":" };
        // The shims' directory is NOT first.
        let path = format!("{}{sep}{}{sep}{}", real_dir.display(), shim.display(), std::env::var("PATH").unwrap_or_default());
        let mut cmd = std::process::Command::new(bash);
        cmd.arg(shim.join("task")).arg("dev").env("PATH", path).env("HOME", h.path()).env("AGENTMUX_BUILD_SLOT_OFF", "1");
        #[cfg(windows)]
        {
            use crate::win32::NoWindow;
            cmd.no_window();
        }
        let out = String::from_utf8_lossy(&cmd.output().unwrap().stdout).replace("\r\n", "\n").trim().to_string();
        assert!(out.starts_with("FIRST ") && out.ends_with(".agentmux/tools/bin"), "got {out:?}");
    }

    fn spawn_slot(bash: &Path, script: &Path, root: &Path, limit: Option<&str>, args: &[&str]) -> std::process::Child {
        let mut cmd = std::process::Command::new(bash);
        cmd.arg(script)
            .args(args)
            .env("AGENTMUX_BUILD_SLOT_DIR", root)
            .env_remove("BUILD_SLOT_HELD")
            .env_remove("BUILD_SLOT_MAX")
            .env("BUILD_SLOT_POLL", "1")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        if let Some(n) = limit {
            cmd.env("BUILD_SLOT_MAX", n);
        }
        #[cfg(windows)]
        {
            use crate::win32::NoWindow;
            cmd.no_window();
        }
        cmd.spawn().unwrap()
    }

    /// One run at a time by default; with a limit of two, two hold a slot at once and a third waits,
    /// saying how long it has waited and who holds the slot.
    #[test]
    fn a_limit_of_two_lets_two_runs_hold_a_slot_and_a_third_waits() {
        let Some(bash) = bash() else { return };
        let h = home();
        let script = h.path().join("build-slot.sh");
        std::fs::write(&script, lf(SCRIPT)).unwrap();
        let root = h.path().join("locks");
        let status = |limit: Option<&str>| {
            let out = spawn_slot(&bash, &script, &root, limit, &["status"]).wait_with_output().unwrap();
            String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n")
        };

        // Waits (up to 30 s) for the status to say `want`: process start-up is slow under load.
        let until = |limit: Option<&str>, want: &[&str]| {
            let mut last = String::new();
            for _ in 0..60 {
                last = status(limit);
                if contains_all(&last, want) {
                    return last;
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            panic!("never saw {want:?} in the status; the last was {last:?}");
        };

        // Default limit: the second run queues behind the first.
        let first = spawn_slot(&bash, &script, &root, None, &["--", "sleep", "10"]);
        until(None, &["slot: HELD"]);
        let second = spawn_slot(&bash, &script, &root, None, &["--", "true"]);
        let s = until(None, &["slot: HELD", "queued #1"]);
        assert!(!s.contains("slot-2"), "one slot: the second waits, it does not take another: {s:?}");
        let second = second.wait_with_output().unwrap();
        assert_eq!(second.status.code(), Some(0));
        let waited = String::from_utf8_lossy(&second.stderr).to_string();
        assert!(contains_all(&waited, &["waiting for the build slot", "has waited", "slot: HELD by"]), "got {waited:?}");
        first.wait_with_output().unwrap();

        // A limit of two, from the limit file: two hold at once, the third waits.
        std::fs::write(root.join("limit"), "2\n").unwrap();
        let a = spawn_slot(&bash, &script, &root, None, &["--", "sleep", "10"]);
        until(None, &["slots: 1 of 2 in use"]);
        let b = spawn_slot(&bash, &script, &root, None, &["--", "sleep", "10"]);
        until(None, &["slot: HELD", "slot-2: HELD", "slots: 2 of 2 in use"]);
        let c = spawn_slot(&bash, &script, &root, None, &["--", "true"]);
        until(None, &["slots: 2 of 2 in use", "queued #1"]);
        assert_eq!(c.wait_with_output().unwrap().status.code(), Some(0), "the third runs once one is released");
        a.wait_with_output().unwrap();
        b.wait_with_output().unwrap();
    }

    /// A call from a process that already holds a slot (task -> cargo, a build script running
    /// cargo) goes straight through: it never waits for the slot it is inside.
    #[test]
    fn a_nested_call_inside_a_held_slot_does_not_wait_for_itself() {
        let Some(bash) = bash() else { return };
        let h = home();
        let script = h.path().join("build-slot.sh");
        std::fs::write(&script, lf(SCRIPT)).unwrap();
        let root = h.path().join("locks");
        let outer = format!("bash '{}' -- echo inner", script.to_string_lossy().replace('\\', "/"));
        let out = spawn_slot(&bash, &script, &root, None, &["--", "bash", "-c", &outer]);
        let started = std::time::Instant::now();
        let out = out.wait_with_output().unwrap();
        assert!(started.elapsed() < std::time::Duration::from_secs(20), "the nested call waited for its own slot");
        assert_eq!(out.status.code(), Some(0));
        assert!(String::from_utf8_lossy(&out.stdout).contains("inner"));
        // And it passes straight through even while somebody else holds the only slot.
        let holder = spawn_slot(&bash, &script, &root, None, &["--", "sleep", "4"]);
        std::thread::sleep(std::time::Duration::from_millis(1500));
        let mut nested = std::process::Command::new(&bash);
        nested.arg(&script).args(["--", "echo", "ran"]).env("AGENTMUX_BUILD_SLOT_DIR", &root).env("BUILD_SLOT_HELD", "1");
        #[cfg(windows)]
        {
            use crate::win32::NoWindow;
            nested.no_window();
        }
        let started = std::time::Instant::now();
        let out = nested.output().unwrap();
        assert!(started.elapsed() < std::time::Duration::from_secs(3), "a nested call queued behind another holder");
        assert!(String::from_utf8_lossy(&out.stdout).contains("ran"));
        holder.wait_with_output().unwrap();
    }

    /// A lock folder that can't be made or written never stops the build: the command runs, with a warning.
    #[test]
    fn an_unusable_lock_folder_runs_the_command_with_a_warning() {
        let Some(bash) = bash() else { return };
        let h = home();
        let script = h.path().join("build-slot.sh");
        std::fs::write(&script, lf(SCRIPT)).unwrap();
        // A file where the folder's parent should be: mkdir -p can't succeed.
        let blocker = h.path().join("not-a-folder");
        std::fs::write(&blocker, "x").unwrap();
        let root = blocker.join("locks");
        let started = std::time::Instant::now();
        let out = spawn_slot(&bash, &script, &root, None, &["--", "sh", "-c", "echo ran; exit 7"]).wait_with_output().unwrap();
        assert!(started.elapsed() < std::time::Duration::from_secs(10), "it must not wait for a lock it can't take");
        assert_eq!(out.status.code(), Some(7), "the command's own exit code");
        assert!(String::from_utf8_lossy(&out.stdout).contains("ran"));
        assert!(String::from_utf8_lossy(&out.stderr).contains("running without the build slot"), "{:?}", String::from_utf8_lossy(&out.stderr));
    }

    #[test]
    fn the_shim_fails_open() {
        if bash().is_none() {
            return;
        }
        assert_eq!(run_shim(false, false, &["test"]).unwrap(), "REAL test", "no lock script: cargo still runs");
        assert_eq!(run_shim(true, true, &["test"]).unwrap(), "REAL test", "AGENTMUX_BUILD_SLOT_OFF skips the slot");
    }

    /// A wait started by a native program (this test binary; on Windows also `cmd.exe` and
    /// `task.exe`, which is how the `cargo` shim usually reaches the script) is not an orphan.
    /// Git Bash reports pid 1 as the parent of such a wait; the script used to read that as
    /// "the process that started this wait is gone" and leave the queue with exit 129, so a
    /// `cargo build` that had to wait for the slot failed instead of waiting.
    #[test]
    fn a_wait_started_by_a_native_parent_waits_instead_of_leaving() {
        let Some(bash) = bash() else { return };
        let h = home();
        let script = h.path().join("build-slot.sh");
        std::fs::write(&script, lf(SCRIPT)).unwrap();
        // A slot held by someone else, with a heartbeat from just now.
        let root = h.path().join("slots");
        let slot = root.join("slot");
        std::fs::create_dir_all(&slot).unwrap();
        std::fs::write(slot.join("owner"), "agent=other\ntoken=t\nstart=1\ncwd=/\ncmd=cargo test\n").unwrap();
        std::fs::write(slot.join("heartbeat"), "").unwrap();
        let mut cmd = std::process::Command::new(bash);
        cmd.arg(&script)
            .args(["--", "echo", "never"])
            .env("AGENTMUX_BUILD_SLOT_DIR", &root)
            // A test run inside the slot (build-slot.sh -- cargo test) inherits this, and a
            // script that finds it set runs its command without waiting.
            .env_remove("BUILD_SLOT_HELD")
            .env("BUILD_SLOT_POLL", "1")
            .env("BUILD_SLOT_MAX_WAIT", "3");
        #[cfg(windows)]
        {
            use crate::win32::NoWindow;
            cmd.no_window();
        }
        let out = cmd.output().unwrap();
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(!err.contains("is gone"), "the wait took its parent for gone: {err}");
        assert_eq!(out.status.code(), Some(75), "it should wait out BUILD_SLOT_MAX_WAIT and give up: {err}");
        assert!(!String::from_utf8_lossy(&out.stdout).contains("never"), "the command must not run while the slot is held");
    }
}
