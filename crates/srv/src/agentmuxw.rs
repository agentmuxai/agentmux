// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `agentmuxw`: what the Windows logon task runs. A program of the console subsystem that a task starts at sign-in puts
//! a console window on the person's screen; this one is of the graphical subsystem, so it never has one, and what it
//! does is start `agentmux --channel <channel> run` beside it with no window and wait for it, so the task is running
//! as long as the server is. It takes `--channel <name>` and nothing else, runs only the `agentmux.exe` in its own
//! folder, and is built in this repository and shipped in the package (it is never fetched).

#![cfg_attr(windows, windows_subsystem = "windows")]

/// `--channel <name>` and nothing else: the arguments to pass on, or `None` for any other.
#[cfg(any(windows, test))]
fn channel_args(args: &[std::ffi::OsString]) -> Option<Vec<std::ffi::OsString>> {
    match args {
        [] => Some(Vec::new()),
        [flag, name] if flag == "--channel" && !name.is_empty() && !name.to_string_lossy().starts_with('-') => Some(args.to_vec()),
        _ => None,
    }
}

#[cfg(windows)]
fn main() {
    use agentmux_common::win32::NoWindow;
    use std::process::Stdio;
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    let Some(pass_on) = channel_args(&args) else { std::process::exit(2) };
    let Some(agentmux) = std::env::current_exe().ok().and_then(|exe| exe.parent().map(|dir| dir.join("agentmux.exe"))).filter(|p| p.is_file()) else {
        std::process::exit(3)
    };
    let status = std::process::Command::new(agentmux).args(&pass_on).arg("run").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).no_window().status();
    std::process::exit(match status {
        Ok(s) => s.code().unwrap_or(1),
        Err(_) => 3,
    });
}

#[cfg(not(windows))]
fn main() {
    eprintln!("agentmuxw is the Windows logon task's program; use `agentmux` here");
    std::process::exit(2);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    fn os(parts: &[&str]) -> Vec<OsString> {
        parts.iter().map(OsString::from).collect()
    }

    #[test]
    fn only_a_channel_is_passed_on() {
        assert_eq!(channel_args(&os(&[])), Some(Vec::new()));
        assert_eq!(channel_args(&os(&["--channel", "headless"])), Some(os(&["--channel", "headless"])));
        for refused in [&["run"][..], &["--channel"], &["--channel", ""], &["--channel", "--srv-bin"], &["--channel", "a", "stop"], &["--srv-bin", "x.exe"], &["stop"]] {
            assert_eq!(channel_args(&os(refused)), None, "{refused:?}");
        }
    }
}
