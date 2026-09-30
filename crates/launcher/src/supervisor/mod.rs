// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Platform host-supervision loops. The Windows and Unix supervisors are
//! fully `#[cfg]`-gated in separate files; only the flow spine
//! (`launcher_main`) lives at the crate root.

/// Phase 1 host supervision (spec
/// `docs/specs/SPEC_SERVICE_SUPERVISION_AND_RECOVERY_2026_05_20.md`): on an
/// abnormal host exit the launcher relaunches the host, but at most
/// `HOST_RESTART_BUDGET` times within `HOST_RESTART_WINDOW` — a crash budget
/// so a deterministic crash cannot spin forever (spec §10-A). Shared by
/// the Windows (`run_windows`) and Unix (`run_unix`) supervisors.
pub(crate) const HOST_RESTART_BUDGET: usize = 3;
pub(crate) const HOST_RESTART_WINDOW: std::time::Duration = std::time::Duration::from_secs(60);

/// SPEC_SRV_SUPERVISION_RECYCLE_2026_07_11 (#942 Phase 2) — srv gets its
/// own crash budget: an unexpected srv exit respawns srv and recycles the
/// host through the existing supervised-restart path (crash-reproject
/// restores the session), at most this many times per window. A wider
/// window than the host's: each srv recycle also costs a host restart, so
/// a crash-looping srv burns both budgets and gives up loudly either way.
#[cfg(target_os = "windows")]
pub(crate) const SRV_RESTART_BUDGET: usize = 3;
#[cfg(target_os = "windows")]
pub(crate) const SRV_RESTART_WINDOW: std::time::Duration = std::time::Duration::from_secs(120);

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub(crate) use windows::run_windows;

#[cfg(not(target_os = "windows"))]
mod unix;
#[cfg(not(target_os = "windows"))]
pub(crate) use unix::run_unix;
