// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Identity injection at agent CLI spawn time.
//!
//! When an agent instance is launched, the launch modal records an
//! `identity_id` on the `db_agent_instances` row (v7 schema). Right
//! before the CLI subprocess starts, this module:
//!
//! 1. Looks up the active instance for the spawning block.
//! 2. Reads its `identity_id` (a legacy filter value; credentials come
//!    from the links in step 3, not from this).
//! 3. Reads the direct `db_agent_identity_links` rows for the instance's
//!    definition.
//! 4. For each link: looks up the Account row, resolves its
//!    `SecretRef` to a plaintext value, looks up the provider →
//!    env-var matrix, and merges those env vars into the spawn
//!    `env_vars` HashMap.
//!
//! Failure modes differ by provider class. An api-key-class failure
//! (missing account, unresolvable secret, unknown provider) is logged and
//! skipped. An oauth-class provider with no resolvable bound account
//! blocks the spawn (`resolver::inject`'s `gate_oauth_failure`); there is
//! no fallback to the user's global CLI login.
//!
//! Closes Phase 2 of issue #678 (the per-instance injection layer).
//! Phase 1 (Account registry + UI) and the v7 schema reshape (Bundle
//! entity) were prerequisites; Phase 3 (encrypted vault, OAuth flows)
//! is deferred.

pub mod account_email;
pub mod auth_diag;
pub mod auth_patterns;
pub mod auth_session;
pub mod browser_credential_store;
pub mod cleanup;
pub mod key_validator;
pub mod oauth_client;
pub mod resolver;
pub mod secret_store;

// Legacy convenience re-export — newer call sites use
// `resolver::inject_identity_env_with_broker` directly so the OAuth
// expiry probe (PR D, spec §4.4) can publish on status change. The
// broker-less wrapper is kept for tests and is intentionally allowed
// to be unused in production.
#[allow(unused_imports)]
pub use resolver::inject_identity_env;
