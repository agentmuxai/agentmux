// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `ReactiveHandler`'s needs-review hooks: which deliveries fire them.

use super::*;

fn resp(success: bool, requires_stop: Option<bool>, block: Option<&str>) -> InjectionResponse {
    InjectionResponse {
        deferred: None,
        success,
        request_id: "r".into(),
        block_id: block.map(Into::into),
        error: None,
        timestamp: 0,
        effective_tier: None,
        requires_stop,
        channel_verified: None,
    }
}

#[test]
fn hook_fires_only_for_successful_escalate_required_deliveries() {
    let h = ReactiveHandler::new();
    let hits = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let h2 = hits.clone();
    h.add_needs_review_hook(std::sync::Arc::new(move |b: &str| h2.lock().unwrap().push(b.to_string())));
    h.after_delivery(&resp(true, Some(true), Some("b1")));
    h.after_delivery(&resp(true, Some(false), Some("b2"))); // ESCALATE=none
    h.after_delivery(&resp(true, None, Some("b3")));        // not sensitive
    h.after_delivery(&resp(false, Some(true), Some("b4"))); // delivery failed
    h.after_delivery(&resp(true, Some(true), None));        // no block
    assert_eq!(*hits.lock().unwrap(), vec!["b1".to_string()]);
}

#[test]
fn every_registered_hook_is_called() {
    let h = ReactiveHandler::new();
    let hits = std::sync::Arc::new(std::sync::Mutex::new(0u32));
    for _ in 0..2 {
        let h2 = hits.clone();
        h.add_needs_review_hook(std::sync::Arc::new(move |_: &str| *h2.lock().unwrap() += 1));
    }
    h.after_delivery(&resp(true, Some(true), Some("b1")));
    assert_eq!(*hits.lock().unwrap(), 2, "a second router must not replace the first");
}
