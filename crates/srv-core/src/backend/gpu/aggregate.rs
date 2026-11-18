// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A read of the GPU counters, made into what the status bar and the panel show: per adapter its use and
//! memory, per process its use and memory. Pure: the counters' instances in, figures out, so every case
//! (several adapters, an adapter no process uses, a counter set the machine lacks) is a test on any platform.

use super::names::{parse_adapter, parse_engine, parse_process_memory};
use crate::backend::rpc_types::{GpuAdapter, GpuAdapterKind};
use std::collections::HashMap;

/// One counter's instances: the name and the value.
pub type Instances = Vec<(String, f64)>;

/// What one read of the counters returned. `None` is a counter set the machine doesn't have, which is not the
/// same as one with no instances (an adapter no process is using).
#[derive(Debug, Default, Clone)]
pub struct RawCounters {
    /// `GPU Engine\Utilization Percentage`, a process's use of one engine, percent.
    pub engines: Option<Instances>,
    /// `GPU Adapter Memory\Dedicated Usage` and `\Shared Usage`, bytes.
    pub adapter_dedicated: Option<Instances>,
    pub adapter_shared: Option<Instances>,
    /// `GPU Process Memory\Dedicated Usage` and `\Shared Usage`, bytes: read only while the panel is open.
    pub process_dedicated: Option<Instances>,
    pub process_shared: Option<Instances>,
}

/// What DXGI says an adapter is: its name and the memory it was built with. The counters name an adapter by its id only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterInfo {
    pub luid: String,
    pub name: String,
    pub software: bool,
    pub dedicated_total: u64,
    pub shared_total: u64,
}

/// One process's figures, before it is named.
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessFigures {
    pub pid: u32,
    pub use_pct: Option<f64>,
    pub dedicated: Option<u64>,
    pub shared: Option<u64>,
}

/// A read, in figures.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Reading {
    pub adapters: Vec<GpuAdapter>,
    pub processes: Vec<ProcessFigures>,
    /// The per-process memory was read (the panel asked), so a row's memory is known rather than absent.
    pub per_process_memory: bool,
}

/// Below this much dedicated memory an adapter is the CPU's own graphics, which use system memory: its
/// memory pressure is the shared figure. (An integrated GPU's "dedicated" is a carve-out of 64 MB to 512 MB.)
const INTEGRATED_BELOW: u64 = 1 << 30;

/// Whether a name is the CPU's own graphics. DXGI has no flag for it (an AMD APU reserves a gigabyte or two, which looks
/// like a card), so the names of the CPU makers' graphics are the other half of the rule: Intel's, except its Arc cards,
/// and AMD's "Radeon Graphics" and "Vega n Graphics", which end in "Graphics" where a card ends in a model.
fn named_integrated(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    if n.contains("intel") {
        return !n.contains("arc");
    }
    (n.contains("radeon") || n.contains("amd")) && n.trim_end().ends_with("graphics") && !n.contains(" rx ")
}

/// An adapter's kind, from what DXGI says.
fn kind_of(info: &AdapterInfo) -> GpuAdapterKind {
    if info.software {
        GpuAdapterKind::Software
    } else if info.dedicated_total < INTEGRATED_BELOW || named_integrated(&info.name) {
        GpuAdapterKind::Integrated
    } else {
        GpuAdapterKind::Discrete
    }
}

/// An adapter's use of its memory, as a share the status bar can turn a warning on: dedicated for a card with its
/// own memory, shared for one that uses the machine's.
pub fn memory_share(a: &GpuAdapter) -> Option<f64> {
    let (used, total) = match a.kind {
        GpuAdapterKind::Discrete => (a.dedicated_used?, a.dedicated_total?),
        _ => (a.shared_used?, a.shared_total?),
    };
    (total > 0).then(|| (used as f64 / total as f64 * 100.0).clamp(0.0, 100.0))
}

fn bytes(v: f64) -> u64 {
    if v.is_finite() && v > 0.0 {
        v as u64
    } else {
        0
    }
}

/// Sum the instances by what `key` makes of their names; the instances `key` does not understand are left out.
fn sum_by<K: std::hash::Hash + Eq>(
    instances: &Instances,
    key: impl Fn(&str) -> Option<K>,
) -> HashMap<K, f64> {
    let mut sums = HashMap::new();
    for (name, v) in instances {
        if let Some(k) = key(name) {
            *sums.entry(k).or_insert(0.0) += v.max(0.0);
        }
    }
    sums
}

/// The figures of one read. `infos` are the adapters DXGI lists, which name them and give their memory; an adapter
/// the counters know and DXGI doesn't (one that went away) is shown by its id.
pub fn aggregate(raw: &RawCounters, infos: &[AdapterInfo]) -> Reading {
    // One engine's load is the sum of what every process asks of it; an adapter is as busy as its busiest engine
    // (what Task Manager shows too), and a process as busy as its engines added up, within a whole.
    let mut engine_load: HashMap<(String, String), f64> = HashMap::new();
    let mut process_use: HashMap<u32, HashMap<String, f64>> = HashMap::new();
    for (name, v) in raw.engines.iter().flatten() {
        let Some(e) = parse_engine(name) else { continue };
        *engine_load.entry((e.adapter.clone(), e.engine)).or_insert(0.0) += v.max(0.0);
        *process_use.entry(e.pid).or_default().entry(e.adapter).or_insert(0.0) += v.max(0.0);
    }
    let dedicated = raw.adapter_dedicated.as_ref().map(|i| sum_by(i, parse_adapter));
    let shared = raw.adapter_shared.as_ref().map(|i| sum_by(i, parse_adapter));

    let mut ids: Vec<String> = infos.iter().map(|i| i.luid.clone()).collect();
    let known = |luid: &str, ids: &mut Vec<String>| {
        if !ids.iter().any(|i| i == luid) {
            ids.push(luid.to_string());
        }
    };
    for (adapter, _) in engine_load.keys() {
        known(adapter, &mut ids);
    }
    for map in [&dedicated, &shared].into_iter().flatten() {
        for adapter in map.keys() {
            known(adapter, &mut ids);
        }
    }
    let mut adapters: Vec<GpuAdapter> = ids
        .into_iter()
        .map(|luid| {
            let info = infos.iter().find(|i| i.luid == luid);
            let busiest = engine_load
                .iter()
                .filter(|((a, _), _)| *a == luid)
                .map(|(_, v)| *v)
                .fold(0.0_f64, f64::max);
            let used = |m: &Option<HashMap<String, f64>>| m.as_ref().map(|m| bytes(m.get(&luid).copied().unwrap_or(0.0)));
            GpuAdapter {
                name: info.map_or_else(|| format!("GPU {luid}"), |i| i.name.clone()),
                kind: info.map_or(GpuAdapterKind::Discrete, kind_of),
                // A counter set the machine has, with nothing in it for this adapter: nothing is using it. The
                // software renderer has no engines to read: its use is unknown, not idle.
                use_pct: raw
                    .engines
                    .as_ref()
                    .filter(|_| !info.is_some_and(|i| i.software))
                    .map(|_| busiest.min(100.0)),
                dedicated_used: used(&dedicated),
                dedicated_total: info.map(|i| i.dedicated_total),
                shared_used: used(&shared),
                shared_total: info.map(|i| i.shared_total),
                luid,
            }
        })
        .collect();
    // DXGI always lists the OS's software renderer, which no one asked about: it is a row only when it is all there is.
    if adapters.iter().any(|a| a.kind != GpuAdapterKind::Software) {
        adapters.retain(|a| a.kind != GpuAdapterKind::Software);
    }

    let pdm = raw.process_dedicated.as_ref().map(|i| sum_by(i, |n| parse_process_memory(n).map(|(pid, _)| pid)));
    let psm = raw.process_shared.as_ref().map(|i| sum_by(i, |n| parse_process_memory(n).map(|(pid, _)| pid)));
    let mut pids: Vec<u32> = process_use.keys().copied().collect();
    for map in [&pdm, &psm].into_iter().flatten() {
        pids.extend(map.keys().copied().filter(|p| !process_use.contains_key(p)));
    }
    pids.sort_unstable();
    pids.dedup();
    let processes = pids
        .into_iter()
        .map(|pid| ProcessFigures {
            pid,
            // Per adapter, its busiest: a process using two adapters is shown at the one it uses most.
            use_pct: raw.engines.as_ref().map(|_| {
                process_use
                    .get(&pid)
                    .map_or(0.0, |per| per.values().copied().fold(0.0_f64, f64::max))
                    .min(100.0)
            }),
            dedicated: pdm.as_ref().map(|m| bytes(m.get(&pid).copied().unwrap_or(0.0))),
            shared: psm.as_ref().map(|m| bytes(m.get(&pid).copied().unwrap_or(0.0))),
        })
        .collect();
    Reading { adapters, processes, per_process_memory: pdm.is_some() || psm.is_some() }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "0x00000000_0x0000F3A8";
    const B: &str = "0x00000000_0x00010AAA";

    fn inst(name: &str, v: f64) -> (String, f64) {
        (name.to_string(), v)
    }
    fn engine(pid: u32, luid: &str, eng: u32, ty: &str, v: f64) -> (String, f64) {
        inst(&format!("pid_{pid}_luid_{luid}_phys_0_eng_{eng}_engtype_{ty}"), v)
    }
    fn info(luid: &str, name: &str, dedicated_mb: u64) -> AdapterInfo {
        AdapterInfo {
            luid: luid.into(),
            name: name.into(),
            software: false,
            dedicated_total: dedicated_mb << 20,
            shared_total: 16 << 30,
        }
    }

    #[test]
    fn an_adapter_is_as_busy_as_its_busiest_engine() {
        let raw = RawCounters {
            engines: Some(vec![
                engine(10, A, 0, "3D", 30.0),
                engine(11, A, 0, "3D", 20.0),
                engine(10, A, 1, "Copy", 40.0),
            ]),
            ..Default::default()
        };
        let r = aggregate(&raw, &[info(A, "RTX", 8192)]);
        assert_eq!(r.adapters.len(), 1);
        assert_eq!(r.adapters[0].use_pct, Some(50.0));
    }

    #[test]
    fn a_process_is_as_busy_as_its_engines_added_up_within_a_whole() {
        let raw = RawCounters {
            engines: Some(vec![
                engine(10, A, 0, "3D", 70.0),
                engine(10, A, 1, "Copy", 50.0),
                engine(11, A, 0, "3D", 5.0),
            ]),
            ..Default::default()
        };
        let r = aggregate(&raw, &[info(A, "RTX", 8192)]);
        let p = |pid| r.processes.iter().find(|p| p.pid == pid).unwrap().use_pct;
        assert_eq!(p(10), Some(100.0));
        assert_eq!(p(11), Some(5.0));
    }

    #[test]
    fn several_adapters_are_one_row_each_and_a_process_is_shown_at_the_one_it_uses_most() {
        let raw = RawCounters {
            engines: Some(vec![engine(10, A, 0, "3D", 10.0), engine(10, B, 0, "3D", 60.0), engine(12, B, 0, "3D", 5.0)]),
            adapter_dedicated: Some(vec![
                inst("luid_0x00000000_0x0000F3A8_phys_0", 1e9),
                inst("luid_0x00000000_0x00010AAA_phys_0", 2e9),
            ]),
            ..Default::default()
        };
        let r = aggregate(&raw, &[info(A, "Intel UHD", 128), info(B, "RTX", 8192)]);
        assert_eq!(r.adapters.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["Intel UHD", "RTX"]);
        assert_eq!(r.adapters[0].kind, GpuAdapterKind::Integrated);
        assert_eq!(r.adapters[1].kind, GpuAdapterKind::Discrete);
        assert_eq!(r.adapters[0].use_pct, Some(10.0));
        assert_eq!(r.adapters[1].use_pct, Some(65.0));
        assert_eq!(r.adapters[1].dedicated_used, Some(2_000_000_000));
        assert_eq!(r.processes.iter().find(|p| p.pid == 10).unwrap().use_pct, Some(60.0));
    }

    #[test]
    fn the_cpus_own_graphics_are_integrated_whatever_they_reserve() {
        let kind = |name: &str, mb: u64| kind_of(&info(A, name, mb));
        assert_eq!(kind("AMD Radeon(TM) Graphics", 2048), GpuAdapterKind::Integrated);
        assert_eq!(kind("AMD Radeon(TM) Vega 8 Graphics", 2048), GpuAdapterKind::Integrated);
        assert_eq!(kind("Intel(R) UHD Graphics 770", 128), GpuAdapterKind::Integrated);
        assert_eq!(kind("Intel(R) Iris(R) Xe Graphics", 2048), GpuAdapterKind::Integrated);
        assert_eq!(kind("Intel(R) Arc(TM) A770 Graphics", 16384), GpuAdapterKind::Discrete);
        assert_eq!(kind("AMD Radeon RX 7900 XTX", 24576), GpuAdapterKind::Discrete);
        assert_eq!(kind("NVIDIA GeForce RTX 3060", 12288), GpuAdapterKind::Discrete);
    }

    #[test]
    fn the_software_renderer_is_a_row_only_when_it_is_all_there_is() {
        let soft = AdapterInfo { software: true, ..info(B, "Microsoft Basic Render Driver", 0) };
        let both = aggregate(&RawCounters::default(), &[info(A, "RTX", 8192), soft.clone()]);
        assert_eq!(both.adapters.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["RTX"]);
        let alone = aggregate(&RawCounters::default(), &[soft]);
        assert_eq!(alone.adapters.len(), 1);
        assert_eq!(alone.adapters[0].kind, GpuAdapterKind::Software);
    }

    #[test]
    fn an_adapter_no_process_uses_is_idle_not_unknown() {
        let raw = RawCounters { engines: Some(vec![]), ..Default::default() };
        let r = aggregate(&raw, &[info(A, "RTX", 8192)]);
        assert_eq!(r.adapters[0].use_pct, Some(0.0));
    }

    #[test]
    fn a_missing_counter_set_leaves_its_figure_unknown_never_zero() {
        let raw = RawCounters::default();
        let r = aggregate(&raw, &[info(A, "RTX", 8192)]);
        assert_eq!(r.adapters.len(), 1);
        assert_eq!(r.adapters[0].use_pct, None);
        assert_eq!(r.adapters[0].dedicated_used, None);
        assert!(r.processes.is_empty());
        assert!(!r.per_process_memory);
    }

    #[test]
    fn no_counters_and_no_adapter_is_nothing_to_show() {
        assert_eq!(aggregate(&RawCounters::default(), &[]), Reading::default());
    }

    #[test]
    fn an_adapter_the_counters_know_and_dxgi_does_not_is_shown_by_its_id() {
        let raw = RawCounters { engines: Some(vec![engine(10, A, 0, "3D", 1.0)]), ..Default::default() };
        let r = aggregate(&raw, &[]);
        assert_eq!(r.adapters[0].name, format!("GPU {A}"));
        assert_eq!(r.adapters[0].dedicated_total, None);
    }

    #[test]
    fn a_process_memory_row_adds_the_processes_that_only_hold_memory() {
        let raw = RawCounters {
            engines: Some(vec![engine(10, A, 0, "3D", 1.0)]),
            process_dedicated: Some(vec![
                inst("pid_10_luid_0x00000000_0x0000F3A8_phys_0", 100.0),
                inst("pid_77_luid_0x00000000_0x0000F3A8_phys_0", 500.0),
            ]),
            process_shared: Some(vec![inst("pid_10_luid_0x00000000_0x0000F3A8_phys_0", 7.0)]),
            ..Default::default()
        };
        let r = aggregate(&raw, &[info(A, "RTX", 8192)]);
        assert!(r.per_process_memory);
        let get = |pid| r.processes.iter().find(|p| p.pid == pid).unwrap().clone();
        assert_eq!(get(10), ProcessFigures { pid: 10, use_pct: Some(1.0), dedicated: Some(100), shared: Some(7) });
        assert_eq!(get(77), ProcessFigures { pid: 77, use_pct: Some(0.0), dedicated: Some(500), shared: Some(0) });
    }

    #[test]
    fn the_memory_share_is_of_the_memory_the_adapter_really_uses() {
        let discrete = GpuAdapter {
            luid: A.into(),
            name: "RTX".into(),
            kind: GpuAdapterKind::Discrete,
            use_pct: Some(1.0),
            dedicated_used: Some(9 << 30),
            dedicated_total: Some(10 << 30),
            shared_used: Some(1 << 30),
            shared_total: Some(16 << 30),
        };
        assert_eq!(memory_share(&discrete), Some(90.0));
        let integrated = GpuAdapter { kind: GpuAdapterKind::Integrated, ..discrete.clone() };
        assert_eq!(memory_share(&integrated), Some(6.25));
        let unknown = GpuAdapter { dedicated_total: None, ..discrete };
        assert_eq!(memory_share(&unknown), None);
    }
}
