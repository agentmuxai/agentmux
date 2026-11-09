// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `agentmux --version` (or `-V`): print the version and exit 0, starting nothing.
//!
//! Without this the flag fell through to the normal start: the launcher brought up srv and the
//! CEF host, whose own `--version` handling printed the Chromium version, so a person checking
//! an install (the install page says to) saw no AgentMux version at all, and on Ubuntu the
//! sandbox check stopped it first. The deb and rpm wrappers, the tarball's `agentmux.sh` and the
//! AppImage's AppRun all `exec` this launcher with their arguments, so one check here covers
//! them (AppRun answers it earlier still, before it unpacks anything).

use std::io::Write;

/// Only the first argument counts: `agentmux --version`, not a `--version` somewhere in a
/// command line that means something else.
fn requested(args: &[String]) -> bool {
    matches!(args.get(1).map(String::as_str), Some("--version" | "-V"))
}

/// The one line printed. A local build also names its build label, which is how two builds of
/// the same version tell apart; a release build's label is its version, so nothing is added.
fn line(version: &str, label: Option<&str>) -> String {
    match label.filter(|l| !l.is_empty() && *l != version) {
        Some(label) => format!("AgentMux {version} (build {label})"),
        None => format!("AgentMux {version}"),
    }
}

/// `Some(exit code)` after printing the version when the arguments ask for it; `None` otherwise.
pub fn run_if_requested(args: &[String]) -> Option<i32> {
    if !requested(args) {
        return None;
    }
    // A closed pipe (`agentmux --version | head -0`) is not an error worth a panic.
    let _ = writeln!(
        std::io::stdout(),
        "{}",
        line(env!("CARGO_PKG_VERSION"), option_env!("AGENTMUX_BUILD_LABEL"))
    );
    Some(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        std::iter::once("agentmux").chain(a.iter().copied()).map(String::from).collect()
    }

    #[test]
    fn long_and_short_flag_ask_for_the_version() {
        assert!(requested(&args(&["--version"])));
        assert!(requested(&args(&["-V"])));
    }

    #[test]
    fn nothing_else_does() {
        assert!(!requested(&args(&[])));
        assert!(!requested(&args(&["--background"])));
        assert!(!requested(&args(&["--diag", "sagas", "--version"])));
        assert!(!requested(&args(&["--version-info"])));
        assert!(!requested(&args(&["-v"])));
    }

    #[test]
    fn a_release_prints_only_its_version() {
        assert_eq!(line("0.60.21", None), "AgentMux 0.60.21");
        assert_eq!(line("0.60.21", Some("")), "AgentMux 0.60.21");
        assert_eq!(line("0.60.21", Some("0.60.21")), "AgentMux 0.60.21");
    }

    #[test]
    fn a_local_build_adds_its_label() {
        assert_eq!(line("0.60.21", Some("0.60.21-dev.1109")), "AgentMux 0.60.21 (build 0.60.21-dev.1109)");
    }

    #[test]
    fn the_flag_exits_zero_and_other_arguments_do_not() {
        assert_eq!(run_if_requested(&args(&["--version"])), Some(0));
        assert_eq!(run_if_requested(&args(&["--background"])), None);
    }
}
