// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What the sign-in code tells the account episodes. A sign-in that completed, or a validate that passed, means
//! the account's credentials are good, so the panes stopped on a refused sign-in can go on. The episodes live in
//! the server layer, which registers its reader here once at startup ([`set_after_sign_in`]), so `identity` does
//! not name `server` (Area54 #1350). Before that registration there is nothing to tell.

use std::sync::OnceLock;

static AFTER_SIGN_IN: OnceLock<fn(&str)> = OnceLock::new();

/// Register what to run when an account's credentials were just proven good. Later calls change nothing.
pub fn set_after_sign_in(f: fn(&str)) {
    let _ = AFTER_SIGN_IN.set(f);
}

/// `account_id`'s credentials were just proven good. Does nothing before [`set_after_sign_in`].
pub fn after_sign_in(account_id: &str) {
    if let Some(f) = AFTER_SIGN_IN.get() {
        f(account_id);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    static CALLS: AtomicUsize = AtomicUsize::new(0);

    fn count(_: &str) {
        CALLS.fetch_add(1, Ordering::SeqCst);
    }

    #[test]
    fn it_tells_the_registered_reader_and_nothing_else() {
        // Nothing registered yet in this process, or the same reader if another test got here first.
        after_sign_in("acct-before");
        set_after_sign_in(count);
        let before = CALLS.load(Ordering::SeqCst);
        after_sign_in("acct-1");
        assert_eq!(CALLS.load(Ordering::SeqCst), before + 1);
        set_after_sign_in(|_| panic!("a later registration must change nothing"));
        after_sign_in("acct-2");
        assert_eq!(CALLS.load(Ordering::SeqCst), before + 2);
    }
}
