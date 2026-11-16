// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;

const LOGIN: &str = "    \"/Users/me/Library/Keychains/login.keychain-db\"\n";

#[test]
fn a_default_keychain_on_disk_is_usable() {
    assert_eq!(keychain_problem(true, LOGIN, |_| true), None);
}

#[test]
fn no_default_keychain_is_a_problem() {
    // `security` exits non-zero and prints "A default keychain could not be found." on stderr.
    assert_eq!(keychain_problem(false, "", |_| true), Some(NO_DEFAULT_KEYCHAIN));
}

#[test]
fn a_default_keychain_that_is_not_on_disk_is_a_problem() {
    assert_eq!(keychain_problem(true, LOGIN, |_| false), Some(NO_DEFAULT_KEYCHAIN));
}

#[test]
fn an_answer_that_cannot_be_read_blocks_nothing() {
    assert_eq!(keychain_problem(true, "", |_| false), None);
    assert_eq!(keychain_problem(true, "something else\n", |_| false), None);
    assert_eq!(keychain_problem(true, "\"\"\n", |_| false), None);
}

#[test]
fn the_path_asked_about_is_the_one_printed() {
    let seen = std::cell::RefCell::new(String::new());
    keychain_problem(true, LOGIN, |p| {
        *seen.borrow_mut() = p.to_string_lossy().into_owned();
        true
    });
    assert_eq!(*seen.borrow(), "/Users/me/Library/Keychains/login.keychain-db");
}
