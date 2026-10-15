// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The jekt handler's rate limiter: at most `max_tokens` deliveries a second.

use std::time::{Duration, Instant};

pub(super) struct RateLimiter {
    tokens: u32,
    max_tokens: u32,
    last_refill: Instant,
}

impl RateLimiter {
    pub(super) fn new(max_tokens: u32) -> Self {
        Self {
            tokens: max_tokens,
            max_tokens,
            last_refill: Instant::now(),
        }
    }

    pub(super) fn check(&mut self) -> bool {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill);
        if elapsed >= Duration::from_secs(1) {
            self.tokens = self.max_tokens;
            self.last_refill = now;
        }
        if self.tokens > 0 {
            self.tokens -= 1;
            true
        } else {
            false
        }
    }
}
