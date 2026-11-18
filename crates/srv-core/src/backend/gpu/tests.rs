// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::aggregate::ProcessFigures;
use super::*;
use crate::backend::rpc_types::{GpuAdapter, GpuAdapterKind};

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

#[test]
fn a_slow_read_widens_the_interval_and_a_cheap_one_brings_it_back() {
    let mut b = Budget::new(PANEL_EVERY, PANEL_MAX);
    assert_eq!((b.interval(), b.widened()), (ms(2000), false));
    assert_eq!(b.record(ms(80)), ms(4000));
    assert_eq!(b.record(ms(80)), ms(8000));
    assert_eq!(b.record(ms(80)), ms(10_000), "never past the longest");
    assert!(b.widened());
    // Between half the budget and the budget the interval stays where it is.
    assert_eq!(b.record(ms(40)), ms(10_000));
    assert_eq!(b.record(ms(5)), ms(5000));
    assert_eq!(b.record(ms(5)), ms(2500));
    assert_eq!(b.record(ms(5)), ms(2000), "never under the base");
    assert!(!b.widened());
}

fn adapter(kind: GpuAdapterKind, use_pct: Option<f64>, dedicated: (u64, u64)) -> GpuAdapter {
    GpuAdapter {
        luid: "0x00000000_0x00000001".into(),
        name: "GPU".into(),
        kind,
        use_pct,
        dedicated_used: Some(dedicated.0),
        dedicated_total: Some(dedicated.1),
        shared_used: Some(1 << 30),
        shared_total: Some(8 << 30),
    }
}

#[test]
fn the_status_bar_shows_the_busiest_adapter_and_the_fullest_memory() {
    let reading = Reading {
        adapters: vec![
            adapter(GpuAdapterKind::Integrated, Some(10.0), (0, 128 << 20)),
            adapter(GpuAdapterKind::Discrete, Some(35.0), (9 << 30, 10 << 30)),
        ],
        ..Default::default()
    };
    assert_eq!(compact_of(&reading), Some(Compact { use_pct: 35.0, mem_pct: Some(90.0) }));
}

#[test]
fn when_no_gpu_is_readable_there_is_nothing_to_show_not_zero() {
    assert_eq!(compact_of(&Reading::default()), None);
    let unknown = Reading { adapters: vec![adapter(GpuAdapterKind::Software, None, (0, 0))], ..Default::default() };
    assert_eq!(compact_of(&unknown), None);
}

fn figures(pid: u32, use_pct: f64, dedicated_mb: u64) -> ProcessFigures {
    ProcessFigures { pid, use_pct: Some(use_pct), dedicated: Some(dedicated_mb << 20), shared: Some(0) }
}

fn names(pid: u32) -> Option<Named> {
    match pid {
        1..=9 => Some(Named {
            name: "agentmux-host.exe".into(),
            label: Some(format!("Renderer {pid}")),
            kind: Some(TowerProcessKind::Agentmux),
        }),
        10..=99 => Some(Named { name: format!("app{pid}.exe"), label: None, kind: None }),
        _ => None,
    }
}

#[test]
fn the_panel_lists_agentmux_and_the_biggest_five_others_and_adds_up_the_rest() {
    let mut all: Vec<ProcessFigures> = (10..18).map(|p| figures(p, (p - 9) as f64 * 10.0, 100)).collect();
    all.push(figures(2, 5.0, 300));
    let (rows, others) = shape_processes(&all, names);
    assert_eq!(rows.len(), 1 + OTHERS_LISTED);
    // By use first: app17 at 80%, then app16 at 70%, ... AgentMux's 5% row last of those listed.
    assert_eq!(rows.iter().map(|r| r.pid).collect::<Vec<_>>(), [17, 16, 15, 14, 13, 2]);
    let tail = others.unwrap();
    assert_eq!(tail.processes, 3);
    assert_eq!(tail.use_pct, Some(60.0), "app12 at 30%, app11 at 20% and app10 at 10%");
    assert_eq!(tail.dedicated, Some(300 << 20));
}

#[test]
fn agentmuxs_own_row_comes_first_in_a_tie() {
    let all = vec![figures(10, 20.0, 100), figures(3, 20.4, 100)];
    let (rows, _) = shape_processes(&all, names);
    assert_eq!(rows.iter().map(|r| r.pid).collect::<Vec<_>>(), [3, 10]);
}

#[test]
fn a_tie_in_use_goes_to_the_one_with_more_memory() {
    let all = vec![figures(10, 20.0, 100), figures(11, 20.0, 400)];
    let (rows, _) = shape_processes(&all, names);
    assert_eq!(rows[0].pid, 11);
}

#[test]
fn a_process_is_named_by_its_image_and_an_unknown_one_by_its_number() {
    let (rows, _) = shape_processes(&[figures(11, 5.0, 1), figures(500, 5.0, 1)], names);
    assert_eq!(rows.iter().find(|r| r.pid == 11).unwrap().name, "app11.exe");
    assert_eq!(rows.iter().find(|r| r.pid == 500).unwrap().name, "pid 500");
}

#[test]
fn a_process_that_uses_nothing_has_no_row_and_one_that_only_holds_memory_has() {
    let idle = ProcessFigures { pid: 10, use_pct: Some(0.0), dedicated: Some(0), shared: Some(0) };
    let holds = ProcessFigures { pid: 11, use_pct: Some(0.0), dedicated: Some(64 << 20), shared: Some(0) };
    let (rows, others) = shape_processes(&[idle, holds], names);
    assert_eq!(rows.iter().map(|r| r.pid).collect::<Vec<_>>(), [11]);
    assert!(others.is_none());
}

#[cfg(not(windows))]
#[test]
fn where_the_figures_are_not_read_yet_the_sampler_says_so() {
    let (sample, _, _) = GpuSampler::new().sample_for_panel();
    assert_eq!(sample.state, GpuSampleState::Unsupported);
    assert_eq!(CompactPoller::default().poll(&GpuSampler::new()), None);
}

#[test]
fn the_poller_reads_once_per_interval_not_per_tick() {
    let sampler = GpuSampler::new();
    let mut poller = CompactPoller::default();
    poller.poll(&sampler);
    let first = poller.next;
    assert!(first.is_some());
    poller.poll(&sampler);
    poller.poll(&sampler);
    assert_eq!(poller.next, first, "ticks between reads leave the schedule alone");
}
