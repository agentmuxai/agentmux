// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! OS notification Router — `docs/specs/SPEC_OS_NOTIFICATIONS_SYSTEM_2026_09_24.md`.
//!
//! - `policy`: pure decision state machine (debounce, dedupe, focus gate, retract).
//! - `router`: side effects around it (settings, names, redaction, broker publish).
//!
//! Presenters (the launcher's OS toasts, later the tray and in-app surfaces)
//! subscribe to the `notification*` broker events; sources report through the
//! `notify.*` RPCs in `server/notify_handlers.rs`.

pub mod policy;
pub mod router;
pub mod sources;
