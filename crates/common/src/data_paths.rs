// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Unified data-path resolution for AgentMux.
//!
//! Single source of truth for where state lives on disk. Replaces the
//! launcher / host / sidecar trio of independent path computations
//! (see docs/specs/archive/SPEC_DATA_DIR_UNIFICATION_2026-05-05.md §3) and the
//! per-version isolation pattern it set up (data was keyed on the
//! build version so My Agents reset on every patch bump). The current
//! model keys data on a *channel* — a stable identifier that spans
//! versions within the same compat band, so agents survive rebuilds.
//! See docs/specs/SPEC_DATA_CHANNELS_2026_05_24.md and discussion
//! #1026 for the channel design and rationale.
//!
//! Layout:
//!
//! ```text
//! ~/.agentmux/
//! ├── shared/                       (cookies, credentials, account-wide)
//! ├── channels/<channel>/           (installed + portable + custom)
//! │   ├── data/, config/, logs/, cef-cache/, agents/
//! │   └── runtime/                  (lock + IPC, single instance per channel)
//! └── dev/<branch>/                 (per-branch dev isolation)
//!     └── (same children as channels/<channel>/)
//! ```
//!
//! Channel resolution (via [`DataPaths::resolve`]):
//! - `AGENTMUX_CHANNEL=<name>` env override wins for `Installed` /
//!   `Portable` modes — lets the operator point a released binary at
//!   any channel for parallel-channel testing.
//! - `RuntimeMode::Installed` / `Portable` w/o override → a portable folder's
//!   recorded channel (`runtime/agentmux-channel`, after an upgrade), else the build-time
//!   default from `AGENTMUX_BUILD_CHANNEL_DEFAULT` (set by the
//!   packaging script; defaults to `"stable"` if unset, e.g. for
//!   `cargo run`).
//! - `RuntimeMode::Dev { branch }` → channel name is `dev-<branch>`
//!   for diagnostics; on-disk path stays at `~/.agentmux/dev/<branch>/`
//!   (NOT under `channels/`). Both the host (`agentmux-cef`) and
//!   launcher (`agentmux-launcher`) use [`DataPaths::resolve_path_only`]
//!   for dev builds to ignore `AGENTMUX_CHANNEL` — a dev session
//!   launched from inside a parent agentmux pane mustn't inherit
//!   the parent's channel and break per-branch isolation. Channel
//!   override is intentionally NOT supported in dev mode; if you
//!   want a different channel, use a portable build.

use crate::RuntimeMode;
use std::path::{Path, PathBuf};

mod dir_links;
mod isolation;

pub use dir_links::{ensure_history_link, ensure_owner_only_dir, OwnerOnlyOutcome};
pub use isolation::{
    isolated_auth_enabled, isolated_auth_reason, isolated_settings_enabled, isolated_settings_reason,
    IsolatedAuthReason, IsolatedSettingsReason,
};

/// Build-time default channel for `Installed` / `Portable` modes.
/// Set by the packaging script (`task package` exports
/// `AGENTMUX_BUILD_CHANNEL_DEFAULT=local-<branch>`; release CI exports
/// `stable`). Falls back to `"stable"` when the binary is built
/// without the env (e.g. plain `cargo build` / `cargo run` for tests).
const BUILD_CHANNEL_DEFAULT: &str =
    match option_env!("AGENTMUX_BUILD_CHANNEL_DEFAULT") {
        Some(s) => s,
        None => "stable",
    };

/// The channel this binary was built for (`stable` for a release, a
/// `local-…` channel of its own for a local package build). A constant of the
/// binary; the channel an instance runs on can differ (an upgrade's recorded
/// channel, `AGENTMUX_CHANNEL`). For `build_kind`, not for finding data.
pub fn build_channel_default() -> &'static str {
    BUILD_CHANNEL_DEFAULT
}

/// Channel names that would collide with sibling dirs at
/// `~/.agentmux/` or with reserved subdir names inside a channel.
/// Rejected by [`sanitize_channel_name`].
const RESERVED_CHANNEL_NAMES: &[&str] = &[
    "shared",
    "snapshots",
    "dev",
    "versions",
    "channels",
    "runtime",
];

/// All paths a launcher / host / srv needs. Computed once by the
/// launcher; downstream binaries read paths from env vars set by the
/// launcher rather than recomputing (avoids the legacy desync risk
/// where each binary made its own portable / dev-mode determination).
#[derive(Debug, Clone)]
pub struct DataPaths {
    /// `~/.agentmux/` itself — the resolved root. Account-wide config
    /// that predates the unified layout (e.g. the launcher's
    /// `config.toml`) lives directly here. Honors
    /// `AGENTMUX_HOME_OVERRIDE` for tests.
    pub home_dir: PathBuf,

    /// Top-level dir for this channel+mode. All per-channel paths
    /// below are children. Either `~/.agentmux/channels/<channel>/`
    /// (installed / portable / AGENTMUX_CHANNEL override) or
    /// `~/.agentmux/dev/<branch>/` (dev mode without env override).
    ///
    /// Note: the field name is `instance_dir` for backward compat with
    /// downstream call sites; semantically it's now the *channel* root,
    /// not the *version* root.
    pub instance_dir: PathBuf,

    /// Channel identifier this resolution used (e.g. `"stable"`,
    /// `"local-main"`, `"dev-main"`, or a user-specified custom
    /// channel from `AGENTMUX_CHANNEL`). Surfaced for diagnostics,
    /// logging, and the launcher splash; downstream binaries usually
    /// don't need it (paths are passed via env vars).
    pub channel: String,

    /// `instance_dir/data/` — srv DB (objects.db, sagas.db, …).
    pub data_dir: PathBuf,

    /// `instance_dir/config/` — settings.json, repos.json, etc.
    pub config_dir: PathBuf,

    /// `instance_dir/logs/` — host + srv + launcher logs (rotated).
    pub logs_dir: PathBuf,

    /// `instance_dir/cef-cache/` — Chromium runtime cache (regenerable).
    pub cef_cache_dir: PathBuf,

    /// `instance_dir/agents/` — agent workspace state.
    pub agents_dir: PathBuf,

    /// `instance_dir/runtime/` — single-instance lock + IPC (pid,
    /// lockfile, ipc-port, named-pipe). One set per version+mode.
    pub instance_runtime_dir: PathBuf,

    /// `~/.agentmux/shared/` — version-independent, account-wide
    /// state (cookies, OAuth tokens, API keys, dictionary downloads).
    pub shared_dir: PathBuf,

    /// Snapshot of the [`RuntimeMode`] this resolution used. Helpful
    /// for logging and feature gates.
    pub mode: RuntimeMode,
}

impl DataPaths {
    /// Resolve all paths for the given version + mode. Honors
    /// `AGENTMUX_HOME_OVERRIDE` for tests (replaces `~/.agentmux` root).
    ///
    /// Returns `Err` if the input contains values that cannot be
    /// represented as a safe single-segment subpath — e.g. `..` in the
    /// version string, or a Dev branch that sanitizes to empty. This
    /// is belt-and-braces safety: parse-time sanitization in
    /// [`crate::RuntimeMode`] should already have caught these, but a
    /// `RuntimeMode::Dev { branch }` constructed directly (e.g. by a
    /// test or future caller) is also rejected here.
    pub fn resolve(version: &str, mode: &RuntimeMode) -> Result<Self, String> {
        Self::resolve_internal(version, mode, /* honor_env_channel = */ true)
    }

    /// Like [`Self::resolve`], but ignores the `AGENTMUX_CHANNEL` env
    /// override and uses only the mode-based default channel. Mirror
    /// of [`RuntimeMode::current_path_only`] for path resolution.
    ///
    /// Used by dev-build self-detection paths in `agentmux-cef`'s
    /// `main.rs` and `sidecar.rs`. Those paths run when a dev host
    /// has been launched from inside a parent AgentMux instance (e.g.
    /// `task dev` invoked from inside an agent pane in a portable
    /// build), where the child would otherwise inherit the parent's
    /// `AGENTMUX_*` env — including `AGENTMUX_CHANNEL` — and write
    /// into the parent's channel instead of `dev/<branch>/`. That
    /// cross-contamination would also trip the channel's single-
    /// instance lock and route every "open" back to the parent
    /// window. Path-based mode detection is authoritative for dev
    /// builds; channel resolution here mirrors that discipline.
    /// Codex P1 follow-up on PR #1027.
    pub fn resolve_path_only(version: &str, mode: &RuntimeMode) -> Result<Self, String> {
        Self::resolve_internal(version, mode, /* honor_env_channel = */ false)
    }

    fn resolve_internal(
        version: &str,
        mode: &RuntimeMode,
        honor_env_channel: bool,
    ) -> Result<Self, String> {
        let root = resolve_root()?;
        // `version` is still validated for path safety even though it
        // no longer appears in the on-disk path — it flows into
        // logging, the migration framework (Increment B), and
        // `meta.json` records, so a traversal-laced value mustn't
        // round-trip into a future path build by accident.
        sanitize_path_segment(version)
            .ok_or_else(|| format!("invalid version string for path: {:?}", version))?;

        // Channel resolution: env override > mode default. Dev mode's
        // *channel name* and *path* diverge intentionally — name is
        // `dev-<branch>` for diagnostics; path stays at
        // `~/.agentmux/dev/<branch>/` so per-branch isolation works
        // unchanged from Phase 1.
        let (channel, instance_dir) =
            resolve_channel_and_dir(mode, &root, honor_env_channel)?;

        // For Installed/Portable builds, version-scope the mutable
        // runtime dirs so two concurrent release versions don't share
        // SQLite DBs or Chromium caches. Dev builds are already
        // branch-isolated via their path; no extra scoping needed.
        //
        // Layout after this change:
        //   channels/<ch>/versions/<v>/data/      ← objects.db, sagas.db …
        //   channels/<ch>/versions/<v>/logs/
        //   channels/<ch>/versions/<v>/cef-cache/
        //   channels/<ch>/versions/<v>/runtime/   ← ipc-port, lock
        //   channels/<ch>/config/                 ← settings (channel-wide)
        //   channels/<ch>/agents/                 ← agent defs (survive upgrades)
        //
        // See SPEC_VERSION_ISOLATION_2026_06_01.md §5 Phase 2.
        let version_dir = match mode {
            RuntimeMode::Installed | RuntimeMode::Portable => {
                instance_dir.join("versions").join(version)
            }
            RuntimeMode::Dev { .. } => instance_dir.clone(),
        };

        let data_dir = version_dir.join("data");
        let logs_dir = version_dir.join("logs");
        let cef_cache_dir = version_dir.join("cef-cache");
        let instance_runtime_dir = version_dir.join("runtime");
        // config and agents stay channel-wide so settings and agent
        // definitions persist across version upgrades.
        let config_dir = instance_dir.join("config");
        let agents_dir = instance_dir.join("agents");
        let shared_dir = root.join("shared");

        Ok(Self {
            home_dir: root,
            instance_dir,
            channel,
            data_dir,
            config_dir,
            logs_dir,
            cef_cache_dir,
            agents_dir,
            instance_runtime_dir,
            shared_dir,
            mode: mode.clone(),
        })
    }

    /// Create every directory that may be written to. Idempotent.
    /// Safe to call on every launch.
    ///
    /// Makes the AgentMux root owner-only first (issue #3682): it holds
    /// per-agent jekt HMAC keys and LAN private keys (in the DBs), provider
    /// login state (identity dirs) and logs, and on Unix every directory
    /// below it was otherwise created with the process umask — typically
    /// `0755`, readable by other local users wherever the home dir is
    /// traversable (macOS `/Users/<name>` is `0755` by default). Tightening
    /// the root blocks traversal into everything beneath it, including dirs
    /// srv creates later with plain `create_dir_all` (`shared/`, `archives/`).
    /// Best-effort: a root this process cannot chmod (owned by another user,
    /// read-only mount) is logged and the launch continues, as before.
    pub fn ensure_dirs(&self) -> Result<(), String> {
        match ensure_owner_only_dir(&self.home_dir) {
            Ok(OwnerOnlyOutcome::Tightened { previous_mode }) => tracing::warn!(
                root = %self.home_dir.display(),
                previous_mode = format!("{previous_mode:o}"),
                "AgentMux data root was readable by other local users; tightened it to owner-only (0700)"
            ),
            Ok(OwnerOnlyOutcome::Created) => tracing::info!(
                root = %self.home_dir.display(),
                "created AgentMux data root owner-only (0700)"
            ),
            Ok(OwnerOnlyOutcome::AlreadyPrivate | OwnerOnlyOutcome::NotApplicable) => {}
            Err(e) => tracing::warn!(
                root = %self.home_dir.display(),
                error = %e,
                "could not make the AgentMux data root owner-only; continuing"
            ),
        }
        for d in [
            &self.instance_dir,
            &self.data_dir,
            &self.config_dir,
            &self.logs_dir,
            &self.cef_cache_dir,
            &self.agents_dir,
            &self.instance_runtime_dir,
            &self.shared_dir,
        ] {
            std::fs::create_dir_all(d)
                .map_err(|e| format!("failed to create {}: {}", d.display(), e))?;
        }
        // The data dir's `db/` subdir is the canonical srv DB home;
        // mirrors legacy ensure_dirs() and lets srv unconditionally
        // open `data_dir/db/objects.db`.
        std::fs::create_dir_all(self.data_dir.join("db"))
            .map_err(|e| format!("failed to create db dir: {}", e))?;
        Ok(())
    }

    /// Env vars to pass to host + srv subprocesses. The launcher
    /// computes `DataPaths` once and exports these; downstream
    /// binaries read them via [`Self::from_env`] instead of
    /// recomputing.
    ///
    /// Returns `OsString` (not `String`) so paths with non-UTF-8 bytes
    /// — possible on Linux/macOS for users with exotic home dirs —
    /// round-trip losslessly. `Command::env(k, v)` accepts any
    /// `AsRef<OsStr>`, so the OsString flows through to children
    /// unchanged. The mode value is the only `String`-typed entry
    /// (it's a fixed ASCII vocabulary).
    pub fn to_env_vars(&self) -> Vec<(&'static str, std::ffi::OsString)> {
        use std::ffi::OsString;
        let mut vars: Vec<(&'static str, OsString)> = vec![
            ("AGENTMUX_INSTANCE_DIR", self.instance_dir.clone().into_os_string()),
            ("AGENTMUX_DATA_DIR", self.data_dir.clone().into_os_string()),
            ("AGENTMUX_CONFIG_DIR", self.config_dir.clone().into_os_string()),
            ("AGENTMUX_LOG_DIR", self.logs_dir.clone().into_os_string()),
            ("AGENTMUX_CEF_CACHE_DIR", self.cef_cache_dir.clone().into_os_string()),
            ("AGENTMUX_AGENTS_DIR", self.agents_dir.clone().into_os_string()),
            (
                "AGENTMUX_INSTANCE_RUNTIME_DIR",
                self.instance_runtime_dir.clone().into_os_string(),
            ),
            ("AGENTMUX_SHARED_DIR", self.shared_dir.clone().into_os_string()),
            ("AGENTMUX_RUNTIME_MODE", OsString::from(self.mode.to_env_string())),
            // Channel propagated so downstream binaries can log it +
            // surface in diagnostics. NOT used to recompute paths
            // (paths flow through the dir vars above).
            ("AGENTMUX_CHANNEL", OsString::from(self.channel.clone())),
        ];
        // Dev mode also exports AGENTMUX_CLONE_ID so child processes
        // (host, srv) can reconstruct the full `Dev { branch, clone_id }`
        // variant via [`RuntimeMode::from_env_with_clone`]. The
        // mode-string format (`dev:<branch>`) was kept backward-compatible
        // and doesn't carry clone_id itself — see runtime_mode.rs.
        if let RuntimeMode::Dev { clone_id: Some(id), .. } = &self.mode {
            vars.push(("AGENTMUX_CLONE_ID", OsString::from(id.clone())));
        }
        vars
    }

    /// Reconstruct from env vars set by the launcher. Returns
    /// `None` if any required var is missing — fail-fast vs.
    /// silently falling back to legacy paths the way the old
    /// sidecar.rs did.
    ///
    /// Uses `var_os` (not `var`) so non-UTF-8 path bytes survive.
    pub fn from_env() -> Option<Self> {
        let instance_dir = std::env::var_os("AGENTMUX_INSTANCE_DIR")?;
        let data_dir = std::env::var_os("AGENTMUX_DATA_DIR")?;
        let config_dir = std::env::var_os("AGENTMUX_CONFIG_DIR")?;
        let logs_dir = std::env::var_os("AGENTMUX_LOG_DIR")?;
        let cef_cache_dir = std::env::var_os("AGENTMUX_CEF_CACHE_DIR")?;
        let agents_dir = std::env::var_os("AGENTMUX_AGENTS_DIR")?;
        let instance_runtime_dir = std::env::var_os("AGENTMUX_INSTANCE_RUNTIME_DIR")?;
        let shared_dir = std::env::var_os("AGENTMUX_SHARED_DIR")?;
        // Pair AGENTMUX_RUNTIME_MODE with AGENTMUX_CLONE_ID so the Dev
        // variant carries its clone discriminator. Legacy single-var
        // form (no AGENTMUX_CLONE_ID set) leaves clone_id as None,
        // which falls back to the pre-PR two-level dev path layout.
        let mode = RuntimeMode::from_env_with_clone()?;
        // Channel is required from the launcher (same fail-fast
        // discipline as every other dir var). Missing AGENTMUX_CHANNEL
        // means the launcher didn't export it — that's a launcher /
        // srv version skew, surface it loudly rather than silently
        // defaulting and risking a wrong-channel write.
        let channel = std::env::var("AGENTMUX_CHANNEL").ok()?;

        // Re-resolve home_dir (the agentmux root) on the consumer
        // side rather than transmitting it via env — it's a function
        // of the AGENTMUX_HOME_OVERRIDE env (test only) and the OS
        // home dir, which are stable across the launcher → host hop.
        let home_dir = resolve_root().ok()?;

        Some(Self {
            home_dir,
            instance_dir: PathBuf::from(instance_dir),
            channel,
            data_dir: PathBuf::from(data_dir),
            config_dir: PathBuf::from(config_dir),
            logs_dir: PathBuf::from(logs_dir),
            cef_cache_dir: PathBuf::from(cef_cache_dir),
            agents_dir: PathBuf::from(agents_dir),
            instance_runtime_dir: PathBuf::from(instance_runtime_dir),
            shared_dir: PathBuf::from(shared_dir),
            mode,
        })
    }

    /// `~/.agentmux/shared/identities/` — root for per-bundle OAuth
    /// credential directories. Lives under `shared_dir` so it's
    /// account-wide and version-independent: upgrading agentmux does
    /// not move a user's bundle credentials. Per
    /// `docs/specs/archive/SPEC_OAUTH_IDENTITY_BUNDLES_2026_05_22.md` §4.1.
    ///
    /// When [`isolated_auth_enabled`] is set, this resolves to
    /// `instance_dir/identities/` instead — a channel-scoped credential
    /// tree, used only with `AGENTMUX_ISOLATED_AUTH=1`: auth is shared on
    /// every channel by default
    /// (`docs/specs/SPEC_SHARED_AUTH_ACROSS_CHANNELS_2026_10_03.md`).
    /// Originally scoped to destructive
    /// account testing (delete-account flows) that must never touch the
    /// real global identity store other channels/instances use.
    ///
    /// **Isolating this directory isolates a provider's conversation
    /// transcripts too**, since e.g. Claude Code's `projects/` lives
    /// inside the same per-bundle directory tree as its credentials —
    /// see [`identity_history_dir`] for the always-global path those
    /// transcripts are kept reachable at regardless of this flag, and
    /// `docs/specs/SPEC_AGENT_IDENTITY_HISTORY_PERSISTENCE_PROTOCOL_2026_08_16.md`
    /// §4.1 for why that split exists.
    /// See `docs/specs/SPEC_ISOLATED_AUTH_DEV_TESTING_2026_07_27.md`.
    pub fn identities_dir(&self) -> PathBuf {
        if isolated_auth_enabled() {
            self.instance_dir.join("identities")
        } else {
            self.shared_dir.join("identities")
        }
    }

    /// `~/.agentmux/shared/identities/<bundle_id>/<provider_dir>/<history_subdir>/`
    /// — ALWAYS this location, regardless of [`isolated_auth_enabled`].
    /// Conversation transcripts must survive a channel/build change even
    /// when the surrounding credential directory ([`identities_dir`]) is
    /// isolated per-channel for account testing — history and credentials
    /// are different data-persistence categories (CONVERSATION HISTORY
    /// vs CREDENTIAL in
    /// `docs/specs/SPEC_AGENT_IDENTITY_HISTORY_PERSISTENCE_PROTOCOL_2026_08_16.md`
    /// P1) and must not share one isolation switch just because they
    /// happen to live under the same directory as far as the provider
    /// CLI is concerned. Callers that isolate credentials are expected
    /// to redirect the isolated `<bundle_id>/<provider_dir>/<history_subdir>`
    /// subpath to this location via [`ensure_history_link`] rather than
    /// letting the provider CLI write real session data there directly.
    ///
    /// `history_subdir` is caller-supplied rather than hardcoded — it is
    /// NOT the same directory name for every provider (reagentx P1 on PR
    /// #2605: this used to hardcode `"projects"`, which is Claude-
    /// specific; Codex uses `"sessions"`, Gemini uses `"history"`, per
    /// `docs/specs/SPEC_UNIFIED_AGENT_HISTORY_STORE_2026-06-10.md` §2.1).
    /// Callers should read this from
    /// `ProviderConfig::history_native_subdir`, the single source of
    /// truth for which providers have a linkable directory at all.
    ///
    /// `bundle_id`/`provider_dir` are not path-validated here — callers
    /// must use [`sanitize_path_segment`]-checked values, same
    /// requirement as [`identity_dir`].
    pub fn identity_history_dir(&self, bundle_id: &str, provider_dir: &str, history_subdir: &str) -> PathBuf {
        self.shared_dir
            .join("identities")
            .join(bundle_id)
            .join(provider_dir)
            .join(history_subdir)
    }

    /// `~/.agentmux/shared/identities/<bundle_id>/` — a specific
    /// bundle's credential root, when `bundle_id` is a safe path
    /// segment. Returns `None` for empty / `.` / `..` / any segment
    /// containing `/`, `\`, drive-letter colons, or Windows-reserved
    /// characters (same rules as the version/branch sanitizer in
    /// `resolve`).
    ///
    /// Defensive return type: `bundle_id` flows from `auth.start`
    /// request bodies (PR C) into `create_dir_all`, so an
    /// unvalidated `PathBuf::join` would let a crafted id escape the
    /// identities root and write outside the bundle area. codex P1
    /// follow-up on #981.
    ///
    /// Per-provider subdirectories (e.g. `claude/`, `codex/`) hang
    /// off this when the bundle gains an OAuth binding (PR C). The
    /// directory is created lazily by the bundle / OAuth flow that
    /// needs it — `ensure_dirs()` does not pre-create it.
    pub fn identity_dir(&self, bundle_id: &str) -> Option<PathBuf> {
        sanitize_path_segment(bundle_id).map(|safe| self.identities_dir().join(safe))
    }

    /// `~/.agentmux/shared/providers/<auth_dir_name>/` — the DEFAULT provider
    /// config + auth dir. Lives under `shared_dir`, so it is account-wide,
    /// version-independent, AND channel-independent: every instance / channel /
    /// version logs in ONCE and shares it. This is the structural fix for the
    /// per-channel "validate-spin" regression — there is no empty-per-instance
    /// auth dir to spin on. The per-identity override (`identity_dir`) still
    /// takes precedence for explicit multi-account bundles. `auth_dir_name`
    /// comes from the static provider registry (e.g. "claude"), never user
    /// input. Retro:
    /// `docs/retro/retro-provider-auth-isolation-regression-2026-06-05.md`.
    pub fn provider_auth_dir(&self, auth_dir_name: &str) -> PathBuf {
        self.shared_dir.join("providers").join(auth_dir_name)
    }

    /// `instance_dir/wan-identity/` — the channel-wide WAN identity store
    /// (`wan.db`: the instance key, agent WAN keys, and WAN verification
    /// state). Beside `config/` and `agents/` rather than under the
    /// per-version `data_dir`, so an upgrade keeps the same instance and the
    /// same agent keys. Deliberately not `identity/`, which would sit next to
    /// the unrelated `identities/`. For dev builds this is the branch (or
    /// clone) dir, so each dev checkout is its own instance.
    /// `SPEC_WAN_JEKT_VERIFICATION_2026_09_24.md` §2.2.
    pub fn wan_identity_dir(&self) -> PathBuf {
        self.instance_dir.join("wan-identity")
    }
}

// The MuxBus reconnect flag lives in its own module; re-exported so callers
// keep reaching it here.
pub use crate::muxbus_reconnect::{
    isolated_muxbus_reconnect_enabled, isolated_muxbus_reconnect_reason, keychain_prompts_per_build,
    IsolatedMuxbusReconnectReason,
};

/// The AgentMux root (`~/.agentmux`) — the single resolver for it.
///
/// Public because srv had its own copy (`backend/base.rs::get_mux_data_dir`)
/// reading a *different* env var and failing differently: where this returns
/// `Err`, that one fell back to `PathBuf::from("/")`, so a host with no
/// resolvable home directory wrote to `/.agentmux` instead of refusing.
/// A10 of docs/analysis/TRACKING_ARCHITECTURE_REFACTOR_A1_A15_2026_06_18.md.
///
/// Two env vars are honoured, in this order, because both already exist in
/// the wild and silently dropping either would move where a live install
/// finds its data:
///
/// - `AGENTMUX_HOME_OVERRIDE` — this module's own, used by tests. It wins over
///   `AGENTMUX_DATA_HOME`, except for a data home inside it (`data_home.rs`).
/// - `AGENTMUX_DATA_HOME` — srv's own, read at startup and relied on by the
///   MSIX packaging path (`blockcontroller/shell/lifecycle.rs`). It is
///   deliberately **stripped** from pane environments, not exported into them:
///   it is one of the identity vars invariant I7 removes so a pane cannot
///   inherit and resolve another instance's data dir
///   (`backend/pane_env.rs`, test `the_identity_vars_that_caused_the_breach_are_stripped`).
///
/// NOT to be confused with `AGENTMUX_DATA_DIR`, which the launcher exports
/// and which names the per-channel *data* directory
/// (`<root>/channels/<channel>/data`), not the root. The similar names are
/// exactly why srv ended up with a second resolver.
pub fn agentmux_root() -> Result<PathBuf, String> {
    // An isolated instance keeps the versioned layout (data_home.rs, task #3569).
    if let Some(data) = crate::data_home::inside_override() {
        return Ok(data);
    }
    // `var_os`: a root that isn't valid UTF-8 must not read as unset.
    for var in ["AGENTMUX_HOME_OVERRIDE", "AGENTMUX_DATA_HOME"] {
        if let Some(s) = std::env::var_os(var) {
            if !s.is_empty() {
                return Ok(PathBuf::from(s));
            }
        }
    }
    let home = dirs::home_dir().ok_or_else(|| "dirs::home_dir() returned None".to_string())?;
    Ok(home.join(".agentmux"))
}

fn resolve_root() -> Result<PathBuf, String> {
    agentmux_root()
}

/// Sanitize a string for use as a single filesystem path segment.
/// Rejects empty, `.`, `..`, segments containing path separators, and
/// any character that has filesystem-special meaning on Windows (which
/// is the most restrictive of the platforms we target). Used as belt-
/// and-braces protection in `DataPaths::resolve` to prevent traversal
/// even when callers pass a directly-constructed `RuntimeMode::Dev` or
/// odd version string.
///
/// Why `:` is rejected: on Windows `C:temp` is a drive-relative path,
/// not a literal filename, so `PathBuf::join("versions").join("C:temp")`
/// would resolve OUTSIDE the intended `~/.agentmux/versions/` subtree.
fn sanitize_path_segment(s: &str) -> Option<String> {
    // Reject whitespace padding rather than silently normalizing it
    // away — otherwise distinct caller-supplied ids like "foo" and
    // " foo " would alias to the same directory. bundle_id is a real
    // caller-supplied identifier (passed through RPC payloads), so
    // this matters for credential isolation. codex P2 follow-up on
    // #981. Internally-generated version strings + branch names
    // shouldn't carry padding anyway, so this is no-op for them.
    if s != s.trim() {
        return None;
    }
    if s.is_empty() || s == "." || s == ".." {
        return None;
    }
    // Filesystem separators + Windows-reserved characters + NUL.
    if s
        .chars()
        .any(|c| matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0'))
    {
        return None;
    }
    Some(s.to_string())
}

/// Sanitize a string for use as a channel name. Same path-segment
/// safety rules as [`sanitize_path_segment`] plus:
/// - Length capped at 64 chars (channel names show up in logs + the
///   launcher splash + may eventually be displayed in a picker, so
///   the cap is for UI sanity, not security).
/// - Rejects names in [`RESERVED_CHANNEL_NAMES`] that would collide
///   with sibling dirs at `~/.agentmux/` or reserved subdir names
///   inside a channel.
/// - The synonym `"default"` maps to `"stable"` (per
///   `SPEC_DATA_CHANNELS_2026_05_24.md` §7.5).
fn sanitize_channel_name(s: &str) -> Option<String> {
    let base = sanitize_path_segment(s)?;
    if base.len() > 64 {
        return None;
    }
    if RESERVED_CHANNEL_NAMES.contains(&base.as_str()) {
        return None;
    }
    if base == "default" {
        return Some("stable".to_string());
    }
    Some(base)
}

/// Resolve the channel name and on-disk channel dir for a given mode.
/// Pure function over (env, mode, root). When `honor_env_channel` is
/// `true`, `AGENTMUX_CHANNEL` overrides the mode default; when `false`,
/// the env is ignored and resolution depends only on `mode` +
/// build-time defaults. The `false` path is used by dev-build self-
/// detection (see [`DataPaths::resolve_path_only`]).
///
/// Resolution order (mirrors `SPEC_DATA_CHANNELS_2026_05_24.md` §2.2):
///   1. (only if `honor_env_channel`) `AGENTMUX_CHANNEL` env override —
///      any mode → path is `<root>/channels/<channel>/`. Lets the
///      operator point any binary at any channel for parallel-channel
///      testing.
///   2. No override (or env-channel disallowed), mode = Dev { branch }
///      → channel name is `dev-<branch>`, path stays at
///      `<root>/dev/<branch>/` (unchanged from Phase 1).
///   3. Same conditions, mode = Installed | Portable → channel name is
///      [`BUILD_CHANNEL_DEFAULT`] (set at build time by the packaging
///      script), path is `<root>/channels/<channel>/`.
fn resolve_channel_and_dir(
    mode: &RuntimeMode,
    root: &Path,
    honor_env_channel: bool,
) -> Result<(String, PathBuf), String> {
    // (1) Explicit env override — only when caller opted in.
    if honor_env_channel {
        if let Ok(raw) = std::env::var("AGENTMUX_CHANNEL") {
            if !raw.is_empty() {
                let channel = sanitize_channel_name(&raw).ok_or_else(|| {
                    format!("invalid AGENTMUX_CHANNEL value: {:?}", raw)
                })?;
                let dir = root.join("channels").join(&channel);
                return Ok((channel, dir));
            }
        }
    }

    // (2) Dev mode default: dev-<branch>[-<clone_id>], path under
    // dev/<branch>/[<clone_id>/]. The clone_id nests one level deeper
    // so two clones of the same branch don't collide on data dir,
    // lockfile, or named-pipe IPC. When clone_id is None (legacy
    // env-string round-trip or direct test construction) the layout
    // falls back to the original two-level form for back-compat.
    // See SPEC_DATA_CHANNELS_2026_05_24.md §2.4 and
    // docs/analysis/ANALYSIS_MULTI_CLONE_TASK_DEV_ISOLATION_2026-05-26.md.
    if let RuntimeMode::Dev { branch, clone_id } = mode {
        let safe_branch = sanitize_path_segment(branch).ok_or_else(|| {
            format!("invalid dev branch for path: {:?}", branch)
        })?;
        let safe_clone = clone_id
            .as_deref()
            .and_then(sanitize_path_segment)
            .filter(|s| !s.is_empty());
        let (channel, dir) = match safe_clone {
            Some(c) => (
                format!("dev-{}-{}", safe_branch, c),
                root.join("dev").join(safe_branch).join(c),
            ),
            None => (
                format!("dev-{}", safe_branch),
                root.join("dev").join(safe_branch),
            ),
        };
        return Ok((channel, dir));
    }

    // (3) Installed / Portable default: the channel an upgrade recorded for this
    // install (`runtime_mode::recorded_channel`), else the build-time one.
    let recorded = crate::runtime_mode::recorded_channel();
    let channel = recorded.as_deref().and_then(sanitize_channel_name).or_else(|| sanitize_channel_name(BUILD_CHANNEL_DEFAULT));
    let channel = channel.ok_or_else(|| format!("compile-time AGENTMUX_BUILD_CHANNEL_DEFAULT is invalid: {BUILD_CHANNEL_DEFAULT:?}"))?;
    let dir = root.join("channels").join(&channel);
    Ok((channel, dir))
}

#[cfg(test)]
mod tests;
