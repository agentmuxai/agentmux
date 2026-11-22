// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A backed entry's value (docs/specs/SPEC_VAULT_SHARED_ACROSS_MACHINES_LAN_AND_CLOUD_2026_11_01.md §11.1): the Vault
//! keeps the entry's label, kind and grants, and the key itself stays in AWS Secrets Manager. Each use reads it from
//! there with the host's AWS login, so there is no copy in the Vault to go stale, to sync or to leak, and a change made in
//! Secrets Manager is what the next use gets.
//!
//! The value lives in a `Zeroizing` buffer from the CLI's output to the caller, which hands it to the host or a command
//! and never to an agent. A read that fails says why in fixed sentences (`aws_import`); nothing of the value is in one.

use zeroize::Zeroizing;

use super::aws_import::{self, Source};
use super::linked;
use super::source_link::Link;
use crate::backend::storage::vault::VaultRow;
use crate::identity::aws_broker::mint::{HostRunner, Runner};

/// Where a backed entry's value is: its AWS secret, and the field of it when the secret is JSON.
pub fn source_of(row: &VaultRow) -> Result<Source, String> {
    Link::of_row(row).and_then(|link| link.aws()).ok_or_else(|| format!("{} is backed by AWS Secrets Manager, but its place isn't an AWS secret", row.label))
}

/// The value of the backed entry `row` for one use, read with `runner`. Not async: a use runs on a blocking thread
/// (`aws_import::read_value_blocking`).
pub fn value_with<R: Runner>(runner: &R, row: &VaultRow) -> Result<Zeroizing<String>, String> {
    debug_assert!(linked::is_backed_row(row));
    aws_import::read_value_blocking(runner, &source_of(row)?).map_err(|why| format!("{} (read from AWS Secrets Manager): {why}", row.label))
}

/// [`value_with`] under the host's own AWS login.
pub fn value_of(row: &VaultRow) -> Result<Zeroizing<String>, String> {
    value_with(&HostRunner, row)
}

/// Why a backed entry has no earlier value to name (`id@previous`, `id@pending`): the Vault keeps none, Secrets Manager
/// does.
pub fn no_versions(row: &VaultRow, which: &str) -> String {
    format!("{} is backed by AWS Secrets Manager, so the Vault keeps no earlier value of it: @{which} is for an entry that holds its own value", row.label)
}

#[cfg(test)]
#[path = "backed_tests.rs"]
pub(crate) mod tests;
