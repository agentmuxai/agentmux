// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A dev build started by hand must not run on the real AgentMux home.
//!
//! `task dev` gives each dev build a data home of its own (`scripts/dev-home.mjs`) and says so in
//! `AGENTMUX_DEV_HOME_MODE` (`isolated` or `real`) and `AGENTMUX_HOME_OVERRIDE`. A launcher or host
//! started by hand from a dev output folder (`dist/cef-dev*`, `target/debug`, `target/release`)
//! has neither, resolves `~/.agentmux`, creates its `dev/<branch>/<clone>` folders there and
//! starts a srv on the person's own home. This refuses that start with one plain line.
//!
//! The way in on purpose is `AGENTMUX_DEV_USE_REAL_HOME=1`, the same switch `task dev` honours.

use std::path::Path;

const MODE: &str = "AGENTMUX_DEV_HOME_MODE";
const OVERRIDE: &str = "AGENTMUX_HOME_OVERRIDE";
const USE_REAL_HOME: &str = "AGENTMUX_DEV_USE_REAL_HOME";

/// What the guard reads from the environment, as set values only (an empty value is unset).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DevHomeEnv {
    pub mode: Option<String>,
    pub home_override: Option<String>,
    pub use_real_home: bool,
}

impl DevHomeEnv {
    pub fn from_process() -> Self {
        let get = |name| {
            std::env::var(name)
                .ok()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        Self {
            mode: get(MODE),
            home_override: get(OVERRIDE),
            use_real_home: get(USE_REAL_HOME).as_deref() == Some("1"),
        }
    }
}

/// Why a dev build must not start, or `None`. `real_home` is the person's own `~/.agentmux`.
pub fn refusal(env: &DevHomeEnv, real_home: Option<&Path>) -> Option<String> {
    if env.use_real_home || matches!(env.mode.as_deref(), Some("isolated" | "real")) {
        return None;
    }
    let Some(given) = env.home_override.as_deref() else {
        return Some(format!(
            "this dev build has no isolated data home, so it would run on your real AgentMux home. \
             Start it with `task dev`, or set {OVERRIDE}=<empty folder>; to use the real home on purpose set {USE_REAL_HOME}=1."
        ));
    };
    let is_real = real_home.is_some_and(|real| same_folder(Path::new(given), real));
    is_real.then(|| {
        format!(
            "{OVERRIDE} names your real AgentMux home; to use it on purpose set {USE_REAL_HOME}=1."
        )
    })
}

fn same_folder(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| {
        p.to_string_lossy()
            .replace('\\', "/")
            .trim_end_matches('/')
            .to_lowercase()
    };
    norm(a) == norm(b)
}

/// [`refusal`] for this process when it runs from a dev build folder. `exe_dir` is the binary's folder.
pub fn refusal_for_process(exe_dir: &Path) -> Option<String> {
    if !crate::is_dev_build_exe(exe_dir) {
        return None;
    }
    refusal(
        &DevHomeEnv::from_process(),
        dirs::home_dir().map(|h| h.join(".agentmux")).as_deref(),
    )
}

/// Exits with code 2 and one line on stderr when [`refusal_for_process`] applies. Call first, before
/// anything creates a folder, opens a store or starts a child.
pub fn refuse_or_continue(exe_dir: &Path) {
    if let Some(why) = refusal_for_process(exe_dir) {
        eprintln!("agentmux: {why}");
        std::process::exit(2);
    }
}

/// [`refuse_or_continue`] for the folder this binary runs from.
pub fn refuse_or_continue_current_exe() {
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
    {
        refuse_or_continue(&dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn real() -> &'static Path {
        Path::new("C:/Users/me/.agentmux")
    }

    fn env(mode: Option<&str>, home_override: Option<&str>, use_real_home: bool) -> DevHomeEnv {
        DevHomeEnv {
            mode: mode.map(Into::into),
            home_override: home_override.map(Into::into),
            use_real_home,
        }
    }

    #[test]
    fn a_start_with_nothing_said_about_the_home_is_refused_with_what_to_run() {
        let why = refusal(&env(None, None, false), Some(real())).expect("refuses");
        assert!(
            why.contains("task dev") && why.contains(OVERRIDE) && why.contains(USE_REAL_HOME),
            "{why}"
        );
        assert!(!why.contains('\n'), "one line");
    }

    #[test]
    fn task_dev_and_an_isolated_override_start() {
        assert_eq!(
            refusal(
                &env(Some("isolated"), Some("C:/clone/.dev-home/5300"), false),
                Some(real())
            ),
            None
        );
        assert_eq!(
            refusal(&env(None, Some("C:/tmp/iso"), false), Some(real())),
            None
        );
        assert_eq!(refusal(&env(Some("real"), None, false), Some(real())), None);
    }

    #[test]
    fn the_real_home_needs_the_explicit_switch() {
        assert_eq!(refusal(&env(None, None, true), Some(real())), None);
        let named = refusal(
            &env(None, Some("c:\\Users\\me\\.agentmux\\"), false),
            Some(real()),
        );
        assert!(named.is_some_and(|w| w.contains(USE_REAL_HOME)));
        assert_eq!(
            refusal(
                &env(None, Some("C:/Users/me/.agentmux"), true),
                Some(real())
            ),
            None
        );
    }

    #[test]
    fn an_empty_or_odd_value_is_not_a_choice() {
        assert!(refusal(&env(Some("whatever"), None, false), Some(real())).is_some());
    }

    #[test]
    fn only_a_dev_build_folder_is_guarded() {
        assert_eq!(
            refusal_for_process(Path::new("C:/Program Files/AgentMux")),
            None
        );
    }
}
