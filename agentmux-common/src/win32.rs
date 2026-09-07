// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Win32 process-creation flags shared by every crate that spawns a child
//! on Windows.
//!
//! Before this module existed, `CREATE_NO_WINDOW` was re-declared privately
//! in 23 files across five crates — including twice inside this crate, in
//! function bodies that exported it to nobody
//! (`docs/reports/REPORT_DRY_AND_MODULARITY_AUDIT_2026_09_06.md` §2.2).
//! The value is a Win32 ABI constant and never changes; declare it once.
//!
//! These are plain `u32`s rather than a helper that takes a `Command`,
//! because callers use both `std::process::Command` (needs the
//! `CommandExt` trait in scope) and `tokio::process::Command` (inherent
//! method) — a single generic helper would need to abstract over both for
//! no gain. The `#[cfg(windows)]` block at each call site stays; only the
//! private `const` inside it goes away.
//!
//! Compiled on every platform so a `use agentmux_common::win32::*` never
//! needs its own `cfg` guard; the values are only *meaningful* on Windows.

/// `CREATE_NO_WINDOW` — do not allocate a console for a console-subsystem
/// child. GUI-subsystem parents (the host, the launcher) and the
/// windowless sidecar otherwise get a console window flashed open for every
/// `node`, `cmd.exe`, `git`, `taskkill`, etc. they spawn.
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// `CREATE_SUSPENDED` — create the child's primary thread suspended, so the
/// parent can assign it to a Job Object before it runs a single instruction.
pub const CREATE_SUSPENDED: u32 = 0x0000_0004;

#[cfg(test)]
mod tests {
    use super::*;

    /// The values are Win32 ABI constants (`processthreadsapi.h`); pinning
    /// them stops a typo during a future edit from silently changing which
    /// flag every spawn in the workspace passes.
    #[test]
    fn flags_match_the_win32_abi() {
        assert_eq!(CREATE_NO_WINDOW, 0x08000000);
        assert_eq!(CREATE_SUSPENDED, 0x00000004);
    }
}
