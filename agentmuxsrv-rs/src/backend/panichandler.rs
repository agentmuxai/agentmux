// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

//! Panic handler with optional telemetry integration.
//! Port of Go's pkg/panichandler/.

#![allow(dead_code)]

use std::sync::Mutex;

/// Type alias for the telemetry handler function.
/// Called with the panic type string when a panic is caught.
type PanicTelemetryFn = Box<dyn Fn(&str) + Send + Sync>;

/// Global telemetry handler. Set once at startup to enable panic telemetry.
static PANIC_TELEMETRY_HANDLER: Mutex<Option<PanicTelemetryFn>> = Mutex::new(None);

/// Set the global panic telemetry handler.
pub fn set_panic_telemetry_handler<F>(handler: F)
where
    F: Fn(&str) + Send + Sync + 'static,
{
    let mut guard = PANIC_TELEMETRY_HANDLER.lock().unwrap();
    *guard = Some(Box::new(handler));
}

/// Handle a panic without sending telemetry.
/// Logs the panic info and returns an error.
pub fn panic_handler_no_telemetry(debug_str: &str, panic_info: &str) -> String {
    let msg = format!("[panic] {}: {}", debug_str, panic_info);
    tracing::error!("{}", msg);
    msg
}

/// Handle a panic with optional telemetry.
/// Logs the panic, sends to telemetry handler if set, returns an error message.
pub fn panic_handler(debug_str: &str, panic_info: &str) -> String {
    let msg = format!("[panic] {}: {}", debug_str, panic_info);
    tracing::error!("{}", msg);

    // Send to telemetry handler if registered
    if let Ok(guard) = PANIC_TELEMETRY_HANDLER.lock() {
        if let Some(ref handler) = *guard {
            handler(debug_str);
        }
    }

    msg
}

/// Run a closure with panic catching. Returns Ok(T) on success, Err(String) on panic.
pub fn catch_panic<F, T>(debug_str: &str, f: F) -> Result<T, String>
where
    F: FnOnce() -> T + std::panic::UnwindSafe,
{
    match std::panic::catch_unwind(f) {
        Ok(val) => Ok(val),
        Err(panic_val) => {
            let panic_str = if let Some(s) = panic_val.downcast_ref::<&str>() {
                s.to_string()
            } else if let Some(s) = panic_val.downcast_ref::<String>() {
                s.clone()
            } else {
                "unknown panic".to_string()
            };
            Err(panic_handler(debug_str, &panic_str))
        }
    }
}

// ---- Tests ----

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    #[test]
    fn test_panic_handler_no_telemetry() {
        let msg = panic_handler_no_telemetry("test-context", "something broke");
        assert!(msg.contains("test-context"));
        assert!(msg.contains("something broke"));
    }

    #[test]
    fn test_panic_handler_with_telemetry() {
        let called = Arc::new(AtomicBool::new(false));
        let called_clone = called.clone();

        set_panic_telemetry_handler(move |_panic_type| {
            called_clone.store(true, Ordering::SeqCst);
        });

        let msg = panic_handler("test-panic", "error details");
        assert!(msg.contains("test-panic"));
        assert!(called.load(Ordering::SeqCst));
    }

    #[test]
    fn test_catch_panic_success() {
        let result = catch_panic("test", || 42);
        assert_eq!(result.unwrap(), 42);
    }

    #[test]
    fn test_catch_panic_failure() {
        let result = catch_panic::<_, ()>("test-catch", || {
            panic!("deliberate test panic");
        });
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("test-catch"));
        assert!(err.contains("deliberate test panic"));
    }
}
