// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Which installs publish their presence. A normal signed-in desktop does,
//! unless the user turns off "Publish this computer to my devices"
//! (`cloud:publishpresence`, [`setting_on`]). A `task dev` build, a headless
//! srv, an isolated home and a test harness don't by default: each would
//! leave an entry in the account's list of computers that is not a computer
//! anyone uses. [`FORCE_ENV`] turns publishing on for one of those, for
//! testing presence itself; the setting still wins over it.

use crate::backend::rpc_types::PresenceOffReason;
use crate::backend::wconfig::SettingsType;

/// `1` publishes from an install that is off by default (a dev build under
/// test). Not a setting: nobody should come across it by accident.
pub(crate) const FORCE_ENV: &str = "AGENTMUX_PUBLISH_PRESENCE";

/// Set by a test harness that starts srv. `AGENTMUX_DISABLE_CLOUD_SUBSCRIBER`
/// (the srv integration tests) counts too.
pub(crate) const TEST_HARNESS_ENV: &str = "AGENTMUX_TEST_HARNESS";

/// The facts about this install that decide whether it publishes by default,
/// read once at start.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Install {
    pub headless: bool,
    pub home_override: bool,
    pub channel: String,
    pub test_harness: bool,
    pub forced: bool,
}

impl Install {
    /// This process's install. `headless` is srv's own `--headless`.
    pub(crate) fn from_env(headless: bool) -> Self {
        Self::from_vars(headless, crate::backend::reactive::registry::local_channel_id(), |name| {
            std::env::var(name).ok()
        })
    }

    /// [`Self::from_env`] over any variables, so tests don't touch the
    /// process's own.
    pub(crate) fn from_vars(headless: bool, channel: String, var: impl Fn(&str) -> Option<String>) -> Self {
        Self {
            headless,
            home_override: var("AGENTMUX_HOME_OVERRIDE").is_some_and(|v| !v.is_empty()),
            channel,
            // Present at all, like `AGENTMUX_DISABLE_CLOUD_SUBSCRIBER` itself.
            test_harness: var(TEST_HARNESS_ENV).is_some() || var("AGENTMUX_DISABLE_CLOUD_SUBSCRIBER").is_some(),
            forced: var(FORCE_ENV).as_deref() == Some("1"),
        }
    }

    /// Why this install doesn't publish by default, if it doesn't.
    fn default_off(&self) -> Option<PresenceOffReason> {
        if self.test_harness {
            Some(PresenceOffReason::TestHarness)
        } else if self.headless {
            Some(PresenceOffReason::Headless)
        } else if self.home_override {
            Some(PresenceOffReason::IsolatedHome)
        } else if self.channel.starts_with("dev-") {
            Some(PresenceOffReason::DevBuild)
        } else {
            None
        }
    }

    /// Why nothing is published, given the setting; `None` to publish. The
    /// setting turned off always wins; [`FORCE_ENV`] overrides only the
    /// install's own default.
    pub(crate) fn off_reason(&self, setting_on: bool) -> Option<PresenceOffReason> {
        if !setting_on {
            return Some(PresenceOffReason::Setting);
        }
        if self.forced {
            return None;
        }
        self.default_off()
    }
}

/// "Publish this computer to my devices" as `settings.json` has it. Absent
/// means on. The key is written out at the read so the settings-defaults
/// check (`settings-defaults.test.ts`) finds this default.
pub(crate) fn setting_on(settings: &SettingsType) -> bool {
    settings.extra.get("cloud:publishpresence").and_then(|v| v.as_bool()).unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn normal() -> Install {
        Install { channel: "stable".into(), ..Install::default() }
    }

    #[test]
    fn a_normal_install_publishes_and_the_others_are_off_by_default() {
        let cases: [(&str, Install, Option<PresenceOffReason>); 7] = [
            ("stable", normal(), None),
            ("a local package build", Install { channel: "agentmux-0-59-12-k3j9".into(), ..normal() }, None),
            ("a dev build", Install { channel: "dev-feature-x".into(), ..normal() }, Some(PresenceOffReason::DevBuild)),
            ("headless", Install { headless: true, ..normal() }, Some(PresenceOffReason::Headless)),
            ("an isolated home", Install { home_override: true, ..normal() }, Some(PresenceOffReason::IsolatedHome)),
            ("a test harness", Install { test_harness: true, ..normal() }, Some(PresenceOffReason::TestHarness)),
            (
                "a headless dev build under test",
                Install { headless: true, test_harness: true, channel: "dev-x".into(), ..normal() },
                Some(PresenceOffReason::TestHarness),
            ),
        ];
        for (what, install, expect) in cases {
            assert_eq!(install.off_reason(true), expect, "{what}");
        }
    }

    #[test]
    fn the_setting_wins_and_the_override_only_lifts_the_default() {
        assert_eq!(normal().off_reason(false), Some(PresenceOffReason::Setting));
        let dev = Install { channel: "dev-x".into(), ..normal() };
        let forced = Install { forced: true, ..dev.clone() };
        assert_eq!(dev.off_reason(true), Some(PresenceOffReason::DevBuild));
        assert_eq!(forced.off_reason(true), None, "a dev build under test publishes");
        assert_eq!(forced.off_reason(false), Some(PresenceOffReason::Setting), "the setting still wins");
        let everything = Install { headless: true, home_override: true, test_harness: true, ..forced };
        assert_eq!(everything.off_reason(true), None);
    }

    #[test]
    fn the_setting_is_on_unless_it_is_false() {
        let read = |json: serde_json::Value| setting_on(&serde_json::from_value::<SettingsType>(json).unwrap());
        assert!(read(serde_json::json!({})));
        assert!(read(serde_json::json!({ "cloud:publishpresence": true })));
        assert!(!read(serde_json::json!({ "cloud:publishpresence": false })));
        assert!(read(serde_json::json!({ "cloud:publishpresence": "no" })), "not a boolean: the default");
    }

    fn with(vars: &[(&str, &str)]) -> Install {
        let vars: Vec<(String, String)> = vars.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        Install::from_vars(false, "stable".into(), move |name| {
            vars.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone())
        })
    }

    #[test]
    fn the_environment_is_read() {
        assert_eq!(with(&[]), normal());
        assert!(Install::from_vars(true, "stable".into(), |_| None).headless);
        assert!(with(&[(TEST_HARNESS_ENV, "1")]).test_harness);
        assert!(with(&[("AGENTMUX_DISABLE_CLOUD_SUBSCRIBER", "")]).test_harness, "present is enough");
        assert!(with(&[("AGENTMUX_HOME_OVERRIDE", "/tmp/h")]).home_override);
        assert!(!with(&[("AGENTMUX_HOME_OVERRIDE", "")]).home_override, "empty is unset");
        assert!(with(&[(FORCE_ENV, "1")]).forced);
        assert!(!with(&[(FORCE_ENV, "true")]).forced, "only 1");
    }
}
