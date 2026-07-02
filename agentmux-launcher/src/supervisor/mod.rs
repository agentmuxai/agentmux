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

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub(crate) use windows::run_windows;

#[cfg(not(target_os = "windows"))]
mod unix;
#[cfg(not(target_os = "windows"))]
pub(crate) use unix::run_unix;
