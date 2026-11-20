// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The text Linux gives about its GPUs, parsed (SPEC_STATUS_BAR_GPU_PANEL_2026_11_15.md §3, phase 2): a process's
//! `/proc/<pid>/fdinfo/<fd>` for a DRM device (kernel 5.14 and later; amdgpu, i915, xe), the `uevent` of a card,
//! and the PCI id database's names. Pure text and arithmetic, so every case is a test on any platform; `linux.rs`
//! reads the files.
//!
//! ```text
//! drm-driver:     amdgpu
//! drm-pdev:       0000:03:00.0
//! drm-client-id:  4083
//! drm-engine-gfx: 1234567890 ns
//! drm-memory-vram: 1048576 KiB
//! drm-memory-gtt:  2048 KiB
//! ```

use super::aggregate::ProcessFigures;
use crate::backend::rpc_types::GpuAdapterKind;
use std::collections::HashMap;
use std::time::Duration;

pub const VENDOR_INTEL: u32 = 0x8086;
pub const VENDOR_AMD: u32 = 0x1002;
pub const VENDOR_NVIDIA: u32 = 0x10de;

/// One open DRM client (a process's handle on a GPU), as its fdinfo shows it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DrmClient {
    pub driver: String,
    /// The PCI slot of the GPU it is on, lower case (`0000:03:00.0`), where the driver says.
    pub pdev: Option<String>,
    /// The kernel's id for this client: one process's several file descriptors on it share one.
    pub id: Option<u64>,
    /// Each engine's busy time so far, nanoseconds.
    pub engines: Vec<(String, u64)>,
    /// What it holds on the card, bytes: `None` where the driver doesn't say.
    pub dedicated: Option<u64>,
    /// What it holds of the machine's memory (GTT, system), bytes.
    pub shared: Option<u64>,
}

/// A number with its unit: `1024 KiB`, `5 MiB`, `123 ns`, `7`. Memory units are binary; no unit is the base unit.
fn amount(value: &str) -> Option<u64> {
    let mut parts = value.split_whitespace();
    let n: u64 = parts.next()?.parse().ok()?;
    let scale: u64 = match parts.next().map(str::to_ascii_lowercase).as_deref() {
        None | Some("b") | Some("ns") => 1,
        Some("kib") => 1 << 10,
        Some("mib") => 1 << 20,
        Some("gib") => 1 << 30,
        Some("tib") => 1 << 40,
        Some(_) => return None,
    };
    n.checked_mul(scale)
}

/// Where a memory region is: on the card, in the machine's memory, or somewhere a figure would mislead (the CPU-visible
/// domain amdgpu lists besides).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Placement {
    Dedicated,
    Shared,
    Neither,
}

fn placement(region: &str) -> Placement {
    // amdgpu: vram, gtt, cpu. i915: local0 (card memory), system0. xe: vram0, gtt.
    if region.starts_with("vram") || region.starts_with("local") {
        Placement::Dedicated
    } else if region.starts_with("gtt") || region.starts_with("system") {
        Placement::Shared
    } else {
        Placement::Neither
    }
}

/// What a region says it holds: what is resident if the driver says that, else what it names as its memory, else its total.
#[derive(Default)]
struct Region {
    resident: Option<u64>,
    memory: Option<u64>,
    total: Option<u64>,
}

/// A DRM fdinfo's figures; `None` for a file that is not one (no `drm-driver` line).
pub fn parse_fdinfo(text: &str) -> Option<DrmClient> {
    let mut client = DrmClient::default();
    let mut seen_driver = false;
    let mut regions: HashMap<String, Region> = HashMap::new();
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else { continue };
        let value = value.trim();
        match key {
            "drm-driver" => {
                client.driver = value.to_string();
                seen_driver = true;
            }
            "drm-pdev" => client.pdev = Some(value.to_ascii_lowercase()),
            "drm-client-id" => client.id = value.parse().ok(),
            _ => {
                if let Some(engine) = key.strip_prefix("drm-engine-") {
                    // `drm-engine-capacity-<name>` is how many units an engine has, not a time.
                    if !engine.starts_with("capacity-") {
                        if let Some(ns) = amount(value) {
                            client.engines.push((engine.to_string(), ns));
                        }
                    }
                } else if let Some(region) = key.strip_prefix("drm-resident-") {
                    regions.entry(region.to_string()).or_default().resident = amount(value);
                } else if let Some(region) = key.strip_prefix("drm-memory-") {
                    regions.entry(region.to_string()).or_default().memory = amount(value);
                } else if let Some(region) = key.strip_prefix("drm-total-") {
                    regions.entry(region.to_string()).or_default().total = amount(value);
                }
            }
        }
    }
    if !seen_driver {
        return None;
    }
    let held = |want: Placement| {
        regions
            .iter()
            .filter(|(name, _)| placement(name) == want)
            .filter_map(|(_, r)| r.resident.or(r.memory).or(r.total))
            .reduce(|a, b| a.saturating_add(b))
    };
    client.dedicated = held(Placement::Dedicated);
    client.shared = held(Placement::Shared);
    Some(client)
}

/// `KEY=value` lines (a `uevent`).
pub fn parse_uevent(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect()
}

/// `0x1002` (a PCI `vendor` or `device` file) as a number.
pub fn parse_pci_id(text: &str) -> Option<u32> {
    let t = text.trim();
    u32::from_str_radix(t.strip_prefix("0x").unwrap_or(t), 16).ok()
}

/// The vendor's short name, for a name the id database gives without one.
pub fn vendor_name(vendor: u32) -> &'static str {
    match vendor {
        VENDOR_AMD => "AMD",
        VENDOR_INTEL => "Intel",
        VENDOR_NVIDIA => "NVIDIA",
        _ => "GPU",
    }
}

/// A device's name in the PCI id database (`pci.ids`: a vendor line at the left edge, its devices one tab in).
pub fn pci_ids_device(db: &str, vendor: u32, device: u32) -> Option<String> {
    let vendor_key = format!("{vendor:04x}  ");
    let device_key = format!("\t{device:04x}  ");
    let mut in_vendor = false;
    for line in db.lines() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if !line.starts_with('\t') {
            // A new vendor (or a class line, which has a "C " prefix): the one we are in ends here.
            if in_vendor {
                return None;
            }
            in_vendor = line.to_ascii_lowercase().starts_with(&vendor_key);
        } else if in_vendor {
            if let Some(name) = line.to_ascii_lowercase().strip_prefix(&device_key) {
                // Keep the original case: cut the lowered line's length off the real one.
                let cut = line.len() - name.len();
                return Some(line[cut..].trim().to_string());
            }
        }
    }
    None
}

/// What to call a GPU: the marketing name in the database's brackets where it has one (`Navi 21 [Radeon RX 6800]`),
/// with the vendor in front unless the name carries it; else the database's name; else the ids.
pub fn display_name(vendor: u32, device: u32, from_db: Option<&str>) -> String {
    let maker = vendor_name(vendor);
    let Some(raw) = from_db else {
        return format!("{maker} GPU {vendor:04x}:{device:04x}");
    };
    let inner = match (raw.rfind('['), raw.rfind(']')) {
        (Some(open), Some(close)) if open < close => raw[open + 1..close].trim(),
        _ => raw.trim(),
    };
    if inner.to_ascii_lowercase().contains(&maker.to_ascii_lowercase()) {
        inner.to_string()
    } else {
        format!("{maker} {inner}")
    }
}

/// Memory a GPU below this has on its own is the CPU's graphics, which use the machine's (as on Windows).
const INTEGRATED_BELOW: u64 = 1 << 30;

/// Whether the CPU makers' own graphics are named: Intel's (except its Arc cards), AMD's "Radeon ... Graphics".
fn named_integrated(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    if n.contains("intel") {
        return !n.contains("arc");
    }
    (n.contains("radeon") || n.contains("amd")) && n.trim_end().ends_with("graphics") && !n.contains(" rx ")
}

/// An adapter's kind from its name and the memory it has of its own (`None`: it says none, as Intel's do).
pub fn kind_of(name: &str, dedicated_total: Option<u64>) -> GpuAdapterKind {
    if named_integrated(name) || dedicated_total.is_some_and(|t| t < INTEGRATED_BELOW) {
        GpuAdapterKind::Integrated
    } else {
        GpuAdapterKind::Discrete
    }
}

/// One DRM file descriptor of one process, read.
#[derive(Debug, Clone, PartialEq)]
pub struct Scanned {
    pub pid: u32,
    /// The number of the descriptor: a client that does not name itself is told apart by it.
    pub fd: u32,
    pub client: DrmClient,
}

/// A client's identity across two reads.
pub type ClientKey = (u32, String, u64);

fn key_of(s: &Scanned) -> ClientKey {
    // The kernel's client id is what several descriptors on one client share; a driver that gives none is counted per
    // descriptor (and tagged high, so it cannot meet a real id).
    (s.pid, s.client.pdev.clone().unwrap_or_default(), s.client.id.unwrap_or((1 << 63) | u64::from(s.fd)))
}

/// Reads this close together say nothing of how busy an engine was.
const MIN_ELAPSED: Duration = Duration::from_millis(100);

/// Each process's figures from the clients read, and the busy times to compare with at the next read.
///
/// A client's use is the time its engines were busy since `prev`, of the time between the reads; a process's is its
/// clients' added up within a whole (as on Windows). With nothing to compare with (the first read, or a client that is
/// new) the use is unknown, not zero. A process's memory is its clients' added up, each client once however many
/// descriptors it has.
pub fn process_figures(
    scanned: &[Scanned],
    prev: &HashMap<ClientKey, u64>,
    elapsed: Option<Duration>,
) -> (Vec<ProcessFigures>, HashMap<ClientKey, u64>) {
    let elapsed_ns = elapsed.filter(|e| *e >= MIN_ELAPSED).map(|e| e.as_nanos() as f64);
    let mut next: HashMap<ClientKey, u64> = HashMap::new();
    let mut by_pid: HashMap<u32, ProcessFigures> = HashMap::new();
    for s in scanned {
        let key = key_of(s);
        if next.contains_key(&key) {
            continue; // the same client on another descriptor
        }
        let busy: u64 = s.client.engines.iter().fold(0u64, |a, (_, ns)| a.saturating_add(*ns));
        next.insert(key.clone(), busy);
        let use_pct = match (elapsed_ns, prev.get(&key)) {
            (Some(dt), Some(before)) if !s.client.engines.is_empty() => Some((busy.saturating_sub(*before) as f64 / dt * 100.0).clamp(0.0, 100.0)),
            _ => None,
        };
        let p = by_pid.entry(s.pid).or_insert(ProcessFigures { pid: s.pid, use_pct: None, dedicated: None, shared: None });
        p.use_pct = match (p.use_pct, use_pct) {
            (Some(a), Some(b)) => Some((a + b).min(100.0)),
            (a, b) => a.or(b),
        };
        let add = |a: Option<u64>, b: Option<u64>| match (a, b) {
            (Some(a), Some(b)) => Some(a.saturating_add(b)),
            (a, b) => a.or(b),
        };
        p.dedicated = add(p.dedicated, s.client.dedicated);
        p.shared = add(p.shared, s.client.shared);
    }
    let mut out: Vec<ProcessFigures> = by_pid.into_values().collect();
    out.sort_by_key(|p| p.pid);
    (out, next)
}

/// What the clients hold on one GPU (by PCI slot), for an adapter that can't say itself: Intel's memory is the machine's,
/// and only the processes' own figures add up to how much of it the GPU holds.
pub fn held_on(scanned: &[Scanned], pdev: &str) -> (Option<u64>, Option<u64>) {
    let mut seen: HashMap<ClientKey, ()> = HashMap::new();
    let (mut dedicated, mut shared): (Option<u64>, Option<u64>) = (None, None);
    for s in scanned.iter().filter(|s| s.client.pdev.as_deref() == Some(pdev)) {
        if seen.insert(key_of(s), ()).is_some() {
            continue;
        }
        let add = |a: Option<u64>, b: Option<u64>| match (a, b) {
            (Some(a), Some(b)) => Some(a.saturating_add(b)),
            (a, b) => a.or(b),
        };
        dedicated = add(dedicated, s.client.dedicated);
        shared = add(shared, s.client.shared);
    }
    (dedicated, shared)
}

#[cfg(test)]
mod tests {
    use super::*;

    const AMDGPU: &str = "pos:\t0\nflags:\t02100002\nmnt_id:\t26\nino:\t331\n\
drm-driver:\tamdgpu\n\
drm-client-name:\t\n\
drm-pdev:\t0000:03:00.0\n\
drm-client-id:\t4083\n\
drm-engine-gfx:\t1234567890 ns\n\
drm-engine-compute:\t0 ns\n\
drm-engine-dec:\t77 ns\n\
drm-memory-vram:\t1048576 KiB\n\
drm-memory-gtt: \t2048 KiB\n\
drm-memory-cpu: \t16 KiB\n";

    const I915: &str = "drm-driver:\ti915\n\
drm-pdev:\t0000:00:02.0\n\
drm-client-id:\t12\n\
drm-engine-render:\t5000 ns\n\
drm-engine-capacity-render:\t1\n\
drm-total-system0:\t30 MiB\n\
drm-resident-system0:\t20 MiB\n";

    const XE: &str = "drm-driver:\txe\n\
drm-client-id:\t9\n\
drm-pdev:\t0000:0A:00.0\n\
drm-engine-rcs:\t100 ns\n\
drm-total-vram0:\t512 MiB\n\
drm-resident-vram0:\t256 MiB\n\
drm-total-gtt:\t64 MiB\n";

    #[test]
    fn an_amdgpu_client_names_its_engines_and_its_memory() {
        let c = parse_fdinfo(AMDGPU).unwrap();
        assert_eq!((c.driver.as_str(), c.pdev.as_deref(), c.id), ("amdgpu", Some("0000:03:00.0"), Some(4083)));
        assert_eq!(c.engines, [("gfx".to_string(), 1_234_567_890), ("compute".to_string(), 0), ("dec".to_string(), 77)]);
        assert_eq!(c.dedicated, Some(1 << 30), "1048576 KiB of video memory");
        assert_eq!(c.shared, Some(2 << 20), "the GTT; the CPU-visible domain is neither");
    }

    #[test]
    fn an_i915_client_uses_what_is_resident_and_skips_the_engine_capacity_line() {
        let c = parse_fdinfo(I915).unwrap();
        assert_eq!(c.engines, [("render".to_string(), 5000)]);
        assert_eq!((c.dedicated, c.shared), (None, Some(20 << 20)), "resident, not the total");
    }

    #[test]
    fn an_xe_client_lower_cases_its_slot_and_keeps_card_and_system_memory_apart() {
        let c = parse_fdinfo(XE).unwrap();
        assert_eq!(c.pdev.as_deref(), Some("0000:0a:00.0"));
        assert_eq!((c.dedicated, c.shared), (Some(256 << 20), Some(64 << 20)));
    }

    #[test]
    fn a_descriptor_that_is_not_a_drm_one_is_not_a_client() {
        assert_eq!(parse_fdinfo("pos:\t0\nflags:\t02\nmnt_id:\t15\n"), None);
        assert_eq!(parse_fdinfo(""), None);
        assert!(parse_fdinfo("drm-driver:\tamdgpu\n").is_some(), "no figures is still a client");
    }

    #[test]
    fn a_figure_with_a_unit_it_does_not_know_is_left_out_not_guessed() {
        let c = parse_fdinfo("drm-driver:\tx\ndrm-memory-vram:\t5 furlongs\ndrm-engine-gfx:\tlots\n").unwrap();
        assert_eq!((c.dedicated, c.engines.len()), (None, 0));
    }

    #[test]
    fn the_amounts_are_binary_and_bare_numbers_are_bytes() {
        assert_eq!(amount("1 KiB"), Some(1024));
        assert_eq!(amount("2 gib"), Some(2 << 30));
        assert_eq!(amount("700"), Some(700));
        assert_eq!(amount("12 ns"), Some(12));
        assert_eq!(amount("-3 KiB"), None);
        assert_eq!(amount(""), None);
    }

    #[test]
    fn a_uevent_and_a_pci_id_file_are_read() {
        let u = parse_uevent("DRIVER=amdgpu\nPCI_CLASS=30000\nPCI_ID=1002:73BF\nPCI_SLOT_NAME=0000:03:00.0\n");
        assert_eq!(u.get("DRIVER").map(String::as_str), Some("amdgpu"));
        assert_eq!(u.get("PCI_SLOT_NAME").map(String::as_str), Some("0000:03:00.0"));
        assert_eq!(parse_pci_id("0x1002\n"), Some(0x1002));
        assert_eq!(parse_pci_id("73bf"), Some(0x73bf));
        assert_eq!(parse_pci_id("nope"), None);
    }

    const PCI_IDS: &str = "# a comment\n\
1002  Advanced Micro Devices, Inc. [AMD/ATI]\n\
\t73bf  Navi 21 [Radeon RX 6800/6800 XT / 6900 XT]\n\
\t\t1002 0e3a  Subsystem name\n\
\t1638  Cezanne [Radeon Vega Series / Radeon Vega Mobile Series]\n\
8086  Intel Corporation\n\
\t4680  AlderLake-S GT1 [UHD Graphics 770]\n\
\t56a0  DG2 [Arc A770]\n\
10de  NVIDIA Corporation\n\
\t2484  GA104 [GeForce RTX 3070]\n\
\t1234  Plain Name\n";

    #[test]
    fn the_id_database_gives_a_device_its_name() {
        assert_eq!(pci_ids_device(PCI_IDS, 0x1002, 0x73bf).as_deref(), Some("Navi 21 [Radeon RX 6800/6800 XT / 6900 XT]"));
        assert_eq!(pci_ids_device(PCI_IDS, 0x8086, 0x56a0).as_deref(), Some("DG2 [Arc A770]"));
        assert_eq!(pci_ids_device(PCI_IDS, 0x10de, 0x1234).as_deref(), Some("Plain Name"));
        // A device another vendor has is not this vendor's, and an unknown one is nothing.
        assert_eq!(pci_ids_device(PCI_IDS, 0x8086, 0x73bf), None);
        assert_eq!(pci_ids_device(PCI_IDS, 0x1002, 0xffff), None);
        assert_eq!(pci_ids_device(PCI_IDS, 0x15ad, 0x0405), None);
    }

    #[test]
    fn a_name_is_the_marketing_one_with_its_maker() {
        let name = |v, d| display_name(v, d, pci_ids_device(PCI_IDS, v, d).as_deref());
        assert_eq!(name(0x1002, 0x73bf), "AMD Radeon RX 6800/6800 XT / 6900 XT");
        assert_eq!(name(0x8086, 0x4680), "Intel UHD Graphics 770");
        assert_eq!(name(0x8086, 0x56a0), "Intel Arc A770");
        assert_eq!(name(0x10de, 0x2484), "NVIDIA GeForce RTX 3070");
        assert_eq!(name(0x10de, 0x1234), "NVIDIA Plain Name");
        assert_eq!(display_name(0x1002, 0x73bf, None), "AMD GPU 1002:73bf", "no database: the ids");
    }

    #[test]
    fn the_cpus_own_graphics_are_integrated_a_card_is_discrete() {
        assert_eq!(kind_of("Intel UHD Graphics 770", None), GpuAdapterKind::Integrated);
        assert_eq!(kind_of("Intel Arc A770", Some(16 << 30)), GpuAdapterKind::Discrete);
        assert_eq!(kind_of("Intel Arc A770", None), GpuAdapterKind::Discrete, "an Arc names itself");
        assert_eq!(kind_of("Intel Iris Xe", None), GpuAdapterKind::Integrated);
        assert_eq!(kind_of("AMD Radeon RX 6800/6800 XT / 6900 XT", Some(16 << 30)), GpuAdapterKind::Discrete);
        assert_eq!(kind_of("AMD Radeon Vega Series / Radeon Vega Mobile Series", Some(512 << 20)), GpuAdapterKind::Integrated);
        assert_eq!(kind_of("AMD Radeon Graphics", Some(2 << 30)), GpuAdapterKind::Integrated);
        assert_eq!(kind_of("NVIDIA GeForce RTX 3070", Some(8 << 30)), GpuAdapterKind::Discrete);
    }

    fn client(pid: u32, fd: u32, id: Option<u64>, busy: u64, vram: u64, gtt: u64) -> Scanned {
        Scanned {
            pid,
            fd,
            client: DrmClient {
                driver: "amdgpu".into(),
                pdev: Some("0000:03:00.0".into()),
                id,
                engines: vec![("gfx".into(), busy)],
                dedicated: Some(vram),
                shared: Some(gtt),
            },
        }
    }

    #[test]
    fn the_first_read_has_no_use_and_the_second_has_the_time_the_engines_were_busy() {
        let first = [client(10, 5, Some(1), 1_000_000_000, 100 << 20, 5 << 20)];
        let (figs, next) = process_figures(&first, &HashMap::new(), None);
        assert_eq!(figs, [ProcessFigures { pid: 10, use_pct: None, dedicated: Some(100 << 20), shared: Some(5 << 20) }]);
        // Two seconds later it was busy for half a second of the two: a quarter.
        let second = [client(10, 5, Some(1), 1_500_000_000, 100 << 20, 5 << 20)];
        let (figs, _) = process_figures(&second, &next, Some(Duration::from_secs(2)));
        assert_eq!(figs[0].use_pct, Some(25.0));
    }

    #[test]
    fn two_descriptors_on_one_client_are_one_client() {
        let scanned = [client(10, 5, Some(1), 100, 64 << 20, 0), client(10, 9, Some(1), 100, 64 << 20, 0)];
        let (figs, next) = process_figures(&scanned, &HashMap::new(), None);
        assert_eq!(next.len(), 1);
        assert_eq!(figs[0].dedicated, Some(64 << 20), "not 128 MiB");
        assert_eq!(held_on(&scanned, "0000:03:00.0").0, Some(64 << 20));
    }

    #[test]
    fn two_clients_of_one_process_add_up_within_a_whole() {
        let prev: HashMap<ClientKey, u64> = [((10, "0000:03:00.0".to_string(), 1), 0), ((10, "0000:03:00.0".to_string(), 2), 0)].into();
        let scanned = [client(10, 5, Some(1), 900_000_000, 1 << 20, 0), client(10, 6, Some(2), 800_000_000, 2 << 20, 0)];
        let (figs, _) = process_figures(&scanned, &prev, Some(Duration::from_secs(1)));
        assert_eq!(figs[0].use_pct, Some(100.0), "90% and 80% are not 170% of one GPU");
        assert_eq!(figs[0].dedicated, Some(3 << 20));
    }

    #[test]
    fn a_new_client_or_a_reads_too_close_together_leaves_use_unknown() {
        let prev: HashMap<ClientKey, u64> = [((10, "0000:03:00.0".to_string(), 1), 0)].into();
        let scanned = [client(10, 5, Some(1), 500, 0, 0), client(11, 5, Some(7), 500, 0, 0)];
        let (figs, _) = process_figures(&scanned, &prev, Some(Duration::from_secs(1)));
        assert!(figs.iter().find(|p| p.pid == 10).unwrap().use_pct.is_some());
        assert_eq!(figs.iter().find(|p| p.pid == 11).unwrap().use_pct, None, "no earlier figure for this client");
        let (figs, _) = process_figures(&scanned, &prev, Some(Duration::from_millis(10)));
        assert!(figs.iter().all(|p| p.use_pct.is_none()), "10 ms between reads says nothing");
    }

    #[test]
    fn a_counter_that_went_backwards_is_not_a_negative_use() {
        let prev: HashMap<ClientKey, u64> = [((10, "0000:03:00.0".to_string(), 1), 9_000)].into();
        let (figs, _) = process_figures(&[client(10, 5, Some(1), 100, 0, 0)], &prev, Some(Duration::from_secs(1)));
        assert_eq!(figs[0].use_pct, Some(0.0));
    }

    #[test]
    fn what_the_clients_hold_on_one_gpu_is_its_memory_where_it_can_say_none() {
        let mut other = client(11, 5, Some(3), 0, 0, 7 << 20);
        other.client.pdev = Some("0000:00:02.0".into());
        let scanned = [client(10, 5, Some(1), 0, 0, 30 << 20), other];
        assert_eq!(held_on(&scanned, "0000:03:00.0"), (Some(0), Some(30 << 20)));
        assert_eq!(held_on(&scanned, "0000:00:02.0").1, Some(7 << 20));
        assert_eq!(held_on(&scanned, "0000:ff:00.0"), (None, None));
    }
}
