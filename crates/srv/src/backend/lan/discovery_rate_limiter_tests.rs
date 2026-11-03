// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The token bucket on LAN pubkey fan-outs (`LookupRateLimiter`).

use super::LookupRateLimiter;

#[test]
fn allows_up_to_max_tokens_per_window() {
    let mut limiter = LookupRateLimiter::new(3);
    assert!(limiter.check());
    assert!(limiter.check());
    assert!(limiter.check());
    assert!(!limiter.check(), "a 4th check within the same second must be denied");
}

#[test]
fn refills_after_the_window_elapses() {
    let mut limiter = LookupRateLimiter::new(1);
    assert!(limiter.check());
    assert!(!limiter.check());
    // Simulate the refill window having passed rather than sleeping in
    // a test — same approach as the existing RATE_LIMIT_MAX tests use
    // conceptually, just directly on the struct field since this type
    // has no injectable clock.
    limiter.last_refill -= std::time::Duration::from_secs(2);
    assert!(limiter.check(), "must refill once a full second has elapsed");
}
