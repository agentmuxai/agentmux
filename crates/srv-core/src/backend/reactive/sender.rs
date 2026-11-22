//! Whether a jekt's sender name can be shown as an agent's.
//!
//! AgentMux's own senders (`cron`, `loop`, `github-consumer`, ...) are reserved agent names. A message that came in
//! over the LAN or the WAN and is not signed by AgentMux's own relay could carry any of them, so there such a name is
//! shown the way a name that is no agent id is shown: with a leading `?` and no reply hint.

use super::sanitize::validate_agent_id;
use crate::backend::reserved_names;

/// True when `name` may be shown, and replied to, as an agent id. `reagent_verified` is the relay's signature check
/// (`Some(true)` for a message AgentMux's own consumers signed).
pub fn is_shown_as_agent(name: &str, delivery_tier: &str, reagent_verified: Option<bool>) -> bool {
    if !validate_agent_id(name) {
        return false;
    }
    let from_afar = matches!(delivery_tier, "lan" | "wan");
    !(from_afar && reagent_verified != Some(true) && reserved_names::check(name).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reserved_name_from_afar_without_the_relay_signature_is_not_shown_as_an_agent() {
        for tier in ["lan", "wan"] {
            for name in ["cron", "github-consumer", "Muxreview", "cr0n", "operator", "swarm-2"] {
                assert!(!is_shown_as_agent(name, tier, None), "{name} over {tier}");
                assert!(!is_shown_as_agent(name, tier, Some(false)), "{name} over {tier}");
            }
        }
    }

    #[test]
    fn the_relay_signature_and_the_host_keep_the_names_working() {
        assert!(is_shown_as_agent("github-consumer", "wan", Some(true)));
        assert!(is_shown_as_agent("cron", "host", None));
        assert!(is_shown_as_agent("loop", "channel", None));
    }

    #[test]
    fn an_ordinary_sender_is_unchanged() {
        for tier in ["host", "channel", "lan", "wan"] {
            assert!(is_shown_as_agent("agentx", tier, None), "{tier}");
        }
        assert!(!is_shown_as_agent("not an id", "host", None));
    }
}
