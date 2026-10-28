// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A srv given a data folder of its own must not write into the real shared folder
//! by accident.
//!
//! The shared folder (`shared/`: agent definitions, accounts, the shared store) follows
//! the AgentMux home, not the data folder. A srv started with only a data folder
//! outside the home (`AGENTMUX_DATA_DIR` or `--wavedata`) therefore kept its own
//! databases in that folder and still wrote the agents it created into the real
//! `~/.agentmux/shared`. Where shared data goes has to be said: by a home override
//! (an isolated home) or by `AGENTMUX_SHARED_DIR` (the launcher always exports it).

use std::path::Path;

/// Why srv must not start, or `None`. `home` is the AgentMux home as it was when srv started;
/// `home_override` and `shared_dir_set` say whether `AGENTMUX_HOME_OVERRIDE` and
/// `AGENTMUX_SHARED_DIR` were given.
pub(super) fn refusal(
    data_home: &Path,
    home: Option<&Path>,
    home_override: bool,
    shared_dir_set: bool,
) -> Option<String> {
    if data_home.as_os_str().is_empty() || home_override || shared_dir_set {
        return None;
    }
    let home = home?;
    if data_home.starts_with(home) {
        return None;
    }
    Some(format!(
        "the data folder {} is outside the AgentMux home {}, so the shared folder ({}) would be the real one. \
         Start it on an isolated home (AGENTMUX_HOME_OVERRIDE=<folder>), \
         or set AGENTMUX_SHARED_DIR to the shared folder to use on purpose.",
        data_home.display(),
        home.display(),
        home.join("shared").display(),
    ))
}

/// [`refusal`] for this process; exits with the reason on stderr when it applies.
pub(super) fn refuse_unlisted_shared_root(data_home: &Path) {
    let set = |name| std::env::var_os(name).is_some_and(|v| !v.is_empty());
    let home = crate::srv_info::agentmux_home();
    if let Some(why) = refusal(
        data_home,
        home.as_deref(),
        set("AGENTMUX_HOME_OVERRIDE"),
        set("AGENTMUX_SHARED_DIR"),
    ) {
        tracing::error!("startup: {why}");
        eprintln!("agentmux-srv: {why}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn home() -> PathBuf {
        PathBuf::from("/home/me/.agentmux")
    }

    #[test]
    fn a_data_folder_outside_the_home_with_nothing_said_about_shared_refuses() {
        let why = refusal(Path::new("/tmp/iso"), Some(&home()), false, false).expect("refuses");
        assert!(
            why.contains("/tmp/iso")
                && why.contains("AGENTMUX_HOME_OVERRIDE")
                && why.contains("AGENTMUX_SHARED_DIR"),
            "{why}"
        );
    }

    #[test]
    fn a_home_override_or_an_explicit_shared_dir_says_where_shared_data_goes() {
        assert_eq!(
            refusal(Path::new("/tmp/iso"), Some(&home()), true, false),
            None
        );
        assert_eq!(
            refusal(Path::new("/tmp/iso"), Some(&home()), false, true),
            None
        );
    }

    #[test]
    fn a_data_folder_inside_the_home_or_none_at_all_is_fine() {
        assert_eq!(
            refusal(
                &home().join("channels/stable/versions/1/data"),
                Some(&home()),
                false,
                false
            ),
            None
        );
        assert_eq!(refusal(Path::new(""), Some(&home()), false, false), None);
        assert_eq!(refusal(Path::new("/tmp/iso"), None, false, false), None);
    }

    #[test]
    fn a_folder_that_only_shares_the_homes_name_prefix_is_outside_it() {
        assert!(refusal(
            Path::new("/home/me/.agentmux-iso/data"),
            Some(&home()),
            false,
            false
        )
        .is_some());
    }
}
