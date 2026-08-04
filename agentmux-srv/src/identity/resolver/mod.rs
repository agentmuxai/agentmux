// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Identity → env-var resolver.
//!
//! Per-provider matrix of which env vars carry which credential. The
//! GitHub PAT becomes both `GITHUB_TOKEN` and `GH_TOKEN` because both
//! the official `gh` CLI and direct API consumers (curl, oct.js) read
//! one or the other; emitting both is the lowest-friction way to make
//! every common workflow Just Work.
//!
//! **Before touching `gate_oauth_failure` / `inject_identity_env_with_broker`:**
//! this module is where `SPEC_PROVIDER_ISOLATION_2026_06_20.md`'s INV-A
//! ("never the user's global `~/.<P>` dir") is enforced — or, once already,
//! silently stopped being enforced. Read
//! `docs/retro/retro-auth-isolation-invariant-silently-orphaned-2026-07-14.md`
//! first. Short version: an unbound oauth-class provider used to
//! auto-route to an AgentMux-owned isolated dir (no user action, no global
//! exposure); a 2026-07-08 refactor orphaned that path without meaning to,
//! and it was never restored — today's gate only chooses between "block"
//! and "true ambient" (`use_ambient_login=true`, zero isolation), not the
//! isolated-auto-provision option that used to exist implicitly.
//!
//! ## Module layout
//!
//! Split from a single ~2193-line `resolver.rs` file into a directory
//! (pure relocation, no behavior change — mirrors the
//! `backend/subagent_watcher/` and `backend/blockcontroller/subprocess/`
//! splits) so each self-contained piece lives in its own file:
//!   - `oauth_probe`: `oauth_status` constants, `OAuthProbeStatus`, and
//!     `probe_oauth_status` — on-disk OAuth token-file probing.
//!   - `errors`: `SpawnGateError`, `ResolverError`.
//!   - `provider`: `ProviderClass`, `provider_class`
//!     — the provider classification table.
//!   - `secret`: `resolve_secret`.
//!   - `inject` — this module's security-critical core (see the warning
//!     above): `inject_identity_env`/`inject_identity_env_async`,
//!     `IdentityBinding`, `resolve_bindings_for_instance`, and
//!     `inject_identity_env_with_broker` — the layer-3 credential-
//!     injection spawn gate. The INV-A warning is repeated there,
//!     attached directly to `inject_identity_env_with_broker`, and every
//!     test exercising that function moved WITH it into `inject.rs` as
//!     one atomic unit.

mod errors;
mod inject;
mod oauth_probe;
mod provider;
mod secret;

pub use errors::{ResolverError, SpawnGateError};
pub use inject::{inject_identity_env, inject_identity_env_async, inject_identity_env_with_broker};
pub use oauth_probe::{oauth_status, probe_oauth_status, OAuthProbeStatus};
pub use provider::{provider_class, ProviderClass};
pub use secret::resolve_secret;
