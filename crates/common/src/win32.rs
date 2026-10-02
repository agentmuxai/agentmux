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
//! The flags are plain `u32`s so a spawn that needs something other than
//! `CREATE_NO_WINDOW` alone (`CREATE_SUSPENDED | CREATE_NO_WINDOW`,
//! `CREATE_NEW_CONSOLE`) can still pass them to `creation_flags` directly.
//! The common case, no console window, is [`NoWindow::no_window`], which
//! covers both `std::process::Command` and `tokio::process::Command`.
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

/// Spawn a child without opening a console window for it.
///
/// Every crate that spawns a child on Windows needs the same three lines:
/// `#[cfg(windows)]`, `use std::os::windows::process::CommandExt`,
/// `cmd.creation_flags(CREATE_NO_WINDOW)`. Forgetting them is silent, and shows up
/// only as a console flashing over the splash on a user's desktop (#2171, #2173,
/// #3788). This trait makes it one call that is a no-op off Windows, and works for
/// both `std::process::Command` and `tokio::process::Command`.
///
/// `creation_flags` *replaces* the flags, it does not OR them, so a spawn that needs
/// other flags too (`CREATE_SUSPENDED | CREATE_NO_WINDOW`) or a different one
/// (`CREATE_NEW_CONSOLE`) still calls `creation_flags` itself.
pub trait NoWindow {
    fn no_window(&mut self) -> &mut Self;
}

impl NoWindow for std::process::Command {
    #[allow(unused_mut)]
    fn no_window(&mut self) -> &mut Self {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            self.creation_flags(CREATE_NO_WINDOW);
        }
        self
    }
}

impl NoWindow for tokio::process::Command {
    fn no_window(&mut self) -> &mut Self {
        // `tokio::process::Command` has `creation_flags` as an inherent method on Windows.
        #[cfg(windows)]
        {
            self.creation_flags(CREATE_NO_WINDOW);
        }
        self
    }
}

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

    /// `no_window` chains and compiles for both Command types on every platform; on
    /// Windows it must have set the flag (a spawned `cmd` would otherwise be the one
    /// that flashes), which `creation_flags` offers no way to read back, so this
    /// pins the chaining shape and the real effect is covered by the spawn inventory.
    #[test]
    fn no_window_chains_on_both_command_types() {
        let mut std_cmd = std::process::Command::new("agentmux-no-such-program");
        std_cmd.no_window().arg("x");
        let mut tokio_cmd = tokio::process::Command::new("agentmux-no-such-program");
        tokio_cmd.no_window().arg("x");
        assert_eq!(std_cmd.get_program(), "agentmux-no-such-program");
    }
}
