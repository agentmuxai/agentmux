// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Identity injection at agent CLI spawn time.
//!
//! When an agent instance is launched, the launch modal records an
//! `identity_id` on the `db_agent_instances` row (v7 schema). Right
//! before the CLI subprocess starts, this module:
//!
//! 1. Looks up the active instance for the spawning block.
//! 2. Reads its `identity_id`. Empty / "blank" / not-found → noop
//!    (the agent inherits ambient credentials).
//! 3. Reads the bindings for that Identity bundle.
//! 4. For each binding: looks up the Account row, resolves its
//!    `SecretRef` to a plaintext value, looks up the provider →
//!    env-var matrix, and merges those env vars into the spawn
//!    `env_vars` HashMap.
//!
//! Failure mode is **warn-don't-block**: missing accounts, env-var
//! resolution errors, unknown providers — all logged and skipped.
//! The agent CLI launches with whatever ambient credentials remain.
//! This is intentional: identity injection is a convenience, not a
//! security gate. The caller flags hard-required-creds workflows
//! separately.
//!
//! Closes Phase 2 of issue #678 (the per-instance injection layer).
//! Phase 1 (Account registry + UI) and the v7 schema reshape (Bundle
//! entity) were prerequisites; Phase 3 (encrypted vault, OAuth flows)
//! is deferred.

pub mod auth_patterns;
pub mod auth_session;
pub mod resolver;

pub use resolver::inject_identity_env;
