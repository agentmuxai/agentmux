// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Where a channel's `settings.json` lives: one answer for srv, the host and
//! the launcher (#1903).
//!
//! srv loads and watches `<settings dir>/settings.json`, where the settings
//! dir is:
//!
//! 1. `AGENTMUX_SETTINGS_DIR`, when set;
//! 2. on an isolated channel (every channel but `stable`, see
//!    [`crate::isolated_settings_enabled`]), the channel's config dir itself:
//!    `<root>/channels/<ch>/config`;
//! 3. on a global channel (`stable`), two levels above the config dir:
//!    `<root>/channels`, the file every global channel shares.
//!
//! The config dir already sits under the resolved AgentMux root, so a
//! `AGENTMUX_HOME_OVERRIDE` carries through. The host and the launcher used to
//! answer this themselves: the host seeded `channels/stable/config/settings.json`,
//! which srv never reads on stable, and its window settings fell back to the
//! real `~/.agentmux/channels/settings.json` under an override.

use std::path::{Path, PathBuf};

use crate::data_paths::IsolatedSettingsReason;
use crate::DataPaths;

/// The settings file's name inside its dir.
pub const SETTINGS_FILE: &str = "settings.json";

/// Explicit settings dir; wins over everything else.
pub const SETTINGS_DIR_ENV: &str = "AGENTMUX_SETTINGS_DIR";

/// `1` isolates the channel's settings, any other value shares them.
pub const ISOLATED_SETTINGS_ENV: &str = "AGENTMUX_ISOLATED_SETTINGS";

/// The isolation rule, with its inputs passed in: `opt` is the value of
/// `AGENTMUX_ISOLATED_SETTINGS` (if set) and `channel` the channel name.
pub fn isolated_settings_reason_for(opt: Option<&str>, channel: Option<&str>) -> IsolatedSettingsReason {
    match (opt, channel) {
        (Some("1"), _) => IsolatedSettingsReason::ExplicitOptIn,
        (Some(_), _) => IsolatedSettingsReason::ExplicitOptOut,
        (None, Some(ch)) if ch != "stable" => IsolatedSettingsReason::ChannelDefaultIsolated,
        (None, _) => IsolatedSettingsReason::ChannelDefaultGlobal,
    }
}

/// The settings dir from its inputs, or `None` when neither an explicit dir
/// nor a config dir (with two parents, for a global channel) is known. Empty
/// paths count as unset.
pub fn settings_dir_for(explicit: Option<&Path>, config_dir: Option<&Path>, isolated: bool) -> Option<PathBuf> {
    let nonempty = |p: &&Path| !p.as_os_str().is_empty();
    if let Some(dir) = explicit.filter(nonempty) {
        return Some(dir.to_path_buf());
    }
    let config_dir = config_dir.filter(nonempty)?;
    if isolated {
        return Some(config_dir.to_path_buf());
    }
    config_dir.parent().and_then(Path::parent).map(Path::to_path_buf)
}

/// A non-empty env var as a path.
pub(crate) fn env_dir(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from)
}

impl DataPaths {
    /// The dir holding this channel's `settings.json`: the one srv reads
    /// when started with these paths.
    pub fn settings_dir(&self) -> PathBuf {
        let opt = std::env::var(ISOLATED_SETTINGS_ENV).ok();
        let isolated = isolated_settings_reason_for(opt.as_deref(), Some(&self.channel)).is_isolated();
        settings_dir_for(env_dir(SETTINGS_DIR_ENV).as_deref(), Some(&self.config_dir), isolated)
            .unwrap_or_else(|| self.home_dir.clone())
    }

    /// This channel's `settings.json`.
    pub fn settings_file(&self) -> PathBuf {
        self.settings_dir().join(SETTINGS_FILE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RuntimeMode, TEST_ENV_LOCK};

    #[test]
    fn stable_shares_the_file_two_levels_above_its_config_dir() {
        let config = Path::new("/o/channels/stable/config");
        assert_eq!(settings_dir_for(None, Some(config), false), Some(PathBuf::from("/o/channels")));
    }

    #[test]
    fn an_isolated_channel_uses_its_config_dir() {
        let config = Path::new("/o/channels/local-x/config");
        assert_eq!(settings_dir_for(None, Some(config), true), Some(config.to_path_buf()));
    }

    #[test]
    fn an_explicit_dir_wins_and_empty_values_are_unset() {
        let config = Path::new("/o/channels/stable/config");
        assert_eq!(settings_dir_for(Some(Path::new("/x")), Some(config), false), Some(PathBuf::from("/x")));
        assert_eq!(settings_dir_for(Some(Path::new("")), Some(config), true), Some(config.to_path_buf()));
        assert_eq!(settings_dir_for(None, Some(Path::new("")), true), None);
        assert_eq!(settings_dir_for(None, None, false), None);
    }

    #[test]
    fn a_global_config_dir_without_two_parents_has_no_settings_dir() {
        assert_eq!(settings_dir_for(None, Some(Path::new("config")), false), None);
    }

    #[test]
    fn isolation_rule() {
        use IsolatedSettingsReason::*;
        assert_eq!(isolated_settings_reason_for(Some("1"), Some("stable")), ExplicitOptIn);
        assert_eq!(isolated_settings_reason_for(Some("0"), Some("local-x")), ExplicitOptOut);
        assert_eq!(isolated_settings_reason_for(Some(""), Some("local-x")), ExplicitOptOut);
        assert_eq!(isolated_settings_reason_for(None, Some("local-x")), ChannelDefaultIsolated);
        assert_eq!(isolated_settings_reason_for(None, Some("stable")), ChannelDefaultGlobal);
        assert_eq!(isolated_settings_reason_for(None, None), ChannelDefaultGlobal);
    }

    /// Env the resolution reads, cleared on entry and on drop (even on a
    /// panic), under the crate-wide env lock.
    struct EnvGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    const VARS: &[&str] =
        &["AGENTMUX_HOME_OVERRIDE", "AGENTMUX_DATA_HOME", "AGENTMUX_CHANNEL", SETTINGS_DIR_ENV, ISOLATED_SETTINGS_ENV];

    impl EnvGuard {
        fn new() -> Self {
            let lock = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            VARS.iter().for_each(|v| std::env::remove_var(v));
            EnvGuard { _lock: lock }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            VARS.iter().for_each(|v| std::env::remove_var(v));
        }
    }

    fn resolve_in(home: &Path, channel: &str) -> DataPaths {
        std::env::set_var("AGENTMUX_HOME_OVERRIDE", home);
        std::env::set_var("AGENTMUX_CHANNEL", channel);
        DataPaths::resolve("0.0.0", &RuntimeMode::Installed).expect("resolve")
    }

    #[test]
    fn stable_under_a_home_override_reads_the_overrides_shared_file() {
        let _env = EnvGuard::new();
        let tmp = tempfile::TempDir::new().unwrap();
        let paths = resolve_in(tmp.path(), "stable");
        assert_eq!(paths.settings_file(), tmp.path().join("channels").join(SETTINGS_FILE));
    }

    #[test]
    fn an_isolated_channel_under_a_home_override_reads_its_own_file() {
        let _env = EnvGuard::new();
        let tmp = tempfile::TempDir::new().unwrap();
        let paths = resolve_in(tmp.path(), "local-x");
        assert_eq!(paths.settings_file(), paths.config_dir.join(SETTINGS_FILE));
        assert!(paths.settings_file().starts_with(tmp.path()));
    }

    #[test]
    fn the_explicit_dir_and_opt_in_apply_to_resolved_paths() {
        let _env = EnvGuard::new();
        let tmp = tempfile::TempDir::new().unwrap();
        let paths = resolve_in(tmp.path(), "stable");
        std::env::set_var(ISOLATED_SETTINGS_ENV, "1");
        assert_eq!(paths.settings_dir(), paths.config_dir);
        std::env::set_var(SETTINGS_DIR_ENV, tmp.path().join("explicit"));
        assert_eq!(paths.settings_dir(), tmp.path().join("explicit"));
    }
}
