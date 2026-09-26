// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Whether this instance runs CEF's remote-debugging (CDP) TCP server, and on
//! which port (#3681).
//!
//! The server has no authentication and listens on loopback, which every
//! account on the machine shares: another local user could list this
//! instance's pages and run script in the AgentMux UI. Nothing in the app
//! needs it any more — the browser API drives CDP in-process
//! (`browser_api::cdp`) and the in-app "Inspect Element" opens CEF's native
//! DevTools — so **release builds don't start it** unless asked:
//!
//! - `AGENTMUX_CDP_PORT` unset (or empty) → release: off; dev: on, 9223.
//! - `AGENTMUX_CDP_PORT=<1024..=65535>` → on, preferring that port.
//! - `AGENTMUX_CDP_PORT=1|on|true|yes|auto` → on, preferring 9222 (9223 dev).
//! - `AGENTMUX_CDP_PORT=0|off|false|no` → off (dev builds too).
//!
//! It is the same variable `scripts/ui-screenshots` already reads for the port
//! to connect to, so setting it once for both sides just works. Dev builds
//! keep the server by default: that is where agents and test harnesses drive
//! the UI over CDP, via the port published in `authkey.dev`.
//!
//! "Preferring" because several instances run side by side: when the port is
//! taken, `lib.rs` falls back to an OS-assigned one and publishes the actual
//! port (see SPEC_INSTANCE_DISCOVERY_FOR_TOOLING_2026_09_17.md).

pub const CDP_PORT_ENV: &str = "AGENTMUX_CDP_PORT";

const RELEASE_DEFAULT_PORT: u16 = 9222;
const DEV_DEFAULT_PORT: u16 = 9223;

/// The port to *prefer* for the CDP server, or `None` to not start one.
/// `env` is the raw value of [`CDP_PORT_ENV`].
pub fn requested_cdp_port(is_dev: bool, env: Option<&str>) -> Option<u16> {
    let default = if is_dev { DEV_DEFAULT_PORT } else { RELEASE_DEFAULT_PORT };
    let value = env.map(str::trim).filter(|v| !v.is_empty());
    let Some(value) = value else {
        return is_dev.then_some(default);
    };
    match value.to_ascii_lowercase().as_str() {
        "0" | "off" | "false" | "no" => None,
        "1" | "on" | "true" | "yes" | "auto" => Some(default),
        other => match other.parse::<u16>() {
            Ok(port) if port >= 1024 => Some(port),
            // Set to something we don't understand: the user clearly meant to
            // turn it on, so do that on the default port and say so.
            _ => {
                tracing::warn!(
                    "{CDP_PORT_ENV}={value:?} is not a port (1024-65535) or on/off; \
                     enabling the CDP server on its default port"
                );
                Some(default)
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::requested_cdp_port as req;

    #[test]
    fn release_builds_are_off_by_default() {
        assert_eq!(req(false, None), None);
        assert_eq!(req(false, Some("")), None);
        assert_eq!(req(false, Some("   ")), None);
    }

    #[test]
    fn dev_builds_are_on_by_default() {
        assert_eq!(req(true, None), Some(9223));
        assert_eq!(req(true, Some("")), Some(9223));
    }

    #[test]
    fn a_port_number_opts_in_on_that_port() {
        assert_eq!(req(false, Some("9222")), Some(9222));
        assert_eq!(req(false, Some(" 40123 ")), Some(40123));
        assert_eq!(req(true, Some("40123")), Some(40123));
    }

    #[test]
    fn on_words_opt_in_on_the_default_port() {
        for v in ["1", "on", "ON", "true", "yes", "auto"] {
            assert_eq!(req(false, Some(v)), Some(9222), "{v}");
            assert_eq!(req(true, Some(v)), Some(9223), "{v}");
        }
    }

    #[test]
    fn off_words_turn_it_off_even_in_dev() {
        for v in ["0", "off", "Off", "false", "no"] {
            assert_eq!(req(false, Some(v)), None, "{v}");
            assert_eq!(req(true, Some(v)), None, "{v}");
        }
    }

    #[test]
    fn nonsense_or_privileged_ports_enable_on_the_default_port() {
        assert_eq!(req(false, Some("banana")), Some(9222));
        assert_eq!(req(false, Some("80")), Some(9222));
        assert_eq!(req(false, Some("70000")), Some(9222));
    }
}
