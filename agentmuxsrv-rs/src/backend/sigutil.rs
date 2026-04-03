// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Signal handling utilities.
//! Port of Go's `pkg/util/sigutil/sigutil.go`.
//!
//! Installs OS signal handlers (SIGTERM, SIGINT, SIGHUP) that call a
//! shutdown callback. Uses tokio for async signal handling.


/// Install shutdown signal handlers that call `do_shutdown` with a description.
///
/// Spawns a tokio task that waits for SIGTERM, SIGINT, or SIGHUP (on Unix)
/// and calls the provided callback.
///
/// # Panics
/// Panics if called outside a tokio runtime.
#[cfg(unix)]
pub fn install_shutdown_signal_handlers<F>(do_shutdown: F)
where
    F: Fn(String) + Send + 'static,
{
    tokio::spawn(async move {
        use tokio::signal::unix::{signal, SignalKind};

        let mut sigterm = signal(SignalKind::terminate()).expect("failed to register SIGTERM");
        let mut sigint = signal(SignalKind::interrupt()).expect("failed to register SIGINT");
        let mut sighup = signal(SignalKind::hangup()).expect("failed to register SIGHUP");

        tokio::select! {
            _ = sigterm.recv() => do_shutdown("got signal SIGTERM".to_string()),
            _ = sigint.recv() => do_shutdown("got signal SIGINT".to_string()),
            _ = sighup.recv() => do_shutdown("got signal SIGHUP".to_string()),
        }
    });
}

/// Install shutdown signal handlers (Windows version).
///
/// On Windows, only Ctrl+C (SIGINT equivalent) is supported.
#[cfg(not(unix))]
pub fn install_shutdown_signal_handlers<F>(do_shutdown: F)
where
    F: Fn(String) + Send + 'static,
{
    tokio::spawn(async move {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to register ctrl-c handler");
        do_shutdown("got signal ctrl-c".to_string());
    });
}

#[cfg(test)]
mod tests {
    // Signal handler tests are inherently integration-level since they require
    // a running tokio runtime and actual signal delivery. We test the API
    // compiles and basic setup works.

    #[test]
    fn test_handler_type_signature() {
        // Verify the callback signature is correct
        fn _assert_send<F: Fn(String) + Send + 'static>(_f: F) {}
        _assert_send(|msg: String| {
            let _ = msg;
        });
    }
}
