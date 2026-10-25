// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Makes the build-slot shims win in the shells this wrapper starts.
//!
//! Every Bash call of an agent runs under this process, and bash imports functions from its
//! environment, so exporting `cargo`, `task`, `npm`, `npx` and `docker` as functions that call the
//! shims (`agentmux_common::build_slot::shim_function_env`) puts the slot in front of those
//! tools however the shell's profile ordered the PATH. Without it, a `cargo.exe` in `~/bin`
//! (ahead of the shims' directory in the CLI's shell snapshot) is found first and the slot is
//! skipped: docs/specs/SPEC_BUILD_SLOT_ENFORCEMENT_2026_10_20.md §4.

/// Sets the functions in this process's environment, which the bash it starts inherits.
pub(crate) fn export() {
    for (name, body) in agentmux_common::build_slot::shim_function_env_for_current_user() {
        std::env::set_var(name, body);
    }
}

#[cfg(test)]
mod tests {
    /// A variable set with `std::env::set_var` under the name bash uses for exported functions
    /// reaches a bash started afterwards as a function (the mechanism `export` relies on).
    #[test]
    fn an_exported_function_variable_reaches_a_child_bash() {
        let _guard = crate::test_env_lock::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let Ok(bash) = crate::bash_wrap::locate_bash() else { return };
        std::env::set_var("BASH_FUNC_agentmux_probe%%", "() {  echo from-function \"$@\"\n}");
        let out = std::process::Command::new(bash).args(["-c", "agentmux_probe a b"]).output().unwrap();
        std::env::remove_var("BASH_FUNC_agentmux_probe%%");
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "from-function a b");
    }
}
