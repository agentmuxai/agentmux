// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Whether srv reconnects to MuxBus at launch with the stored sign-in, or
//! waits for the user to sign in (moved out of `data_paths.rs`, which holds
//! the auth and settings isolation flags this one deliberately stays apart
//! from).

/// Skip the automatic, EAGER MuxBus cloud-session reconnect
/// (`CloudSubscriber::init_global`, called at boot) on a randomized
/// per-build local-package channel. See
/// `docs/retro/retro-macos-0560-stale-cef-cache-launch-crash-2026-09-16.md`
/// for the incident this closes.
///
/// Without this flag, `CloudSubscriber::init_global`
/// (`crates/srv/src/bootstrap/background.rs`) runs on every launch (unless
/// `AGENTMUX_DISABLE_CLOUD_SUBSCRIBER` is set) and performs a real, synchronous
/// OS-keychain read of the single global `muxbus:global` credential
/// almost immediately. On macOS that read requires interactive OS consent
/// the first time a given code signature touches it — and every local
/// `task package:macos`/`task package`/`task package:linux` build bakes a
/// brand-new, randomized per-build channel (and, on macOS, a bundle
/// identifier derived from it) into the binary. So every fresh local
/// build looks like a never-before-seen app to the Keychain, and the same
/// already-`Always Allow`'d credential prompts again on every single
/// local rebuild — indefinitely, since a new random channel is minted
/// every time.
///
/// **This gates ONLY the one-shot eager call at boot** — not every future
/// MuxBus interaction on that channel. `muxbus.status` (reagentx P0/P1 on
/// PR #3248, round 2) instead checks whether
/// `muxbus::cloud_subscriber::get_global_subscriber()` has been
/// initialized — `None` until either this flag let boot initialize it
/// (stable/dev channels), or a user explicitly completes `muxbus.login`
/// on an isolated channel, which lazily initializes it right there. That
/// split is what makes "only prompt once the user actually enables
/// MuxBus" true: skipping the eager boot call must not permanently wall
/// off every other MuxBus code path on that process for its whole
/// lifetime, or an explicit login would "succeed" while silently never
/// opening a WebSocket (reagentx P1, round 2) and the status bar's
/// 60-second poll would keep hitting Keychain regardless of this flag
/// (reagentx P0, round 2).
///
/// Resolution order:
/// 1. `AGENTMUX_ISOLATED_MUXBUS=1` / `=0` — explicit override, always wins.
/// 2. Otherwise, defaults to isolated (skip the eager reconnect) for a
///    **local package channel on macOS** — `AGENTMUX_CHANNEL` is set, isn't
///    `"stable"`, and doesn't start with `"dev-"`, and the keychain asks
///    consent per code signature ([`keychain_prompts_per_build`]). On Windows
///    and Linux the stored sign-in is read without any prompt, so a local
///    channel reconnects like any other: skipping it there only signed MuxBus
///    out on every restart of a local build, and on every upgrade of one
///    (the release it upgrades to keeps its `local-…` channel), which stopped
///    review and CI notices until someone noticed (seen on the operator's
///    0.59.31 → 0.59.32 upgrade, 2026-10-18). `stable` is the real
///    release channel; `dev-<branch>[-<clone-id>]` (`RuntimeMode::Dev`'s
///    channel format, below) is `task dev`'s **stable, per-branch**
///    channel name — reused across every `task dev`
///    invocation on that branch, unlike a package build's channel, which
///    mints a brand-new random hash on literally every single build. A
///    dev session therefore doesn't have the "never-before-seen app to
///    Keychain every time" problem this flag exists to solve, and
///    isolating it anyway would be a pure regression (reagentx P1 on PR
///    #3248, round 1 + round 2: dev sessions losing automatic MuxBus
///    reconnect, and thus WAN notifications, for the entire session).
///    This is a **deliberate narrowing** relative to
///    [`crate::isolated_auth_enabled`]/[`crate::isolated_settings_enabled`], which
///    isolate `dev-*` too — those exist to make local testing exercise
///    the real accounts/settings flow, a goal `dev-*`'s stability doesn't
///    undermine the same way it undermines this flag's actual, narrower
///    motivation (avoiding a repeat OS prompt from a randomized
///    per-build identity).
/// 3. If `AGENTMUX_CHANNEL` isn't set yet, stays global — conservative
///    default when channel context is unknown, not a guess.
///
/// Deliberately a separate flag from `AGENTMUX_ISOLATED_AUTH`/
/// `AGENTMUX_ISOLATED_SETTINGS`, not folded into either — MuxBus's cloud
/// session is a different kind of credential (one real cloud account, not
/// per-provider OAuth or local UI prefs) and callers should be able to
/// isolate one without the others.
pub fn isolated_muxbus_reconnect_enabled() -> bool {
    isolated_muxbus_reconnect_reason().is_isolated()
}

/// Which rule decided [`isolated_muxbus_reconnect_enabled`]'s result —
/// mirrors [`crate::data_paths::IsolatedAuthReason`]/[`crate::data_paths::IsolatedSettingsReason`] exactly, same
/// rationale (one resolution, two views).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsolatedMuxbusReconnectReason {
    /// `AGENTMUX_ISOLATED_MUXBUS=1`.
    ExplicitOptIn,
    /// `AGENTMUX_ISOLATED_MUXBUS` is set to anything other than exactly
    /// `"1"` — fail-safe by construction, same rule as
    /// `IsolatedAuthReason::ExplicitOptOut` and for the same reason: a
    /// typo'd opt-out attempt must not silently isolate a non-stable
    /// channel instead of falling back to the safe (global/reconnect)
    /// state.
    ExplicitOptOut,
    /// No override; `AGENTMUX_CHANNEL` is set, isn't `"stable"`, and
    /// doesn't start with `"dev-"` — a randomized local-package channel — on
    /// macOS, where the keychain would ask again for every build.
    ChannelDefaultIsolated,
    /// No override; a local-package channel, on an OS whose stored sign-in
    /// is read without a prompt (Windows, Linux): reconnects as usual.
    LocalChannelNoPrompt,
    /// No override; `AGENTMUX_CHANNEL` is `"stable"`, starts with
    /// `"dev-"`, or is unset entirely.
    ChannelDefaultGlobal,
}

impl IsolatedMuxbusReconnectReason {
    pub fn is_isolated(self) -> bool {
        matches!(self, Self::ExplicitOptIn | Self::ChannelDefaultIsolated)
    }

    /// Short, log-friendly label.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ExplicitOptIn => "explicit opt-in",
            Self::ExplicitOptOut => "explicit opt-out",
            Self::ChannelDefaultIsolated => "channel default — isolated",
            Self::LocalChannelNoPrompt => "local channel — reconnects (no keychain prompt on this OS)",
            Self::ChannelDefaultGlobal => "channel default — global",
        }
    }
}

pub fn isolated_muxbus_reconnect_reason() -> IsolatedMuxbusReconnectReason {
    muxbus_reconnect_reason_for(
        std::env::var("AGENTMUX_ISOLATED_MUXBUS").ok().as_deref(),
        std::env::var("AGENTMUX_CHANNEL").ok().as_deref(),
        keychain_prompts_per_build(),
    )
}

/// Whether reading the stored MuxBus sign-in can make the OS ask the user,
/// once per code signature: macOS's keychain does, and a local build is a
/// new signature every time. Windows' Credential Manager and Linux's secret
/// store don't.
pub fn keychain_prompts_per_build() -> bool {
    cfg!(target_os = "macos")
}

/// [`isolated_muxbus_reconnect_reason`] for an explicit override, a channel
/// and whether this OS's keychain prompts per build.
pub(crate) fn muxbus_reconnect_reason_for(
    isolated: Option<&str>,
    channel: Option<&str>,
    keychain_prompts: bool,
) -> IsolatedMuxbusReconnectReason {
    match isolated {
        Some("1") => IsolatedMuxbusReconnectReason::ExplicitOptIn,
        Some(_) => IsolatedMuxbusReconnectReason::ExplicitOptOut,
        None => match channel {
            Some(ch) if ch != "stable" && !ch.starts_with("dev-") => {
                if keychain_prompts {
                    IsolatedMuxbusReconnectReason::ChannelDefaultIsolated
                } else {
                    IsolatedMuxbusReconnectReason::LocalChannelNoPrompt
                }
            }
            _ => IsolatedMuxbusReconnectReason::ChannelDefaultGlobal,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_local_channel_reconnects_where_the_keychain_never_prompts() {
        // Windows and Linux: a local build, or a release a local build
        // upgraded to (it keeps the `local-…` channel), stays signed in to
        // MuxBus across restarts. macOS keeps skipping the eager read.
        for channel in ["local-main-b28b7a-651ced27", "local-agent4-release-v0.59.20-e732be-02929809"] {
            let no_prompt = muxbus_reconnect_reason_for(None, Some(channel), false);
            assert_eq!(no_prompt, IsolatedMuxbusReconnectReason::LocalChannelNoPrompt, "{channel}");
            assert!(!no_prompt.is_isolated(), "{channel}");
            let prompts = muxbus_reconnect_reason_for(None, Some(channel), true);
            assert_eq!(prompts, IsolatedMuxbusReconnectReason::ChannelDefaultIsolated, "{channel}");
            assert!(prompts.is_isolated(), "{channel}");
        }
        // The overrides and the other channels read the same on every OS.
        for prompts in [false, true] {
            assert!(muxbus_reconnect_reason_for(Some("1"), Some("stable"), prompts).is_isolated());
            assert!(!muxbus_reconnect_reason_for(Some("0"), Some("local-x-1"), prompts).is_isolated());
            for channel in [Some("stable"), Some("dev-main"), None] {
                assert_eq!(
                    muxbus_reconnect_reason_for(None, channel, prompts),
                    IsolatedMuxbusReconnectReason::ChannelDefaultGlobal,
                    "{channel:?}"
                );
            }
        }
    }
}
