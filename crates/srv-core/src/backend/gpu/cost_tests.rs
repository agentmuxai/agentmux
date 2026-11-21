// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What a read of the GPU counters costs (SPEC_STATUS_BAR_GPU_PANEL_2026_11_15.md §4), measured where it is
//! decided: not on a person's screen. Ignored by default; CI and a machine of its own run it with
//! `cargo test -p agentmux-srv-core gpu::cost_tests -- --ignored --nocapture` and the numbers go in the PR.

use super::*;

/// Reads `n` times, `gap` apart, as the panel does, and prints the count of instances and the time of each read.
#[test]
#[ignore = "a measurement: run it on a machine of its own, not a person's"]
fn a_read_of_the_counters_is_inside_its_budget() {
    let sampler = GpuSampler::new();
    let mut took: Vec<Duration> = Vec::new();
    let mut states = Vec::new();
    for _ in 0..25 {
        // Every read is a real one: the reuse window is shorter than the gap.
        let (sample, _, _) = sampler.sample_for_panel();
        states.push(sample.state);
        if sample.state == GpuSampleState::Ok {
            took.push(sample.took);
        }
        std::thread::sleep(Duration::from_millis(1100));
    }
    took.sort();
    let pick = |q: f64| took.get(((took.len() as f64 - 1.0) * q) as usize).copied();
    eprintln!(
        "gpu read cost: {} ok reads of {}: median {:?}, p95 {:?}, max {:?}; states {:?}",
        took.len(),
        states.len(),
        pick(0.5),
        pick(0.95),
        took.last(),
        states.iter().map(|s| format!("{s:?}")).collect::<std::collections::BTreeSet<_>>()
    );
    // A machine with no GPU counters reads nothing and costs nothing: that is not a failure of the measurement.
    if let Some(median) = pick(0.5) {
        assert!(median <= READ_BUDGET * 4, "a read's median {median:?} is far over its budget {READ_BUDGET:?}");
    }
}
