// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! NVIDIA's management library (NVML), loaded when it is needed (SPEC_STATUS_BAR_GPU_PANEL_2026_11_15.md §3).
//!
//! `libnvidia-ml.so.1` comes with NVIDIA's driver. AgentMux does not link to it: a computer without it (no NVIDIA card, or
//! the open driver) loads nothing, and [`Nvml::load`] says so as an error the sampler treats as "NVIDIA tools not found",
//! never as a failure. Only the handful of functions the panel needs are looked up, and one that an older driver lacks
//! (the process lists, which grew a version) is left out, not required.
//!
//! The calls are plain C with out-parameters; everything unsafe is in this file, behind [`Nvml::read`].

use libloading::Library;
use std::ffi::{c_char, c_int, c_uint, c_ulonglong, c_void, CStr, OsStr};

type Handle = *mut c_void;

const SUCCESS: c_int = 0;
const NOT_FOUND: c_int = 6;
const INSUFFICIENT_SIZE: c_int = 7;
/// What a driver puts in a figure it cannot give.
const VALUE_NOT_AVAILABLE: u64 = u64::MAX;
/// The library's name where the driver puts it; the version-less `.so` is a development link that is often absent.
const LIBRARY: &str = "libnvidia-ml.so.1";

#[repr(C)]
#[derive(Default)]
struct Utilization {
    gpu: c_uint,
    memory: c_uint,
}

#[repr(C)]
#[derive(Default)]
struct Memory {
    total: c_ulonglong,
    free: c_ulonglong,
    used: c_ulonglong,
}

#[repr(C)]
struct PciInfo {
    bus_id_legacy: [c_char; 16],
    domain: c_uint,
    bus: c_uint,
    device: c_uint,
    pci_device_id: c_uint,
    pci_sub_system_id: c_uint,
    bus_id: [c_char; 32],
}

/// `nvmlProcessInfo_t` as the `_v2` and `_v3` calls fill it.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct ProcessInfo {
    pid: c_uint,
    used_gpu_memory: c_ulonglong,
    gpu_instance_id: c_uint,
    compute_instance_id: c_uint,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct ProcessUtilizationSample {
    pid: c_uint,
    time_stamp: c_ulonglong,
    sm_util: c_uint,
    mem_util: c_uint,
    enc_util: c_uint,
    dec_util: c_uint,
}

type ProcessList = unsafe extern "C" fn(Handle, *mut c_uint, *mut ProcessInfo) -> c_int;
type ProcessUtilization = unsafe extern "C" fn(Handle, *mut ProcessUtilizationSample, *mut c_uint, c_ulonglong) -> c_int;

struct Api {
    shutdown: unsafe extern "C" fn() -> c_int,
    count: unsafe extern "C" fn(*mut c_uint) -> c_int,
    handle: unsafe extern "C" fn(c_uint, *mut Handle) -> c_int,
    name: unsafe extern "C" fn(Handle, *mut c_char, c_uint) -> c_int,
    rates: unsafe extern "C" fn(Handle, *mut Utilization) -> c_int,
    memory: unsafe extern "C" fn(Handle, *mut Memory) -> c_int,
    pci: unsafe extern "C" fn(Handle, *mut PciInfo) -> c_int,
    compute: Option<ProcessList>,
    graphics: Option<ProcessList>,
    process_utilization: Option<ProcessUtilization>,
    /// Keeps the library mapped as long as the function pointers above are in use.
    _lib: Library,
}

/// One device, as it was read.
#[derive(Debug, Clone, PartialEq)]
pub struct Device {
    /// Its PCI slot, in the spelling the kernel uses (`0000:01:00.0`): how it is matched to the card sysfs lists.
    pub pdev: String,
    pub name: String,
    /// `None` for a device that was asleep and left alone (reading it would have woken it), or one that said nothing.
    pub use_pct: Option<f64>,
    pub memory_total: Option<u64>,
    pub memory_used: Option<u64>,
    /// The processes that have the device open, with what they hold of its memory and how busy they keep it. Only read
    /// when the panel is open.
    pub processes: Vec<Process>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Process {
    pub pid: u32,
    pub memory: Option<u64>,
    pub use_pct: Option<f64>,
}

/// An open NVML. Dropping it shuts the library down.
pub struct Nvml {
    api: Api,
    /// Each device's PCI slot once it has been read awake (index in NVML's order).
    slots: Vec<Option<String>>,
    /// The newest sample time seen per device, so the next read asks only for what is new.
    last_seen: Vec<u64>,
}

impl Drop for Nvml {
    fn drop(&mut self) {
        // SAFETY: the library is loaded and initialised, and nothing else of ours holds a handle past this point.
        unsafe {
            (self.api.shutdown)();
        }
    }
}

// The function pointers are plain code addresses and the handles are not kept between calls; the sampler holds this
// behind its own lock, one read at a time.
unsafe impl Send for Nvml {}

/// `00000000:01:00.0` (NVML: an eight-digit domain) as the kernel spells a slot, `0000:01:00.0`.
pub fn normalise_bus_id(bus_id: &str) -> String {
    let lower = bus_id.trim().to_ascii_lowercase();
    match lower.split_once(':') {
        Some((domain, rest)) if domain.len() > 4 => format!("{}:{}", &domain[domain.len() - 4..], rest),
        _ => lower,
    }
}

fn text(buf: &[c_char]) -> String {
    // SAFETY: the buffers are zero-filled and the library writes a terminated string into them.
    let s = unsafe { CStr::from_ptr(buf.as_ptr()) };
    s.to_string_lossy().trim().to_string()
}

macro_rules! symbol {
    ($lib:expr, $name:literal) => {{
        // SAFETY: the signature is the one in NVML's header for this function.
        unsafe { $lib.get(concat!($name, "\0").as_bytes()).map(|s| *s) }
    }};
}

impl Nvml {
    /// Opens the library where the driver puts it. `Err` is "not there" (no NVIDIA driver) or "will not start".
    pub fn load() -> Result<Self, String> {
        Self::load_from(OsStr::new(LIBRARY))
    }

    pub fn load_from(path: &OsStr) -> Result<Self, String> {
        // SAFETY: loading a library runs its initialisers; it is NVIDIA's own, found by the loader's usual search.
        let lib = unsafe { Library::new(path) }.map_err(|e| format!("NVIDIA tools not found: {e}"))?;
        let init: unsafe extern "C" fn() -> c_int = symbol!(lib, "nvmlInit_v2").map_err(|e| format!("nvmlInit_v2: {e}"))?;
        let api = Api {
            shutdown: symbol!(lib, "nvmlShutdown").map_err(|e| format!("nvmlShutdown: {e}"))?,
            count: symbol!(lib, "nvmlDeviceGetCount_v2").map_err(|e| format!("nvmlDeviceGetCount_v2: {e}"))?,
            handle: symbol!(lib, "nvmlDeviceGetHandleByIndex_v2").map_err(|e| format!("nvmlDeviceGetHandleByIndex_v2: {e}"))?,
            name: symbol!(lib, "nvmlDeviceGetName").map_err(|e| format!("nvmlDeviceGetName: {e}"))?,
            rates: symbol!(lib, "nvmlDeviceGetUtilizationRates").map_err(|e| format!("nvmlDeviceGetUtilizationRates: {e}"))?,
            memory: symbol!(lib, "nvmlDeviceGetMemoryInfo").map_err(|e| format!("nvmlDeviceGetMemoryInfo: {e}"))?,
            pci: symbol!(lib, "nvmlDeviceGetPciInfo_v3").map_err(|e| format!("nvmlDeviceGetPciInfo_v3: {e}"))?,
            // The process lists: `_v3` (R510 and later), else `_v2`; a driver older than both has none.
            compute: symbol!(lib, "nvmlDeviceGetComputeRunningProcesses_v3")
                .or_else(|_: libloading::Error| symbol!(lib, "nvmlDeviceGetComputeRunningProcesses_v2"))
                .ok(),
            graphics: symbol!(lib, "nvmlDeviceGetGraphicsRunningProcesses_v3")
                .or_else(|_: libloading::Error| symbol!(lib, "nvmlDeviceGetGraphicsRunningProcesses_v2"))
                .ok(),
            process_utilization: symbol!(lib, "nvmlDeviceGetProcessUtilization").ok(),
            _lib: lib,
        };
        // SAFETY: the function is NVML's init, called once, before any other.
        let rc = unsafe { init() };
        if rc != SUCCESS {
            return Err(format!("nvmlInit_v2 returned {rc}"));
        }
        let mut n: c_uint = 0;
        // SAFETY: `n` is a valid out-parameter.
        let rc = unsafe { (api.count)(&mut n) };
        if rc != SUCCESS {
            // SAFETY: initialised above.
            unsafe { (api.shutdown)() };
            return Err(format!("nvmlDeviceGetCount_v2 returned {rc}"));
        }
        let n = n as usize;
        Ok(Self { api, slots: vec![None; n], last_seen: vec![0; n] })
    }

    /// How many devices NVML lists.
    pub fn devices(&self) -> usize {
        self.slots.len()
    }

    /// Reads every device. `asleep(slot)` says whether the kernel has that PCI device powered down: such a device is not
    /// asked anything (a query wakes it, and a laptop's second GPU is kept off to save the battery), so it is reported
    /// with no figures. A device is matched to its slot the first time it is read awake.
    pub fn read(&mut self, full: bool, asleep: &dyn Fn(&str) -> bool) -> Vec<Device> {
        let mut out = Vec::new();
        for index in 0..self.slots.len() {
            if let Some(slot) = self.slots[index].clone() {
                if asleep(&slot) {
                    out.push(Device { pdev: slot, name: String::new(), use_pct: None, memory_total: None, memory_used: None, processes: Vec::new() });
                    continue;
                }
            }
            if let Some(d) = self.read_device(index, full) {
                out.push(d);
            }
        }
        out
    }

    fn read_device(&mut self, index: usize, full: bool) -> Option<Device> {
        let mut handle: Handle = std::ptr::null_mut();
        // SAFETY: a valid out-parameter and an index below the count NVML gave.
        if unsafe { (self.api.handle)(index as c_uint, &mut handle) } != SUCCESS || handle.is_null() {
            return None;
        }
        let mut pci: PciInfo = PciInfo {
            bus_id_legacy: [0; 16],
            domain: 0,
            bus: 0,
            device: 0,
            pci_device_id: 0,
            pci_sub_system_id: 0,
            bus_id: [0; 32],
        };
        // SAFETY: `pci` is the struct NVML fills.
        let pdev = if unsafe { (self.api.pci)(handle, &mut pci) } == SUCCESS {
            normalise_bus_id(&text(&pci.bus_id))
        } else {
            format!("nvml:{index}")
        };
        self.slots[index] = Some(pdev.clone());
        let mut name_buf = [0 as c_char; 96];
        // SAFETY: the buffer is as long as the length says.
        let name = if unsafe { (self.api.name)(handle, name_buf.as_mut_ptr(), name_buf.len() as c_uint) } == SUCCESS {
            text(&name_buf)
        } else {
            String::new()
        };
        let mut rates = Utilization::default();
        // SAFETY: a valid out-parameter.
        let use_pct = (unsafe { (self.api.rates)(handle, &mut rates) } == SUCCESS).then(|| f64::from(rates.gpu.min(100)));
        let mut mem = Memory::default();
        // SAFETY: a valid out-parameter.
        let have_mem = unsafe { (self.api.memory)(handle, &mut mem) } == SUCCESS;
        let processes = if full { self.processes(index, handle) } else { Vec::new() };
        Some(Device {
            pdev,
            name,
            use_pct,
            memory_total: have_mem.then_some(mem.total),
            memory_used: have_mem.then_some(mem.used),
            processes,
        })
    }

    /// The processes that have the device open, by pid: the memory each holds (compute and graphics contexts added) and
    /// how busy it keeps the streaming multiprocessors since the last read.
    fn processes(&mut self, index: usize, handle: Handle) -> Vec<Process> {
        let mut by_pid: std::collections::BTreeMap<u32, Process> = std::collections::BTreeMap::new();
        for list in [self.api.compute, self.api.graphics].into_iter().flatten() {
            for info in call_list(list, handle) {
                let memory = (info.used_gpu_memory != VALUE_NOT_AVAILABLE).then_some(info.used_gpu_memory);
                let p = by_pid.entry(info.pid).or_insert(Process { pid: info.pid, memory: None, use_pct: None });
                p.memory = match (p.memory, memory) {
                    (Some(a), Some(b)) => Some(a.saturating_add(b)),
                    (a, b) => a.or(b),
                };
            }
        }
        if let Some(util) = self.api.process_utilization {
            let (samples, newest) = call_utilization(util, handle, self.last_seen[index]);
            self.last_seen[index] = self.last_seen[index].max(newest);
            // The newest sample of each process.
            let mut latest: std::collections::HashMap<u32, (u64, u32)> = std::collections::HashMap::new();
            for s in samples {
                let e = latest.entry(s.pid).or_insert((0, 0));
                if s.time_stamp >= e.0 {
                    *e = (s.time_stamp, s.sm_util);
                }
            }
            for (pid, (_, sm)) in latest {
                if let Some(p) = by_pid.get_mut(&pid) {
                    p.use_pct = Some(f64::from(sm.min(100)));
                }
            }
        }
        by_pid.into_values().collect()
    }
}

/// A process list: asks, grows the buffer when NVML says it was too small, and gives what it returned.
fn call_list(list: ProcessList, handle: Handle) -> Vec<ProcessInfo> {
    let mut capacity: c_uint = 32;
    for _ in 0..3 {
        let mut buf = vec![ProcessInfo::default(); capacity as usize];
        let mut count = capacity;
        // SAFETY: the buffer holds `count` entries, which is what the call is told.
        let rc = unsafe { list(handle, &mut count, buf.as_mut_ptr()) };
        match rc {
            SUCCESS => {
                buf.truncate((count as usize).min(buf.len()));
                return buf;
            }
            // `count` is now how many there are; processes come and go, so leave some room.
            INSUFFICIENT_SIZE => capacity = count.saturating_add(8).max(capacity),
            _ => return Vec::new(),
        }
    }
    Vec::new()
}

/// The per-process utilisation samples since `since`, and the newest sample's time.
fn call_utilization(f: ProcessUtilization, handle: Handle, since: u64) -> (Vec<ProcessUtilizationSample>, u64) {
    let mut count: c_uint = 0;
    // SAFETY: a null buffer with a zero count is how NVML is asked how many samples there are.
    let rc = unsafe { f(handle, std::ptr::null_mut(), &mut count, since) };
    if rc == NOT_FOUND || (rc == SUCCESS && count == 0) || (rc != SUCCESS && rc != INSUFFICIENT_SIZE) {
        return (Vec::new(), 0);
    }
    let mut buf = vec![ProcessUtilizationSample::default(); count.saturating_add(8) as usize];
    let mut count = buf.len() as c_uint;
    // SAFETY: the buffer holds `count` entries.
    if unsafe { f(handle, buf.as_mut_ptr(), &mut count, since) } != SUCCESS {
        return (Vec::new(), 0);
    }
    buf.truncate((count as usize).min(buf.len()));
    let newest = buf.iter().map(|s| s.time_stamp).max().unwrap_or(0);
    (buf, newest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    #[test]
    fn the_structs_are_the_sizes_nvml_expects() {
        assert_eq!(std::mem::size_of::<Utilization>(), 8);
        assert_eq!(std::mem::size_of::<Memory>(), 24);
        assert_eq!(std::mem::size_of::<ProcessInfo>(), 24);
        assert_eq!(std::mem::size_of::<ProcessUtilizationSample>(), 32);
        assert_eq!(std::mem::size_of::<PciInfo>(), 16 + 5 * 4 + 32);
    }

    #[test]
    fn a_bus_id_is_spelled_as_the_kernel_does() {
        assert_eq!(normalise_bus_id("00000000:01:00.0"), "0000:01:00.0");
        assert_eq!(normalise_bus_id("00000000:0A:00.0\n"), "0000:0a:00.0");
        assert_eq!(normalise_bus_id("0000:03:00.0"), "0000:03:00.0");
        assert_eq!(normalise_bus_id("garbage"), "garbage");
    }

    #[test]
    fn a_computer_without_the_library_loads_nothing_and_says_the_tools_are_not_found() {
        let err = Nvml::load_from(OsStr::new("/nonexistent/libnvidia-ml.so.1")).err().expect("no such library");
        assert!(err.starts_with("NVIDIA tools not found"), "{err}");
    }

    /// A stand-in for NVIDIA's library, built with the C compiler the machine has: one device, one compute process.
    const FAKE_NVML: &str = r#"
        #include <string.h>
        typedef void* H;
        typedef struct { unsigned gpu, memory; } U;
        typedef struct { unsigned long long total, free, used; } M;
        typedef struct { char legacy[16]; unsigned domain, bus, device, id, sub; char bus_id[32]; } P;
        typedef struct { unsigned pid; unsigned long long mem; unsigned gi, ci; } PI;
        typedef struct { unsigned pid; unsigned long long ts; unsigned sm, mem, enc, dec; } S;
        int nvmlInit_v2(void) { return 0; }
        int nvmlShutdown(void) { return 0; }
        int nvmlDeviceGetCount_v2(unsigned *n) { *n = 1; return 0; }
        int nvmlDeviceGetHandleByIndex_v2(unsigned i, H *h) { *h = (H)0x10; return 0; }
        int nvmlDeviceGetName(H h, char *b, unsigned n) { strncpy(b, "NVIDIA Fake RTX 4090", n); return 0; }
        int nvmlDeviceGetUtilizationRates(H h, U *u) { u->gpu = 42; u->memory = 10; return 0; }
        int nvmlDeviceGetMemoryInfo(H h, M *m) { m->total = 8ull << 30; m->used = 2ull << 30; m->free = 6ull << 30; return 0; }
        int nvmlDeviceGetPciInfo_v3(H h, P *p) { strcpy(p->bus_id, "00000000:01:00.0"); return 0; }
        int nvmlDeviceGetComputeRunningProcesses_v3(H h, unsigned *n, PI *p) {
            if (*n < 2) { *n = 2; return 7; }
            p[0].pid = 4242; p[0].mem = 512ull << 20; p[1].pid = 77; p[1].mem = (unsigned long long)-1;
            *n = 2; return 0;
        }
        int nvmlDeviceGetGraphicsRunningProcesses_v3(H h, unsigned *n, PI *p) {
            if (*n < 1) { *n = 1; return 7; }
            p[0].pid = 4242; p[0].mem = 100ull << 20; *n = 1; return 0;
        }
        int nvmlDeviceGetProcessUtilization(H h, S *s, unsigned *n, unsigned long long since) {
            if (!s) { *n = 2; return 7; }
            s[0].pid = 4242; s[0].ts = 100; s[0].sm = 20;
            s[1].pid = 4242; s[1].ts = 200; s[1].sm = 33;
            *n = 2; return 0;
        }
    "#;

    fn build_fake(dir: &Path) -> Option<PathBuf> {
        let src = dir.join("fake.c");
        let lib = dir.join("libnvidia-ml.so.1");
        std::fs::write(&src, FAKE_NVML).ok()?;
        let ok = std::process::Command::new("cc").args(["-shared", "-fPIC", "-o"]).arg(&lib).arg(&src).status().ok()?.success();
        ok.then_some(lib)
    }

    #[test]
    fn a_device_is_read_through_the_real_calls() {
        let dir = tempfile::tempdir().unwrap();
        let Some(lib) = build_fake(dir.path()) else {
            eprintln!("no C compiler here: the NVML calls are not exercised");
            return;
        };
        let mut nvml = Nvml::load_from(lib.as_os_str()).unwrap();
        assert_eq!(nvml.devices(), 1);
        let devices = nvml.read(true, &|_| false);
        assert_eq!(devices.len(), 1);
        let d = &devices[0];
        assert_eq!((d.pdev.as_str(), d.name.as_str()), ("0000:01:00.0", "NVIDIA Fake RTX 4090"));
        assert_eq!((d.use_pct, d.memory_total, d.memory_used), (Some(42.0), Some(8 << 30), Some(2 << 30)));
        // Compute and graphics contexts of one process add up; a memory figure the driver can't give is none; the newest
        // utilisation sample is the one used.
        assert_eq!(
            d.processes,
            [
                Process { pid: 77, memory: None, use_pct: None },
                Process { pid: 4242, memory: Some(612 << 20), use_pct: Some(33.0) },
            ]
        );
    }

    #[test]
    fn the_status_bar_read_leaves_the_processes_alone() {
        let dir = tempfile::tempdir().unwrap();
        let Some(lib) = build_fake(dir.path()) else { return };
        let mut nvml = Nvml::load_from(lib.as_os_str()).unwrap();
        let devices = nvml.read(false, &|_| false);
        assert!(devices[0].processes.is_empty());
        assert_eq!(devices[0].use_pct, Some(42.0));
    }

    #[test]
    fn a_device_the_kernel_has_powered_down_is_not_asked_anything() {
        let dir = tempfile::tempdir().unwrap();
        let Some(lib) = build_fake(dir.path()) else { return };
        let mut nvml = Nvml::load_from(lib.as_os_str()).unwrap();
        // The first read finds the slot (the device is awake); the next, with the slot asleep, leaves it alone.
        nvml.read(false, &|_| false);
        let devices = nvml.read(true, &|slot| slot == "0000:01:00.0");
        assert_eq!(devices.len(), 1);
        assert_eq!((devices[0].use_pct, devices[0].memory_used, devices[0].processes.len()), (None, None, 0));
        assert_eq!(devices[0].pdev, "0000:01:00.0");
    }
}
