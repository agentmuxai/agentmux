// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Every place a Windows-reachable crate starts a child process, pinned, so that a new
//! spawn without a console-window decision fails here instead of flashing a console over
//! the splash on a user's desktop.
//!
//! # Why this exists
//!
//! `agentmux-srv` is a console-subsystem program, and the launcher and cef host are GUI
//! programs. A console-program child started from a console-less parent without
//! `CREATE_NO_WINDOW` makes Windows allocate a console window. Nothing checked that:
//! #2171, #2173 and #3788 (`schtasks /Query` at every startup) all shipped it and were found by
//! a person watching the splash. The `--crash-monitor` self-spawn had the same gap since #246.
//!
//! # Shape
//!
//! This is the same design as `spawn_site_coverage` in `crates/srv/src/backend/pane_env.rs`
//! (which pins the same surface for the *environment* sanitiser): an exact
//! `(file, program, count, classification)` table. A per-file check ("does this file call
//! `no_window()` somewhere?") is too weak, because a new unflagged spawn next to an old
//! flagged one passes it. An exact count changes when a spawn is added or removed, so someone
//! has to read the diff and classify it.
//!
//! Adding a spawn is a one-row diff: give it `.no_window()` (`agentmux_common::win32::NoWindow`)
//! or `creation_flags`, and say so here.
//!
//! # Classifications
//!
//! - `no-window:` the file calls `no_window()` or `creation_flags(`; checked against the file.
//! - `mixed:` as above, and the count also includes test fixtures in the same file.
//! - `gui-program:` the child is a GUI-subsystem program, so no console can appear.
//! - `pty:` a `portable-pty` `CommandBuilder`, which runs on a ConPTY pseudoconsole.
//! - `not-windows:` the spawn is in a macOS/Linux/unix-only code path.
//! - `test:` test code only.
//!
//! What this does not prove: that the flag is on *that* spawn rather than a neighbour. It
//! proves a human classified every spawn, and that the file calls the helper at least once.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Crates whose spawns this covers. `agentmux-srv`'s environment inventory is separate.
const CRATES: &[&str] = &["bashwrap", "cef", "common", "launcher", "srv"];

const SPAWN_MARKERS: &[&str] = &["CommandBuilder::new", "process::Command::new", "Command::new("];

const CLASSES: &[&str] = &["no-window:", "mixed:", "gui-program:", "pty:", "not-windows:", "test:"];

/// Defines the spawn markers as strings and does not itself spawn.
const SKIP_FILES: &[&str] = &["crates/srv/src/backend/pane_env.rs"];

/// `(file, program, count, classification)`. `program` is the literal text between `new(` and
/// the first `,` or `)`, which is stable across edits in a way line numbers are not.
const SPAWN_INVENTORY: &[(&str, &str, usize, &str)] = &[
    ("crates/bashwrap/src/bash_wrap.rs", "bash.as_os_str(", 1,
     "no-window: calls no_window()/creation_flags in this file"),
    ("crates/bashwrap/src/bash_wrap.rs", "bash", 1,
     "no-window: calls no_window()/creation_flags in this file"),
    ("crates/bashwrap/src/prefix.rs", "&bash", 1,
     "no-window: calls no_window()/creation_flags in this file"),
    ("crates/cef/src/lib.rs", "&exe", 1,
     "not-windows: Linux-only (zenity/kdialog dialogs) or the unix exec() self-relaunch"),
    ("crates/cef/src/lib.rs", "\"zenity\"", 1,
     "not-windows: Linux-only (zenity/kdialog dialogs) or the unix exec() self-relaunch"),
    ("crates/cef/src/lib.rs", "\"kdialog\"", 1,
     "not-windows: Linux-only (zenity/kdialog dialogs) or the unix exec() self-relaunch"),
    ("crates/cef/src/linux_sandbox.rs", "exe", 1,
     "not-windows: Linux sandbox probe, compiled only on Linux"),
    ("crates/cef/src/linux_sandbox.rs", "\"which\"", 1,
     "not-windows: Linux sandbox probe, compiled only on Linux"),
    ("crates/cef/src/linux_sandbox.rs", "\"zenity\"", 1,
     "not-windows: Linux sandbox probe, compiled only on Linux"),
    ("crates/cef/src/linux_sandbox.rs", "\"kdialog\"", 1,
     "not-windows: Linux sandbox probe, compiled only on Linux"),
    ("crates/cef/src/linux_sandbox.rs", "\"pkexec\"", 1,
     "not-windows: Linux sandbox probe, compiled only on Linux"),
    ("crates/cef/src/sidecar.rs", "&backend_path", 1,
     "no-window: calls no_window()/creation_flags in this file"),
    ("crates/cef/src/app/gl_probe.rs", "exe", 1,
     "gui-program: re-runs agentmux-cef itself (GUI subsystem, no console) to probe GL"),
    ("crates/common/src/xauthority.rs", "\"systemctl\"", 1,
     "not-windows: macOS/Linux/unix-only code path"),
    ("crates/cef/src/commands/autostart.rs", "exe", 1,
     "no-window: calls no_window()/creation_flags in this file"),
    ("crates/cef/src/commands/backend.rs", "&srv_path", 1,
     "no-window: calls no_window()/creation_flags in this file"),
    ("crates/cef/src/commands/cli_login.rs", "cli_path", 1,
     "no-window: calls no_window()/creation_flags in this file"),
    ("crates/cef/src/commands/cli_login.rs", "&spawn_program", 1,
     "no-window: calls no_window()/creation_flags in this file"),
    ("crates/cef/src/commands/cli_login.rs", "\"cmd.exe\"", 1,
     "no-window: calls no_window()/creation_flags in this file"),
    ("crates/cef/src/commands/cli_login.rs", "\"open\"", 1,
     "not-windows: macOS/Linux/unix-only code path"),
    ("crates/cef/src/commands/cli_login.rs", "bin", 1,
     "no-window: calls no_window()/creation_flags in this file"),
    ("crates/cef/src/commands/clipboard.rs", "\"pbpaste\"", 1,
     "not-windows: macOS/Linux/unix-only code path"),
    ("crates/cef/src/commands/clipboard.rs", "\"pbcopy\"", 1,
     "not-windows: macOS/Linux/unix-only code path"),
    ("crates/cef/src/commands/clipboard.rs", "\"wl-paste\"", 3,
     "not-windows: macOS/Linux/unix-only code path"),
    ("crates/cef/src/commands/clipboard.rs", "\"xclip\"", 4,
     "not-windows: macOS/Linux/unix-only code path"),
    ("crates/cef/src/commands/clipboard.rs", "\"xsel\"", 2,
     "not-windows: macOS/Linux/unix-only code path"),
    ("crates/cef/src/commands/clipboard.rs", "\"wl-copy\"", 1,
     "not-windows: macOS/Linux/unix-only code path"),
    ("crates/cef/src/commands/clipboard.rs", "\"osascript\"", 1,
     "not-windows: macOS/Linux/unix-only code path"),
    ("crates/cef/src/commands/platform.rs", "\"explorer\"", 1,
     "gui-program: explorer/rundll32 are GUI-subsystem, on user action only"),
    ("crates/cef/src/commands/platform.rs", "editor", 1,
     "not-windows: macOS/Linux/unix-only code path"),
    ("crates/cef/src/commands/platform.rs", "\"open\"", 4,
     "not-windows: macOS/Linux/unix-only code path"),
    ("crates/cef/src/commands/platform.rs", "\"xdg-open\"", 3,
     "not-windows: macOS/Linux/unix-only code path"),
    ("crates/cef/src/commands/platform.rs", "program", 1,
     "gui-program: spawn_detached runs explorer.exe on Windows, GUI-subsystem, on user action only"),
    ("crates/cef/src/commands/platform.rs", "\"rundll32.exe\"", 1,
     "gui-program: explorer/rundll32 are GUI-subsystem, on user action only"),
    ("crates/cef/src/commands/platform.rs", "\"explorer.exe\"", 1,
     "gui-program: explorer/rundll32 are GUI-subsystem, on user action only"),
    ("crates/common/src/cli.rs", "&program", 1,
     "no-window: calls no_window()/creation_flags in this file"),
    ("crates/common/src/process.rs", "\"taskkill\"", 1,
     "no-window: calls no_window()/creation_flags in this file"),
    ("crates/common/src/process.rs", "\"sleep\"", 1,
     "test: the unix-only force_kill_tree test's child"),
    ("crates/common/src/runtime_mode.rs", "\"git\"", 1,
     "no-window: calls no_window()/creation_flags in this file"),
    ("crates/common/src/toolchain_path.rs", "shell", 1,
     "not-windows: macOS/Linux/unix-only code path"),
    ("crates/common/src/win32.rs", "\"agentmux-no-such-program\"", 2,
     "test: unit-test or fixture code, never on a user machine"),
    ("crates/launcher/src/host_hang_dump.rs", "\"cmd\"", 1,
     "test: unit-test or fixture code, never on a user machine"),
    ("crates/launcher/src/host_spawn.rs", "real_exe", 2,
     "no-window: calls no_window()/creation_flags in this file"),
    ("crates/launcher/src/host_spawn.rs", "\"sleep\"", 3,
     "test: unit-test or fixture code, never on a user machine"),
    ("crates/launcher/src/host_spawn.rs", "\"true\"", 2,
     "test: unit-test or fixture code, never on a user machine"),
    ("crates/launcher/src/host_spawn.rs", "\"agentmux-host\"", 1,
     "test: unit-test or fixture code, never on a user machine"),
    ("crates/launcher/src/splash_info.rs", "cmd", 1,
     "no-window: calls no_window()/creation_flags in this file"),
    ("crates/launcher/src/srv_spawner.rs", "&backend_path", 2,
     "no-window: calls no_window()/creation_flags in this file"),
    ("crates/srv/src/backend/process_tracker/cgroup_linux.rs", "\"sleep\"", 1,
     "test: unit-test or fixture code, never on a user machine"),
    ("crates/srv/src/backend/process_tracker/cgroup_linux.rs", "&argv[0]", 1,
     "test: unit-test or fixture code, never on a user machine"),
    ("crates/srv/src/backend/process_tracker/scan.rs", "\"sh\"", 1,
     "test: unit-test or fixture code, never on a user machine"),
    ("crates/srv/src/backend/process_tracker/cgroup_linux.rs", "\"sh\"", 1,
     "test: unit-test or fixture code, never on a user machine"),
    ("crates/launcher/src/upgrade.rs", "\"sleep\"", 1,
     "test: unit-test or fixture code, never on a user machine"),
    ("crates/launcher/src/upgrade.rs", "\"cmd\"", 1,
     "test: unit-test or fixture code, never on a user machine"),
    ("crates/launcher/src/upgrade.rs", "\"sh\"", 2,
     "test: unit-test or fixture code, never on a user machine"),
    ("crates/launcher/src/upgrade.rs", "if cfg!(windows", 1,
     "test: unit-test or fixture code, never on a user machine"),
    ("crates/launcher/src/autostart/mod.rs", "\"schtasks\"", 1,
     "no-window: calls no_window()/creation_flags in this file"),
    ("crates/srv/src/crash_monitor.rs", "&exe", 1,
     "no-window: calls no_window()/creation_flags in this file"),
    ("crates/srv/src/test_support.rs", "\"rustc\"", 1,
     "test: unit-test or fixture code, never on a user machine"),
    ("crates/srv/src/util.rs", "\"cmd\"", 3,
     "mixed: production spawn sets the flag; the rest are test fixtures in the same file"),
    ("crates/srv/src/util.rs", "\"open\"", 1,
     "not-windows: macOS/Linux/unix-only code path"),
    ("crates/srv/src/util.rs", "\"xdg-open\"", 1,
     "not-windows: macOS/Linux/unix-only code path"),
    ("crates/srv/src/agents/runner.rs", "bin", 1,
     "no-window: calls no_window()/creation_flags in this file"),
    ("crates/srv/src/backend/remote/host.rs", "&self.ssh_path", 1,
     "no-window: calls no_window()/creation_flags in this file"),
    ("crates/srv/src/backend/remote/ssh.rs", "ssh", 1,
     "no-window: calls no_window()/creation_flags in this file"),
    ("crates/srv/src/backend/claude_layout.rs", "\"git\"", 1,
     "test: unit-test or fixture code, never on a user machine"),
    ("crates/srv/src/backend/container_cli.rs", "\"sh\"", 2,
     "test: unit-test or fixture code, never on a user machine"),
    ("crates/srv/src/backend/cli_prune.rs", "\"ps\"", 1,
     "not-windows: returns None on Windows before it spawns `ps`"),
    ("crates/srv/src/backend/cli_install.rs", "\"cmd\"", 1,
     "no-window: calls no_window()/creation_flags in this file"),
    ("crates/srv/src/backend/cli_install.rs", "\"npm\"", 1,
     "no-window: calls no_window()/creation_flags in this file"),
    ("crates/srv/src/backend/mcp_probe.rs", "command", 1,
     "no-window: calls no_window()/creation_flags in this file"),
    ("crates/srv/src/backend/remote/helper_install.rs", "\"sh\"", 2,
     "test: unit-test or fixture code, never on a user machine"),
    ("crates/srv/src/backend/fs_ops/host_jobs.rs", "\"mkfifo\"", 1,
     "test: unit-test or fixture code, never on a user machine"),
    ("crates/srv/src/backend/tower_remote.rs", "\"wsl.exe\"", 1,
     "no-window: calls no_window()/creation_flags in this file"),
    ("crates/srv/src/backend/remote/wsl.rs", "\"wsl.exe\"", 1,
     "no-window: calls no_window()/creation_flags in this file"),
    ("crates/srv/src/backend/shell_node.rs", "\"cmd\"", 1,
     "no-window: calls no_window()/creation_flags in this file"),
    ("crates/srv/src/backend/shell_node.rs", "\"sh\"", 1,
     "no-window: calls no_window()/creation_flags in this file"),
    ("crates/srv/src/backend/shell_node.rs", "program", 1,
     "no-window: calls no_window()/creation_flags in this file"),
    ("crates/srv/src/backend/storage/muxbus.rs", "std::env::current_exe(", 1,
     "test: unit-test or fixture code, never on a user machine"),
    ("crates/srv/src/backend/tool_store.rs", "\"where\"", 2,
     "no-window: calls no_window()/creation_flags in this file"),
    ("crates/srv/src/backend/tool_store.rs", "\"which\"", 2,
     "not-windows: macOS/Linux/unix-only code path"),
    ("crates/srv/src/backend/tool_store.rs", "cmd", 1,
     "no-window: calls no_window()/creation_flags in this file"),
    ("crates/srv/src/backend/attachments/extract.rs", "exe", 1,
     "no-window: calls no_window()/creation_flags in this file"),
    ("crates/srv/src/backend/attachments/extract.rs", "harness", 1,
     "test: unit-test or fixture code, never on a user machine"),
    ("crates/srv/src/backend/blockcontroller/app_server.rs", "fake_server_binary(", 1,
     "test: unit-test or fixture code, never on a user machine"),
    ("crates/srv/src/backend/blockcontroller/persistent/tests/eager_resume.rs", "\"taskkill\"", 1,
     "test: unit-test or fixture code, never on a user machine"),
    ("crates/srv/src/backend/blockcontroller/persistent/tests/eager_resume.rs", "\"kill\"", 1,
     "test: unit-test or fixture code, never on a user machine"),
    ("crates/srv/src/backend/blockcontroller/shell/lifecycle.rs", "&program", 1,
     "pty: portable-pty CommandBuilder, runs on a ConPTY pseudoconsole"),
    ("crates/srv/src/backend/blockcontroller/shell/lifecycle.rs", "\"cmd.exe\"", 1,
     "pty: portable-pty CommandBuilder, runs on a ConPTY pseudoconsole"),
    ("crates/srv/src/backend/blockcontroller/shell/lifecycle.rs", "\"/bin/sh\"", 1,
     "pty: portable-pty CommandBuilder, runs on a ConPTY pseudoconsole"),
    ("crates/srv/src/backend/blockcontroller/shell/lifecycle.rs", "&shell_path", 1,
     "pty: portable-pty CommandBuilder, runs on a ConPTY pseudoconsole"),
    ("crates/srv/src/backend/blockcontroller/shell/lifecycle.rs", "\"shell-under-test\"", 1,
     "test: unit-test or fixture code, never on a user machine"),
    ("crates/srv/src/backend/blockcontroller/shell/lifecycle.rs", "\"wsl.exe\"", 1,
     "pty: portable-pty CommandBuilder, runs on a ConPTY pseudoconsole (a WSL pane)"),
    ("crates/srv/src/backend/blockcontroller/shell/lifecycle.rs", "ssh_path", 1,
     "pty: portable-pty CommandBuilder, runs on a ConPTY pseudoconsole (an SSH pane)"),
    ("crates/srv/src/backend/blockcontroller/shell/pty.rs", "\"where\"", 2,
     "no-window: calls no_window()/creation_flags in this file"),
    ("crates/srv/src/backend/blockcontroller/shell/tests.rs", "\"sleep\"", 1,
     "test: unit-test or fixture code, never on a user machine"),
    ("crates/srv/src/backend/fs_ops/git.rs", "\"git\"", 5,
     "no-window: calls no_window()/creation_flags in this file"),
    ("crates/srv/src/backend/fs_ops/platform.rs", "\"explorer.exe\"", 2,
     "gui-program: explorer.exe is GUI-subsystem, on user action only (Files pane open/reveal)"),
    ("crates/srv/src/backend/fs_ops/platform.rs", "\"open\"", 2,
     "not-windows: macOS/Linux/unix-only code path"),
    ("crates/srv/src/backend/fs_ops/platform.rs", "\"xdg-open\"", 2,
     "not-windows: macOS/Linux/unix-only code path"),
    ("crates/srv/src/backend/fs_watch/pool.rs", "\"wsl.exe\"", 2,
     "test: unit-test or fixture code, never on a user machine"),
    ("crates/srv/src/backend/lsp/supervisor.rs", "&resolved", 1,
     "no-window: calls no_window()/creation_flags in this file"),
    ("crates/srv/src/backend/process_tracker/registry.rs", "\"cmd\"", 5,
     "test: unit-test or fixture code, never on a user machine"),
    ("crates/srv/src/backend/process_tracker/registry.rs", "\"sh\"", 1,
     "test: unit-test or fixture code, never on a user machine"),
    ("crates/srv/src/backend/process_tracker/windows.rs", "\"cmd\"", 1,
     "test: unit-test or fixture code, never on a user machine"),
    ("crates/srv/src/sagas/close_pane.rs", "\"cmd\"", 2,
     "test: unit-test or fixture code, never on a user machine"),
    ("crates/srv/src/sagas/close_pane.rs", "\"sh\"", 2,
     "test: unit-test or fixture code, never on a user machine"),
    ("crates/srv/src/server/agent_resources.rs", "\"cmd\"", 2,
     "test: unit-test or fixture code, never on a user machine"),
    ("crates/srv/src/server/agent_resources.rs", "\"sh\"", 2,
     "test: unit-test or fixture code, never on a user machine"),
    ("crates/srv/src/server/cli_handlers.rs", "\"where\"", 2,
     "no-window: calls no_window()/creation_flags in this file"),
    ("crates/srv/src/server/cli_handlers.rs", "\"which\"", 2,
     "not-windows: macOS/Linux/unix-only code path"),
    ("crates/srv/src/server/editor_handlers.rs", "\"explorer.exe\"", 1,
     "gui-program: explorer.exe is GUI-subsystem, on user action only"),
    ("crates/srv/src/server/editor_handlers.rs", "\"open\"", 1,
     "not-windows: macOS/Linux/unix-only code path"),
    ("crates/srv/src/server/editor_handlers.rs", "\"xdg-open\"", 1,
     "not-windows: macOS/Linux/unix-only code path"),
    ("crates/srv/src/server/identity_auth_spawn.rs", "&cli_path", 1,
     "no-window: calls no_window()/creation_flags in this file"),
    ("crates/srv/src/server/identity_auth_spawn.rs", "&spawn_program", 1,
     "no-window: calls no_window()/creation_flags in this file"),
    ("crates/srv/src/server/install_handlers.rs", "if cfg!(windows", 1,
     "no-window: calls no_window()/creation_flags in this file"),
    ("crates/srv/src/server/shell_handlers.rs", "&shell", 1,
     "no-window: calls no_window()/creation_flags in this file"),
    ("crates/srv/src/server/system_install_handlers.rs", "\"winget\"", 1,
     "no-window: calls no_window()/creation_flags in this file"),
    ("crates/srv/src/server/system_install_handlers.rs", "\"brew\"", 1,
     "no-window: calls no_window()/creation_flags in this file"),
    ("crates/srv/src/server/system_install_handlers.rs", "\"apt-cache\"", 1,
     "no-window: calls no_window()/creation_flags in this file"),
    ("crates/srv/src/server/system_install_handlers.rs", "&step.program", 1,
     "no-window: calls no_window()/creation_flags in this file"),
    ("crates/srv/src/server/voice.rs", "&cli", 1,
     "no-window: calls no_window()/creation_flags in this file"),
];

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rs_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// `Command::new(` must not match `AgentCommand::new(`: a hit preceded by an identifier character
/// is a different type. Comment lines are not spawns.
fn line_spawns(line: &str) -> bool {
    if line.trim_start().starts_with("//") {
        return false;
    }
    for marker in SPAWN_MARKERS {
        let mut from = 0;
        while let Some(idx) = line[from..].find(marker) {
            let at = from + idx;
            let prev_is_ident =
                at > 0 && line[..at].chars().next_back().is_some_and(|c| c.is_alphanumeric() || c == '_');
            if !prev_is_ident {
                return true;
            }
            from = at + marker.len();
        }
    }
    false
}

/// The text between `new(` and the first `,` or `)`.
fn program_of(line: &str) -> String {
    let Some(at) = line.find("new(") else { return "?".to_string() };
    let rest = &line[at + 4..];
    let end = rest.find([',', ')']).unwrap_or(rest.len());
    rest[..end].trim().to_string()
}

fn workspace_root() -> PathBuf {
    // CARGO_MANIFEST_DIR is <root>/crates/common.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn actual_inventory() -> BTreeMap<(String, String), usize> {
    let root = workspace_root();
    let mut map: BTreeMap<(String, String), usize> = BTreeMap::new();
    let mut walked = 0;
    for krate in CRATES {
        let mut files = Vec::new();
        rs_files(&root.join("crates").join(krate).join("src"), &mut files);
        walked += files.len();
        for file in files {
            let rel = file.strip_prefix(&root).unwrap_or(&file).to_string_lossy().replace('\\', "/");
            if SKIP_FILES.contains(&rel.as_str()) {
                continue;
            }
            let Ok(src) = std::fs::read_to_string(&file) else { continue };
            for line in src.lines() {
                if line_spawns(line) {
                    *map.entry((rel.clone(), program_of(line))).or_default() += 1;
                }
            }
        }
    }
    assert!(walked > 100, "source walk found only {walked} files; the walk is broken");
    map
}

#[test]
fn the_spawn_surface_matches_the_inventory_exactly() {
    let expected: BTreeMap<(String, String), usize> =
        SPAWN_INVENTORY.iter().map(|(f, p, c, _)| ((f.to_string(), p.to_string()), *c)).collect();
    assert_eq!(expected.len(), SPAWN_INVENTORY.len(), "duplicate (file, program) row in SPAWN_INVENTORY");
    let actual = actual_inventory();

    let mut problems = Vec::new();
    for (key, count) in &actual {
        match expected.get(key) {
            None => problems.push(format!("NEW spawn site: {} -> Command::new({}) x{count}", key.0, key.1)),
            Some(exp) if exp != count => problems.push(format!(
                "COUNT changed: {} -> Command::new({}): inventory says {exp}, source has {count}",
                key.0, key.1
            )),
            Some(_) => {}
        }
    }
    for key in expected.keys() {
        if !actual.contains_key(key) {
            problems.push(format!(
                "GONE: {} -> Command::new({}) is in the inventory but not in the source",
                key.0, key.1
            ));
        }
    }
    assert!(
        problems.is_empty(),
        "The set of process spawns changed. For a new one, call `.no_window()` \
         (agentmux_common::win32::NoWindow) on it and add it to SPAWN_INVENTORY with a classification; \
         if you removed one, drop its row.\n{}",
        problems.join("\n")
    );
}

#[test]
fn every_row_is_classified_and_the_no_window_ones_hold_up() {
    let root = workspace_root();
    for (file, program, _, reason) in SPAWN_INVENTORY {
        assert!(
            CLASSES.iter().any(|c| reason.starts_with(c)),
            "{file} -> {program}: reason must start with one of {CLASSES:?}, got {reason:?}"
        );
        if reason.starts_with("no-window:") || reason.starts_with("mixed:") {
            let src = std::fs::read_to_string(root.join(file)).unwrap_or_default();
            assert!(
                src.contains(".no_window()") || src.contains("creation_flags("),
                "{file} -> {program} is classified `no-window:` but the file never calls `no_window()` or \
                 `creation_flags(`; a console window will flash on Windows"
            );
        }
    }
}
