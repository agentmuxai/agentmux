// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The GPU's use and memory on Linux (SPEC_STATUS_BAR_GPU_PANEL_2026_11_15.md §3, phase 2), from what the kernel and the
//! driver already publish, with nothing installed:
//!
//! - **AMD** (amdgpu): the card's own sysfs files, `gpu_busy_percent`, `mem_info_vram_*` and `mem_info_gtt_*`.
//! - **NVIDIA:** NVML, loaded at run time ([`super::nvml`]); without it the card is listed with no figures.
//! - **Intel** (i915, xe): memory only. Nothing cheap says how busy the GPU is without a privileged perf counter, and
//!   the memory is the machine's, so the figure is what the processes' own `fdinfo` add up to.
//! - **Per process:** `/proc/<pid>/fdinfo/<fd>` of every descriptor that is a DRM device (kernel 5.14 and later), for the
//!   processes this one can read; NVML's process lists on NVIDIA. A process's use is how long its engines were busy
//!   since the last read, so the first read of the panel has none ([`Read::Warming`]).
//!
//! **A sleeping GPU is left asleep.** A laptop's second GPU is powered down by the kernel when nothing uses it, and any
//! query of it (a sysfs read of amdgpu, any NVML call) wakes it for good, which costs the battery. A PCI device whose
//! `power/runtime_status` is `suspended` is reported idle without being read.
//!
//! The paths are a [`Fs`], so the tests give it a directory built like `/sys` and `/proc`.

use super::aggregate::{ProcessFigures, Reading};
use super::drm::{self, ClientKey, Scanned};
use super::nvml::{self, Nvml};
use crate::backend::rpc_types::GpuAdapter;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How long one scan of the processes' descriptors may take before it stops and gives what it has: a machine with
/// thousands of processes costs more, and the sampler widens its interval when a read is slow.
const SCAN_BUDGET: Duration = Duration::from_millis(300);
/// NVML is looked for again this often when it was not there (a driver installed since).
const NVML_RETRY: Duration = Duration::from_secs(300);

/// Where the files are, so a test can point them at a directory of its own.
#[derive(Debug, Clone)]
pub struct Fs {
    pub sys: PathBuf,
    pub proc: PathBuf,
    /// The PCI id databases, the first that exists gives the names.
    pub pci_ids: Vec<PathBuf>,
}

impl Default for Fs {
    fn default() -> Self {
        Self {
            sys: PathBuf::from("/sys"),
            proc: PathBuf::from("/proc"),
            pci_ids: ["/usr/share/hwdata/pci.ids", "/usr/share/misc/pci.ids", "/usr/share/pci.ids", "/var/lib/pciutils/pci.ids"]
                .iter()
                .map(PathBuf::from)
                .collect(),
        }
    }
}

impl Fs {
    fn pci_dir(&self, pdev: &str) -> PathBuf {
        self.sys.join("bus/pci/devices").join(pdev)
    }

    /// Whether the kernel has the device powered down (module doc: it is not to be touched).
    pub fn asleep(&self, pdev: &str) -> bool {
        read_trimmed(&self.pci_dir(pdev).join("power/runtime_status")).is_some_and(|s| s == "suspended")
    }
}

fn read_trimmed(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

fn read_number(path: &Path) -> Option<u64> {
    read_trimmed(path)?.parse().ok()
}

/// One display adapter on the PCI bus.
#[derive(Debug, Clone)]
struct Card {
    pdev: String,
    vendor: u32,
    name: String,
}

/// What a read gave.
pub enum Read {
    /// Figures that need two reads to mean anything (a process's use) have only had one: ask again.
    Warming,
    Reading(Reading),
}

/// The Linux counters: the adapters found, NVML if it is there, and what the last read of the processes saw.
pub struct Gpu {
    fs: Fs,
    cards: Vec<Card>,
    nvml: Option<Nvml>,
    nvml_retry_at: Option<Instant>,
    prev: HashMap<ClientKey, u64>,
    prev_at: Option<Instant>,
}

impl Gpu {
    /// Finds the adapters. `Err` when this computer has no display adapter from a maker whose counters are read.
    pub fn open() -> Result<Self, String> {
        Self::open_with(Fs::default())
    }

    pub fn open_with(fs: Fs) -> Result<Self, String> {
        let cards = discover(&fs);
        if cards.is_empty() {
            return Err("no AMD, Intel or NVIDIA display adapter".to_string());
        }
        Ok(Self { fs, cards, nvml: None, nvml_retry_at: None, prev: HashMap::new(), prev_at: None })
    }

    pub fn read(&mut self, full: bool) -> Result<Read, String> {
        let nvml_devices = self.read_nvml(full);
        let scanned = if full && self.cards.iter().any(|c| c.vendor != drm::VENDOR_NVIDIA) { self.scan_clients() } else { Vec::new() };

        let mut adapters: Vec<GpuAdapter> = Vec::new();
        for card in &self.cards {
            adapters.push(match card.vendor {
                drm::VENDOR_NVIDIA => nvidia_adapter(card, nvml_devices.iter().find(|d| d.pdev == card.pdev)),
                drm::VENDOR_AMD => self.amd_adapter(card),
                _ => intel_adapter(card, &scanned),
            });
        }

        let now = Instant::now();
        let elapsed = self.prev_at.map(|at| now.duration_since(at));
        let warming = full && self.prev_at.is_none() && scanned.iter().any(|s| !s.client.engines.is_empty());
        let mut processes: Vec<ProcessFigures> = Vec::new();
        if full {
            let (figures, next) = drm::process_figures(&scanned, &self.prev, elapsed);
            self.prev = next;
            self.prev_at = Some(now);
            processes = figures;
            for d in &nvml_devices {
                for p in &d.processes {
                    merge(&mut processes, ProcessFigures { pid: p.pid, use_pct: p.use_pct, dedicated: p.memory, shared: None });
                }
            }
        } else {
            // The processes are not read while the panel is closed, so what was kept is going stale: the first read
            // after it is opened again is a first read, not a use over however long it was closed.
            self.prev.clear();
            self.prev_at = None;
        }
        if warming {
            return Ok(Read::Warming);
        }
        Ok(Read::Reading(Reading { adapters, processes, per_process_memory: full }))
    }

    /// NVML's view of the NVIDIA devices, opening it the first time one is wanted and awake.
    fn read_nvml(&mut self, full: bool) -> Vec<nvml::Device> {
        let nvidia: Vec<&Card> = self.cards.iter().filter(|c| c.vendor == drm::VENDOR_NVIDIA).collect();
        if nvidia.is_empty() {
            return Vec::new();
        }
        if self.nvml.is_none() && self.nvml_retry_at.is_none_or(|t| Instant::now() >= t) {
            // Opening NVML touches the devices: not while every one of them is asleep.
            if nvidia.iter().all(|c| self.fs.asleep(&c.pdev)) {
                return Vec::new();
            }
            match Nvml::load() {
                Ok(n) => self.nvml = Some(n),
                Err(why) => {
                    tracing::debug!(error = %why, "gpu: no NVML");
                    self.nvml_retry_at = Some(Instant::now() + NVML_RETRY);
                }
            }
        }
        let fs = self.fs.clone();
        match self.nvml.as_mut() {
            Some(n) => n.read(full, &|slot| fs.asleep(slot)),
            None => Vec::new(),
        }
    }

    fn amd_adapter(&self, card: &Card) -> GpuAdapter {
        let dir = self.fs.pci_dir(&card.pdev);
        let mut a = blank(card);
        if self.fs.asleep(&card.pdev) {
            // Asleep means idle (module doc); its memory is not asked.
            a.use_pct = Some(0.0);
            return a;
        }
        a.use_pct = read_number(&dir.join("gpu_busy_percent")).map(|p| (p as f64).min(100.0));
        a.dedicated_used = read_number(&dir.join("mem_info_vram_used"));
        a.dedicated_total = read_number(&dir.join("mem_info_vram_total"));
        a.shared_used = read_number(&dir.join("mem_info_gtt_used"));
        a.shared_total = read_number(&dir.join("mem_info_gtt_total"));
        a.kind = drm::kind_of(&card.name, a.dedicated_total);
        a
    }

    /// Every DRM descriptor of every process this one can read.
    fn scan_clients(&self) -> Vec<Scanned> {
        let deadline = Instant::now() + SCAN_BUDGET;
        let mut out = Vec::new();
        let Ok(procs) = std::fs::read_dir(&self.fs.proc) else { return out };
        for entry in procs.flatten() {
            if Instant::now() >= deadline {
                tracing::debug!("gpu: the scan of the processes ran out of time");
                break;
            }
            let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
            // A process of another user has an fd directory this one cannot list: it is simply not seen.
            let Ok(fds) = std::fs::read_dir(entry.path().join("fd")) else { continue };
            for fd in fds.flatten() {
                let Some(n) = fd.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
                let Ok(target) = std::fs::read_link(fd.path()) else { continue };
                if !target.starts_with("/dev/dri/") {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(entry.path().join("fdinfo").join(n.to_string())) else { continue };
                if let Some(client) = drm::parse_fdinfo(&text) {
                    out.push(Scanned { pid, fd: n, client });
                }
            }
        }
        out
    }
}

/// A process's figures added to the list: one that is there already (on two GPUs) keeps its busiest use and adds its memory.
fn merge(list: &mut Vec<ProcessFigures>, p: ProcessFigures) {
    let add = |a: Option<u64>, b: Option<u64>| match (a, b) {
        (Some(a), Some(b)) => Some(a.saturating_add(b)),
        (a, b) => a.or(b),
    };
    if let Some(have) = list.iter_mut().find(|q| q.pid == p.pid) {
        have.use_pct = match (have.use_pct, p.use_pct) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        };
        have.dedicated = add(have.dedicated, p.dedicated);
        have.shared = add(have.shared, p.shared);
    } else {
        list.push(p);
    }
}

fn blank(card: &Card) -> GpuAdapter {
    GpuAdapter {
        luid: card.pdev.clone(),
        name: card.name.clone(),
        kind: drm::kind_of(&card.name, None),
        use_pct: None,
        dedicated_used: None,
        dedicated_total: None,
        shared_used: None,
        shared_total: None,
    }
}

fn nvidia_adapter(card: &Card, device: Option<&nvml::Device>) -> GpuAdapter {
    let mut a = blank(card);
    let Some(d) = device else { return a };
    if !d.name.is_empty() {
        a.name = d.name.clone();
    }
    a.use_pct = d.use_pct;
    a.dedicated_used = d.memory_used;
    a.dedicated_total = d.memory_total;
    a.kind = drm::kind_of(&a.name, a.dedicated_total);
    a
}

/// Intel's adapter has no figure of its own: its memory is what the processes were seen to hold.
fn intel_adapter(card: &Card, scanned: &[Scanned]) -> GpuAdapter {
    let mut a = blank(card);
    let (dedicated, shared) = drm::held_on(scanned, &card.pdev);
    a.dedicated_used = dedicated;
    a.shared_used = shared;
    a
}

/// The display adapters on the PCI bus from the makers whose counters are read, in slot order.
fn discover(fs: &Fs) -> Vec<Card> {
    let Ok(entries) = std::fs::read_dir(fs.sys.join("bus/pci/devices")) else { return Vec::new() };
    let mut found: Vec<(String, u32, u32)> = Vec::new();
    for e in entries.flatten() {
        let dir = e.path();
        // PCI class 03: display controllers (VGA, 3D, other).
        let class = read_trimmed(&dir.join("class")).and_then(|c| drm::parse_pci_id(&c));
        if !class.is_some_and(|c| c >> 16 == 0x03) {
            continue;
        }
        let vendor = read_trimmed(&dir.join("vendor")).and_then(|v| drm::parse_pci_id(&v));
        let device = read_trimmed(&dir.join("device")).and_then(|v| drm::parse_pci_id(&v)).unwrap_or(0);
        let Some(vendor) = vendor.filter(|v| matches!(*v, drm::VENDOR_AMD | drm::VENDOR_INTEL | drm::VENDOR_NVIDIA)) else { continue };
        found.push((e.file_name().to_string_lossy().to_ascii_lowercase(), vendor, device));
    }
    if found.is_empty() {
        return Vec::new();
    }
    found.sort();
    let db = fs.pci_ids.iter().find_map(|p| std::fs::read_to_string(p).ok());
    found
        .into_iter()
        .map(|(pdev, vendor, device)| {
            let from_db = db.as_deref().and_then(|d| drm::pci_ids_device(d, vendor, device));
            Card { pdev, vendor, name: drm::display_name(vendor, device, from_db.as_deref()) }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::rpc_types::GpuAdapterKind;
    use std::os::unix::fs::symlink;

    /// A directory built like `/sys` and `/proc` with what a test puts in it.
    struct Root(tempfile::TempDir);

    impl Root {
        fn new() -> Self {
            let r = Root(tempfile::tempdir().unwrap());
            std::fs::create_dir_all(r.0.path().join("sys/bus/pci/devices")).unwrap();
            std::fs::create_dir_all(r.0.path().join("proc")).unwrap();
            std::fs::create_dir_all(r.0.path().join("drivers")).unwrap();
            r
        }
        fn fs(&self) -> Fs {
            Fs { sys: self.0.path().join("sys"), proc: self.0.path().join("proc"), pci_ids: vec![self.0.path().join("pci.ids")] }
        }
        fn write(&self, rel: &str, text: &str) {
            let p = self.0.path().join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
        /// An adapter at `pdev` with the files every PCI device has.
        fn card(&self, pdev: &str, vendor: &str, device: &str, driver: &str) {
            let d = format!("sys/bus/pci/devices/{pdev}");
            self.write(&format!("{d}/class"), "0x030000\n");
            self.write(&format!("{d}/vendor"), &format!("{vendor}\n"));
            self.write(&format!("{d}/device"), &format!("{device}\n"));
            std::fs::create_dir_all(self.0.path().join("drivers").join(driver)).unwrap();
            symlink(self.0.path().join("drivers").join(driver), self.0.path().join(&d).join("driver")).unwrap();
        }
        /// A process holding a DRM descriptor with this fdinfo.
        fn client(&self, pid: u32, fd: u32, fdinfo: &str) {
            let fd_dir = self.0.path().join(format!("proc/{pid}/fd"));
            std::fs::create_dir_all(&fd_dir).unwrap();
            // A client written again (its figures moved on) keeps its descriptor.
            let _ = symlink("/dev/dri/card0", fd_dir.join(fd.to_string()));
            self.write(&format!("proc/{pid}/fdinfo/{fd}"), fdinfo);
        }
    }

    const PCI_IDS: &str = "1002  Advanced Micro Devices, Inc. [AMD/ATI]\n\
\t73bf  Navi 21 [Radeon RX 6800/6800 XT / 6900 XT]\n\
8086  Intel Corporation\n\
\t4680  AlderLake-S GT1 [UHD Graphics 770]\n\
10de  NVIDIA Corporation\n\
\t2484  GA104 [GeForce RTX 3070]\n";

    fn amd_root() -> Root {
        let r = Root::new();
        r.write("pci.ids", PCI_IDS);
        r.card("0000:03:00.0", "0x1002", "0x73bf", "amdgpu");
        let d = "sys/bus/pci/devices/0000:03:00.0";
        r.write(&format!("{d}/gpu_busy_percent"), "37\n");
        r.write(&format!("{d}/mem_info_vram_used"), &format!("{}\n", 3u64 << 30));
        r.write(&format!("{d}/mem_info_vram_total"), &format!("{}\n", 16u64 << 30));
        r.write(&format!("{d}/mem_info_gtt_used"), &format!("{}\n", 1u64 << 30));
        r.write(&format!("{d}/mem_info_gtt_total"), &format!("{}\n", 32u64 << 30));
        r
    }

    fn reading(r: Read) -> Reading {
        match r {
            Read::Reading(r) => r,
            Read::Warming => panic!("expected a reading, got warming"),
        }
    }

    #[test]
    fn an_amd_card_is_read_from_its_own_files_and_named_from_the_id_database() {
        let r = amd_root();
        let mut gpu = Gpu::open_with(r.fs()).unwrap();
        let reading = reading(gpu.read(false).unwrap());
        assert_eq!(reading.adapters.len(), 1);
        let a = &reading.adapters[0];
        assert_eq!(a.name, "AMD Radeon RX 6800/6800 XT / 6900 XT");
        assert_eq!(a.luid, "0000:03:00.0");
        assert_eq!((a.use_pct, a.dedicated_used, a.dedicated_total), (Some(37.0), Some(3 << 30), Some(16 << 30)));
        assert_eq!((a.shared_used, a.shared_total), (Some(1 << 30), Some(32 << 30)));
        assert_eq!(a.kind, GpuAdapterKind::Discrete);
        assert!(reading.processes.is_empty() && !reading.per_process_memory, "the status bar's read leaves the processes alone");
    }

    #[test]
    fn a_card_the_kernel_has_powered_down_is_idle_and_not_read() {
        let r = amd_root();
        r.write("sys/bus/pci/devices/0000:03:00.0/power/runtime_status", "suspended\n");
        // Were its files read, this one would be taken for a busy card.
        r.write("sys/bus/pci/devices/0000:03:00.0/gpu_busy_percent", "99\n");
        let mut gpu = Gpu::open_with(r.fs()).unwrap();
        let a = &reading(gpu.read(false).unwrap()).adapters[0];
        assert_eq!((a.use_pct, a.dedicated_used, a.dedicated_total), (Some(0.0), None, None));
        r.write("sys/bus/pci/devices/0000:03:00.0/power/runtime_status", "active\n");
        let a = &reading(gpu.read(false).unwrap()).adapters[0];
        assert_eq!(a.use_pct, Some(99.0), "awake again: read");
    }

    #[test]
    fn the_first_read_of_the_processes_is_warming_and_the_second_has_their_use() {
        let r = amd_root();
        let info = |busy: u64| {
            format!(
                "drm-driver:\tamdgpu\ndrm-pdev:\t0000:03:00.0\ndrm-client-id:\t7\ndrm-engine-gfx:\t{busy} ns\ndrm-memory-vram:\t{} KiB\ndrm-memory-gtt:\t0 KiB\n",
                200 * 1024
            )
        };
        r.client(4242, 9, &info(1_000_000_000));
        let mut gpu = Gpu::open_with(r.fs()).unwrap();
        assert!(matches!(gpu.read(true).unwrap(), Read::Warming));
        // The engine ran for 0.3 s more, over at least the 0.3 s that passed: some use, never over the whole. The exact
        // figure depends on how long a loaded machine took to wake, so only its bounds are asserted (the arithmetic is
        // pinned with fixed intervals in drm.rs).
        std::thread::sleep(Duration::from_millis(300));
        r.client(4242, 9, &info(1_300_000_000));
        let reading = reading(gpu.read(true).unwrap());
        assert!(reading.per_process_memory);
        assert_eq!(reading.processes.len(), 1);
        let p = &reading.processes[0];
        assert_eq!((p.pid, p.dedicated), (4242, Some(200 << 20)));
        let use_pct = p.use_pct.expect("two reads give a use");
        assert!(use_pct > 0.0 && use_pct <= 100.0, "{use_pct}");
    }

    #[test]
    fn a_panel_opened_again_after_a_long_time_starts_over_instead_of_using_a_stale_read() {
        let r = amd_root();
        let info = |busy: u64| {
            format!("drm-driver:\tamdgpu\ndrm-pdev:\t0000:03:00.0\ndrm-client-id:\t7\ndrm-engine-gfx:\t{busy} ns\ndrm-memory-vram:\t1024 KiB\ndrm-memory-gtt:\t0 KiB\n")
        };
        r.client(4242, 9, &info(1_000_000_000));
        let mut gpu = Gpu::open_with(r.fs()).unwrap();
        assert!(matches!(gpu.read(true).unwrap(), Read::Warming));
        // The panel is closed: only the status bar's reads come, and they leave the processes alone.
        r.client(4242, 9, &info(900_000_000_000));
        let bar = reading(gpu.read(false).unwrap());
        assert!(!bar.per_process_memory && bar.processes.is_empty());
        // Opened again: the baseline from before is not compared against, so this is a first read again.
        assert!(matches!(gpu.read(true).unwrap(), Read::Warming));
        std::thread::sleep(Duration::from_millis(300));
        r.client(4242, 9, &info(900_300_000_000));
        let reading = reading(gpu.read(true).unwrap());
        let use_pct = reading.processes[0].use_pct.expect("two reads give a use");
        assert!(use_pct > 0.0 && use_pct <= 100.0, "{use_pct}");
    }

    #[test]
    fn a_computer_with_no_clients_is_not_warming() {
        let r = amd_root();
        let mut gpu = Gpu::open_with(r.fs()).unwrap();
        let reading = reading(gpu.read(true).unwrap());
        assert!(reading.processes.is_empty() && reading.per_process_memory);
    }

    #[test]
    fn an_intel_card_has_memory_only_made_of_what_its_clients_hold() {
        let r = Root::new();
        r.write("pci.ids", PCI_IDS);
        r.card("0000:00:02.0", "0x8086", "0x4680", "i915");
        let info = "drm-driver:\ti915\ndrm-pdev:\t0000:00:02.0\ndrm-client-id:\t3\ndrm-engine-render:\t5 ns\ndrm-resident-system0:\t20 MiB\n";
        r.client(10, 4, info);
        r.client(11, 5, &info.replace("client-id:\t3", "client-id:\t4"));
        let mut gpu = Gpu::open_with(r.fs()).unwrap();
        let first = gpu.read(true).unwrap();
        assert!(matches!(first, Read::Warming));
        let reading = reading(gpu.read(true).unwrap());
        let a = &reading.adapters[0];
        assert_eq!(a.name, "Intel UHD Graphics 770");
        assert_eq!(a.kind, GpuAdapterKind::Integrated);
        assert_eq!((a.use_pct, a.dedicated_total, a.shared_total), (None, None, None), "no figure of use, and no limit to be a share of");
        assert_eq!(a.shared_used, Some(40 << 20));
    }

    #[test]
    fn an_nvidia_card_without_the_library_is_listed_with_no_figures() {
        let r = Root::new();
        r.write("pci.ids", PCI_IDS);
        r.card("0000:01:00.0", "0x10de", "0x2484", "nvidia");
        let mut gpu = Gpu::open_with(r.fs()).unwrap();
        // This test machine may have NVIDIA's library; the card is only ever read through it, so either way no panic and
        // the row is there, with the name the id database gives it.
        let reading = reading(gpu.read(true).unwrap());
        assert_eq!(reading.adapters.len(), 1);
        assert_eq!(reading.adapters[0].name, "NVIDIA GeForce RTX 3070");
    }

    #[test]
    fn a_hybrid_laptop_lists_both_adapters_in_slot_order() {
        let r = amd_root();
        r.card("0000:00:02.0", "0x8086", "0x4680", "i915");
        let mut gpu = Gpu::open_with(r.fs()).unwrap();
        let names: Vec<String> = reading(gpu.read(false).unwrap()).adapters.into_iter().map(|a| a.name).collect();
        assert_eq!(names, ["Intel UHD Graphics 770", "AMD Radeon RX 6800/6800 XT / 6900 XT"]);
    }

    #[test]
    fn a_computer_with_no_supported_display_adapter_has_nothing_to_open() {
        let r = Root::new();
        // A virtual machine's adapter (VMware's SVGA) is nothing whose counters are read.
        r.card("0000:00:0f.0", "0x15ad", "0x0405", "vmwgfx");
        assert!(Gpu::open_with(r.fs()).is_err());
        // Neither is a PCI device that is not a display controller.
        let r = Root::new();
        r.write("sys/bus/pci/devices/0000:00:1f.3/class", "0x040300\n");
        r.write("sys/bus/pci/devices/0000:00:1f.3/vendor", "0x8086\n");
        assert!(Gpu::open_with(r.fs()).is_err());
    }

    #[test]
    fn a_process_this_one_cannot_list_and_files_that_are_not_drm_are_skipped() {
        let r = amd_root();
        // A descriptor that is not a DRM device, one whose fdinfo has no DRM lines, and a stray name in /proc.
        let fd_dir = r.0.path().join("proc/50/fd");
        std::fs::create_dir_all(&fd_dir).unwrap();
        symlink("/dev/null", fd_dir.join("3")).unwrap();
        r.write("proc/50/fdinfo/3", "pos:\t0\n");
        r.write("proc/self/note", "not a process");
        std::fs::create_dir_all(r.0.path().join("proc/60")).unwrap(); // no fd directory
        let mut gpu = Gpu::open_with(r.fs()).unwrap();
        assert!(reading(gpu.read(true).unwrap()).processes.is_empty());
    }

    #[test]
    fn the_same_process_on_two_gpus_keeps_its_busiest_use_and_adds_its_memory() {
        let mut list = vec![ProcessFigures { pid: 5, use_pct: Some(10.0), dedicated: Some(100), shared: None }];
        merge(&mut list, ProcessFigures { pid: 5, use_pct: Some(40.0), dedicated: Some(50), shared: Some(7) });
        merge(&mut list, ProcessFigures { pid: 6, use_pct: None, dedicated: None, shared: None });
        assert_eq!(list.len(), 2);
        assert_eq!(list[0], ProcessFigures { pid: 5, use_pct: Some(40.0), dedicated: Some(150), shared: Some(7) });
    }

    #[test]
    fn a_read_of_this_machine_does_not_panic_whatever_it_has() {
        // On a build machine with no GPU (or a virtual one) there is nothing to open; with one, a read must work.
        if let Ok(mut gpu) = Gpu::open() {
            let _ = gpu.read(false).unwrap();
            let _ = gpu.read(true).unwrap();
        }
    }

    #[test]
    fn an_amd_apu_with_a_small_carve_out_is_integrated() {
        let r = Root::new();
        r.write("pci.ids", "1002  AMD\n\t1638  Cezanne [Radeon Vega Series / Radeon Vega Mobile Series]\n");
        r.card("0000:05:00.0", "0x1002", "0x1638", "amdgpu");
        r.write("sys/bus/pci/devices/0000:05:00.0/mem_info_vram_total", &format!("{}\n", 512u64 << 20));
        r.write("sys/bus/pci/devices/0000:05:00.0/mem_info_gtt_total", &format!("{}\n", 8u64 << 30));
        r.write("sys/bus/pci/devices/0000:05:00.0/mem_info_gtt_used", &format!("{}\n", 1u64 << 30));
        let mut gpu = Gpu::open_with(r.fs()).unwrap();
        let a = reading(gpu.read(false).unwrap()).adapters.remove(0);
        assert_eq!(a.kind, GpuAdapterKind::Integrated);
        assert_eq!(a.use_pct, None, "no busy file: unknown, never zero");
        assert_eq!(super::super::aggregate::memory_share(&a), Some(12.5), "of the machine's memory it may use");
    }
}
