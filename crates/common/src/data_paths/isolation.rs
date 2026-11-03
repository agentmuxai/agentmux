// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The isolated-auth and isolated-settings switches: whether a channel keeps its own
//! accounts, credential folders and settings, or shares them.

/// Isolated per-channel auth (identity accounts + OAuth credential dirs).
/// Read directly at every call site rather than cached on `DataPaths` so
/// `identities_dir()` behaves consistently regardless of whether the
/// caller built its `DataPaths` via `resolve()` (launcher) or
/// `from_env()` (downstream host/srv).
///
/// Resolution order:
/// 1. `AGENTMUX_ISOLATED_AUTH=1` — isolate this channel's auth. Meant for
///    testing the login and relogin paths from an empty store.
/// 2. Any other value of `AGENTMUX_ISOLATED_AUTH` — shared (explicit opt-out).
/// 3. Unset — **shared, on every channel**. A new build, a rebuilt branch,
///    `task dev` and a release all read the same accounts, provider login
///    folders, bundles, drones, native memory, Global Memory and MuxBus
///    sign-in, so a new build needs no login
///    (`SPEC_SHARED_AUTH_ACROSS_CHANNELS_2026_10_03.md`). Until 2026-10-03 every
///    channel except `stable` defaulted to isolated
///    (`SPEC_ISOLATED_AUTH_DEFAULT_BY_CHANNEL_2026_08_06.md`).
///
/// See `docs/specs/SPEC_ISOLATED_AUTH_DEV_TESTING_2026_07_27.md` (the
/// underlying mechanism — channel-scoped store + credential dirs — this
/// flag drives, still authoritative).
pub fn isolated_auth_enabled() -> bool {
    isolated_auth_reason().is_isolated()
}

/// Which rule decided [`isolated_auth_enabled`]'s result — for boot-time
/// diagnostics (see `bootstrap/stores.rs`'s "shared store: attached" log line)
/// so a developer staring at an empty Connectors → Accounts can tell at a glance whether
/// that is an explicit choice, rather than re-deriving it from the env by
/// hand. Callers that only need the boolean should use
/// [`isolated_auth_enabled`] directly — this exists purely so the two never
/// drift (one resolution, two views).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsolatedAuthReason {
    /// `AGENTMUX_ISOLATED_AUTH=1`.
    ExplicitOptIn,
    /// `AGENTMUX_ISOLATED_AUTH` is set to anything other than exactly `"1"`
    /// (`"0"`, `""`, a typo like `"false"`, anything): shared. Fail-safe by
    /// construction: a malformed opt-in can never silently isolate a channel
    /// (#2431), and since 2026-10-03 the unset default is shared as well.
    ExplicitOptOut,
    /// No override: shared, on every channel.
    DefaultShared,
}

impl IsolatedAuthReason {
    pub fn is_isolated(self) -> bool {
        matches!(self, Self::ExplicitOptIn)
    }

    /// Short, log-friendly label.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ExplicitOptIn => "explicit opt-in",
            Self::ExplicitOptOut => "explicit opt-out",
            Self::DefaultShared => "default — shared",
        }
    }
}

pub fn isolated_auth_reason() -> IsolatedAuthReason {
    match std::env::var("AGENTMUX_ISOLATED_AUTH") {
        // Exactly "1" isolates; any other value the var is explicitly set to
        // ("0", "", a typo) is an explicit opt-out.
        Ok(v) if v == "1" => IsolatedAuthReason::ExplicitOptIn,
        Ok(_) => IsolatedAuthReason::ExplicitOptOut,
        Err(_) => IsolatedAuthReason::DefaultShared,
    }
}

/// Isolated per-channel `settings.json`. Same shape and same reasoning as
/// [`isolated_auth_enabled`] — see
/// `docs/specs/SPEC_SETTINGS_ISOLATED_BY_CHANNEL_2026_08_19.md`.
///
/// Resolution order:
/// 1. `AGENTMUX_ISOLATED_SETTINGS=1` / `=0` — explicit override, always wins.
/// 2. Otherwise, defaults to isolated for every channel except `"stable"`.
///    `stable` is the real release channel — the daily-driver install(s)
///    this machine's actual work depends on — and keeps the old
///    always-global behavior, so nobody's real window theme/pinned
///    widgets/voice API key gets silently blanked by a channel-name
///    coincidence. Every `task dev` branch and every `task package` local
///    build now starts with a genuinely default `settings.json` — the
///    motivating case was `network:lan_discovery` silently carrying
///    `true` into a brand-new build from a decision made in an unrelated
///    channel weeks earlier.
/// 3. If `AGENTMUX_CHANNEL` isn't set yet, stays global — conservative
///    default when channel context is unknown, not a guess.
///
/// Deliberately a SEPARATE flag from `AGENTMUX_ISOLATED_AUTH`, not a
/// shared `AGENTMUX_ISOLATED` umbrella — see the spec's Open Questions
/// §1 for why.
pub fn isolated_settings_enabled() -> bool {
    isolated_settings_reason().is_isolated()
}

/// Which rule decided [`isolated_settings_enabled`]'s result — for
/// boot-time diagnostics, mirroring [`IsolatedAuthReason`] exactly (see
/// that type's doc comment for the rationale: one resolution, two views,
/// so the boolean and the log line can never drift apart).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsolatedSettingsReason {
    /// `AGENTMUX_ISOLATED_SETTINGS=1`.
    ExplicitOptIn,
    /// `AGENTMUX_ISOLATED_SETTINGS` is set to anything other than exactly
    /// `"1"` — fail-safe by construction, same rule as
    /// `IsolatedAuthReason::ExplicitOptOut` and for the same reason: a
    /// typo'd opt-out attempt must not silently isolate a non-stable
    /// channel instead of falling back to the safe (global) state.
    ExplicitOptOut,
    /// No override; `AGENTMUX_CHANNEL` is set and isn't `"stable"`.
    ChannelDefaultIsolated,
    /// No override; `AGENTMUX_CHANNEL` is `"stable"` or unset entirely.
    ChannelDefaultGlobal,
}

impl IsolatedSettingsReason {
    pub fn is_isolated(self) -> bool {
        matches!(self, Self::ExplicitOptIn | Self::ChannelDefaultIsolated)
    }

    /// Short, log-friendly label.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ExplicitOptIn => "explicit opt-in",
            Self::ExplicitOptOut => "explicit opt-out",
            Self::ChannelDefaultIsolated => "channel default — isolated",
            Self::ChannelDefaultGlobal => "channel default — global",
        }
    }
}

/// Reads both inputs from the process env; the rule itself is
/// [`crate::settings_path::isolated_settings_reason_for`].
pub fn isolated_settings_reason() -> IsolatedSettingsReason {
    let opt = std::env::var(crate::settings_path::ISOLATED_SETTINGS_ENV).ok();
    crate::settings_path::isolated_settings_reason_for(opt.as_deref(), std::env::var("AGENTMUX_CHANNEL").ok().as_deref())
}
