// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Where a jekt for a target no tier found live goes (`absent_route_for`).

use super::*;
use crate::muxbus::wan_lease::Holder;

#[test]
fn the_lease_answer_picks_the_route() {
    assert_eq!(absent_route_for(&Holder::Free), AbsentRoute::HoldHere);
    assert_eq!(absent_route_for(&Holder::Yours), AbsentRoute::HoldHere);
    // No relay to send through: keep it here.
    assert_eq!(absent_route_for(&Holder::Unreachable), AbsentRoute::HoldHere);
    assert_eq!(absent_route_for(&Holder::Other(crate::muxbus::wan_lease::HeldBy { desc: "computer area54".into(), ..Default::default() })), AbsentRoute::Relay);
    // An older relay, or one that won't say: the order before this plan.
    assert_eq!(absent_route_for(&Holder::Unsupported), AbsentRoute::Relay);
    assert_eq!(absent_route_for(&Holder::Unknown), AbsentRoute::Relay);
}
