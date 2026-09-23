// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Unit tests for the `bundle.*` App API handlers, one file per former
//! inline `mod *_tests` block. Each file opens with `use super::super::*;`
//! — the `bundle` module itself — so it sees exactly what the inline
//! module used to see via `use super::*;`, private items included.

mod check_provider_model_immutable;
mod import_preview_commit;
mod export_import_for_agent;
