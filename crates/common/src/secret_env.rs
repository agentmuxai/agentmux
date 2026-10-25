// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Taking a secret out of the process environment for good.
//!
//! The launcher hands srv and the host `AGENTMUX_AUTH_KEY` and
//! `AGENTMUX_HOST_REG_SECRET` in their environment. `std::env::remove_var`
//! drops the variable from the process's own view (and from the environment of
//! any process it starts afterwards), but on Unix the strings stay where the
//! kernel put them at `exec`, and `/proc/<pid>/environ` (Linux) and
//! `ps eww` / `KERN_PROCARGS2` (macOS) read exactly that block, for any process
//! of the same user. Wiping overwrites the value in place before removing it.
//!
//! On Windows `SetEnvironmentVariable` rewrites the process's live environment
//! block, which is what another process reads, so removing the variable there
//! is enough.
//!
//! The two startup secrets are moved out of the environment as the first thing
//! srv and the host do ([`stash_startup_secrets`]), before they start anything,
//! so no process they spawn inherits them; the code that needs a value takes it
//! from the stash with [`take`].

use std::collections::HashMap;
use std::sync::Mutex;

/// What the launcher hands srv and the host in their environment.
pub const STARTUP_SECRETS: &[&str] = &["AGENTMUX_AUTH_KEY", "AGENTMUX_HOST_REG_SECRET"];

static STASH: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

fn stash_map() -> std::sync::MutexGuard<'static, Option<HashMap<String, String>>> {
    STASH.lock().unwrap_or_else(|e| e.into_inner())
}

/// Moves [`STARTUP_SECRETS`] out of the environment into the stash (wiped from
/// the environment block in place on Unix). Call it first thing in `main`.
pub fn stash_startup_secrets() {
    stash(STARTUP_SECRETS);
}

/// Moves each named variable that is set and not empty into the stash.
pub fn stash(names: &[&str]) {
    for name in names {
        if let Some(value) = take_from_env(name).filter(|v| !v.is_empty()) {
            stash_map().get_or_insert_with(HashMap::new).insert((*name).to_string(), value);
        }
    }
}

/// The stashed secrets as variables, for a process that re-executes itself and
/// needs them again in its new environment.
pub fn stashed() -> Vec<(String, String)> {
    stash_map().as_ref().map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect()).unwrap_or_default()
}

/// Whether `name` has a non-empty value, stashed or still in the environment.
pub fn is_set(name: &str) -> bool {
    stash_map().as_ref().is_some_and(|m| m.contains_key(name)) || std::env::var(name).is_ok_and(|v| !v.is_empty())
}

/// The value of `name`: the stashed one, else (a process that never stashed, a
/// test) the environment's, wiped from there as it is read. None when unset.
pub fn take(name: &str) -> Option<String> {
    let stashed = stash_map().as_mut().and_then(|m| m.remove(name));
    stashed.or_else(|| take_from_env(name))
}

/// Reads `name`, wipes its value from the process environment (in place on
/// Unix), removes the variable, and returns the value. None when it is unset or
/// not valid UTF-8 (the variable is removed either way).
fn take_from_env(name: &str) -> Option<String> {
    let value = std::env::var(name).ok();
    #[cfg(unix)]
    wipe_in_place(name);
    std::env::remove_var(name);
    value
}

/// Overwrites the bytes `getenv(name)` points at with zeros. The pointer is
/// into the environment block (the kernel's copy from `exec`, or the allocation
/// of a later `setenv`), which stays writable and the same size for the life of
/// the variable.
#[cfg(unix)]
fn wipe_in_place(name: &str) {
    let Ok(c_name) = std::ffi::CString::new(name) else { return };
    // SAFETY: getenv returns a NUL-terminated string that lives until the
    // variable is changed, which this function's caller does next, in that
    // order and on this thread. Writing zeros over its own length stays inside
    // the allocation.
    unsafe {
        let value = libc::getenv(c_name.as_ptr());
        if value.is_null() {
            return;
        }
        let len = libc::strlen(value);
        std::ptr::write_bytes(value as *mut u8, 0, len);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // These mutate process-global state; each uses its own variable name.

    #[test]
    fn returns_the_value_and_removes_the_variable() {
        std::env::set_var("AM_SECRET_ENV_TEST_A", "the-secret-value");
        assert_eq!(take("AM_SECRET_ENV_TEST_A").as_deref(), Some("the-secret-value"));
        assert!(std::env::var("AM_SECRET_ENV_TEST_A").is_err());
        assert_eq!(take("AM_SECRET_ENV_TEST_A"), None);
    }

    #[test]
    fn an_unset_variable_is_none() {
        assert_eq!(take("AM_SECRET_ENV_TEST_NEVER_SET"), None);
    }

    #[test]
    fn a_stashed_secret_is_out_of_the_environment_and_still_there_to_take_once() {
        std::env::set_var("AM_SECRET_ENV_TEST_C", "stash-me");
        std::env::set_var("AM_SECRET_ENV_TEST_C_EMPTY", "");
        stash(&["AM_SECRET_ENV_TEST_C", "AM_SECRET_ENV_TEST_C_EMPTY", "AM_SECRET_ENV_TEST_C_UNSET"]);
        // Gone from the environment, so a process started now does not inherit it.
        assert!(std::env::var("AM_SECRET_ENV_TEST_C").is_err());
        assert!(std::env::var("AM_SECRET_ENV_TEST_C_EMPTY").is_err());
        // What a child started now would inherit is the process's environment as it stands.
        assert!(std::env::vars().all(|(k, v)| k != "AM_SECRET_ENV_TEST_C" && v != "stash-me"));
        assert!(is_set("AM_SECRET_ENV_TEST_C"));
        assert!(!is_set("AM_SECRET_ENV_TEST_C_EMPTY"));
        assert!(stashed().contains(&("AM_SECRET_ENV_TEST_C".to_string(), "stash-me".to_string())));
        assert_eq!(take("AM_SECRET_ENV_TEST_C").as_deref(), Some("stash-me"));
        assert_eq!(take("AM_SECRET_ENV_TEST_C"), None);
        assert!(!is_set("AM_SECRET_ENV_TEST_C"));
    }

    // What /proc/<pid>/environ shows is the block the value lives in, so look
    // at those bytes directly: after the wipe the string getenv points at is empty
    // and every byte of the old value is zero.
    #[cfg(unix)]
    #[test]
    fn the_bytes_the_environment_block_held_are_zero_after_the_wipe() {
        let secret = "wipe-me-0123456789";
        std::env::set_var("AM_SECRET_ENV_TEST_B", secret);
        let c_name = std::ffi::CString::new("AM_SECRET_ENV_TEST_B").unwrap();
        // SAFETY: a getenv pointer is valid until the variable is changed, and nothing
        // changes it between here and the reads below (the wipe writes through it).
        let ptr = unsafe { libc::getenv(c_name.as_ptr()) } as *const u8;
        wipe_in_place("AM_SECRET_ENV_TEST_B");
        let bytes = unsafe { std::slice::from_raw_parts(ptr, secret.len()) };
        assert!(bytes.iter().all(|b| *b == 0), "the secret's bytes are still in the block: {bytes:?}");
        assert_eq!(std::env::var("AM_SECRET_ENV_TEST_B").as_deref(), Ok(""));
        std::env::remove_var("AM_SECRET_ENV_TEST_B");
    }
}
