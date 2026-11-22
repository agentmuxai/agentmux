// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Windows: the GPU performance counters (PDH), and the adapters DXGI lists to name them. Both are the
//! system's own, for every vendor: nothing is installed and no process is started.
//!
//! The counters are wildcards over every process's engines, so one read enumerates hundreds of instances: that is
//! the cost (`backend::gpu` module doc), and why a read is kept to one query that stays open between reads. The
//! engine counter is a rate and needs two collections before it means anything, which is what [`Read::Warming`] is.

use super::aggregate::{AdapterInfo, AdapterType, Instances, RawCounters};
use super::names::luid;
use std::time::{Duration, Instant};
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1};
use windows::Win32::System::Performance::{
    PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterArrayW, PdhOpenQueryW,
    PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE, PDH_HCOUNTER, PDH_HQUERY, PDH_MORE_DATA,
};

const ENGINE: &str = r"\GPU Engine(*)\Utilization Percentage";
const ADAPTER_DEDICATED: &str = r"\GPU Adapter Memory(*)\Dedicated Usage";
const ADAPTER_SHARED: &str = r"\GPU Adapter Memory(*)\Shared Usage";
const PROCESS_DEDICATED: &str = r"\GPU Process Memory(*)\Dedicated Usage";
const PROCESS_SHARED: &str = r"\GPU Process Memory(*)\Shared Usage";

/// `DXGI_ADAPTER_FLAG_SOFTWARE`.
const DXGI_SOFTWARE: u32 = 2;
/// DXGI is asked again for the adapters this often at most, when the counters name one it hasn't listed.
const ADAPTERS_AGAIN: Duration = Duration::from_secs(30);

/// A PDH query and the counters added to it. A counter the machine doesn't have is `None` in its place.
struct Query {
    handle: PDH_HQUERY,
    counters: Vec<Option<PDH_HCOUNTER>>,
    collected: u32,
}

// The handle is only touched under `Gpu`'s owner's lock, one thread at a time.
unsafe impl Send for Query {}

impl Query {
    fn open(paths: &[&str]) -> Result<Query, u32> {
        let mut handle = PDH_HQUERY::default();
        // SAFETY: `handle` is a valid out pointer; the query is closed in `Drop`.
        let status = unsafe { PdhOpenQueryW(PCWSTR::null(), 0, &mut handle) };
        if status != 0 {
            return Err(status);
        }
        let mut query = Query { handle, counters: Vec::new(), collected: 0 };
        for path in paths {
            let mut counter = PDH_HCOUNTER::default();
            // SAFETY: the query is open and `counter` is a valid out pointer.
            let status = unsafe { PdhAddEnglishCounterW(query.handle, &HSTRING::from(*path), 0, &mut counter) };
            query.counters.push((status == 0).then_some(counter));
        }
        Ok(query)
    }

    /// Whether any counter was added: a machine whose GPU driver has no counters adds none.
    fn has_any(&self) -> bool {
        self.counters.iter().any(Option::is_some)
    }

    fn collect(&mut self) -> Result<(), u32> {
        // SAFETY: the query is open.
        let status = unsafe { PdhCollectQueryData(self.handle) };
        if status != 0 {
            return Err(status);
        }
        self.collected += 1;
        Ok(())
    }

    /// Counter `i`'s instances and values, `None` for a counter this machine lacks or can't read yet.
    fn instances(&self, i: usize) -> Option<Instances> {
        let counter = (*self.counters.get(i)?)?;
        let (mut size, mut count) = (0u32, 0u32);
        // Instances come and go between the two calls: ask again with the size the first one wanted, a few times.
        for _ in 0..4 {
            // SAFETY: asking for the size needed; no buffer is passed.
            let status = unsafe { PdhGetFormattedCounterArrayW(counter, PDH_FMT_DOUBLE, &mut size, &mut count, None) };
            if status == 0 {
                return Some(Vec::new());
            }
            if status != PDH_MORE_DATA {
                return None;
            }
            // u64 words: the items hold 8-byte values and pointers.
            let mut buf = vec![0u64; (size as usize).div_ceil(8)];
            let items = buf.as_mut_ptr().cast::<PDH_FMT_COUNTERVALUE_ITEM_W>();
            // SAFETY: `buf` is at least `size` bytes, as the first call said, and aligned for the items.
            let status =
                unsafe { PdhGetFormattedCounterArrayW(counter, PDH_FMT_DOUBLE, &mut size, &mut count, Some(items)) };
            if status == PDH_MORE_DATA {
                continue;
            }
            if status != 0 {
                return None;
            }
            let mut out = Vec::with_capacity(count as usize);
            for k in 0..count as usize {
                // SAFETY: PDH wrote `count` items into the buffer; their names are in the same buffer.
                let item = unsafe { &*items.add(k) };
                // 0 is valid data and 1 is new data; anything else is a value this round could not make.
                if item.FmtValue.CStatus > 1 {
                    continue;
                }
                let Ok(name) = (unsafe { item.szName.to_string() }) else { continue };
                // SAFETY: the format asked for a double.
                out.push((name, unsafe { item.FmtValue.Anonymous.doubleValue }));
            }
            return Some(out);
        }
        None
    }
}

impl Drop for Query {
    fn drop(&mut self) {
        // SAFETY: the query is open and is not used again.
        unsafe { PdhCloseQuery(self.handle) };
    }
}

/// The outcome of a read.
pub enum Read {
    /// Read: the counters' instances and the adapters DXGI lists.
    Counters(RawCounters, Vec<AdapterInfo>),
    /// The first collection of a query that needs two: ask again.
    Warming,
}

/// The counters, open between reads.
pub struct Gpu {
    adapters: Query,
    processes: Option<Query>,
    infos: Vec<AdapterInfo>,
    infos_at: Instant,
}

impl Gpu {
    /// Open the counters; `Err` when this machine has none of the GPU ones (before Windows 10 1709, or a driver without WDDM 2.0).
    pub fn open() -> Result<Gpu, String> {
        let adapters = Query::open(&[ENGINE, ADAPTER_DEDICATED, ADAPTER_SHARED])
            .map_err(|s| format!("PDH could not open a query (0x{s:08X})"))?;
        if !adapters.has_any() {
            return Err("this computer has no GPU performance counters".to_string());
        }
        Ok(Gpu { adapters, processes: None, infos: dxgi_adapters(), infos_at: Instant::now() })
    }

    /// Read now. `want_processes` also reads the processes' memory, which opens its own query on the first such read
    /// and closes it on the first read that doesn't want it (the panel was closed).
    pub fn read(&mut self, want_processes: bool) -> Result<Read, String> {
        self.adapters.collect().map_err(|s| format!("PDH could not collect (0x{s:08X})"))?;
        if want_processes {
            if self.processes.is_none() {
                self.processes = Query::open(&[PROCESS_DEDICATED, PROCESS_SHARED]).ok().filter(Query::has_any);
            }
            if let Some(q) = self.processes.as_mut() {
                q.collect().map_err(|s| format!("PDH could not collect (0x{s:08X})"))?;
            }
        } else {
            self.processes = None;
        }
        // The engine counter is a rate: its first collection has nothing to compare with.
        if self.adapters.collected < 2 {
            return Ok(Read::Warming);
        }
        let raw = RawCounters {
            engines: self.adapters.instances(0),
            adapter_dedicated: self.adapters.instances(1),
            adapter_shared: self.adapters.instances(2),
            process_dedicated: self.processes.as_ref().and_then(|q| q.instances(0)),
            process_shared: self.processes.as_ref().and_then(|q| q.instances(1)),
        };
        if self.infos.is_empty() || (self.infos_at.elapsed() >= ADAPTERS_AGAIN && names_unlisted(&raw, &self.infos)) {
            self.infos = dxgi_adapters();
            self.infos_at = Instant::now();
        }
        Ok(Read::Counters(raw, self.infos.clone()))
    }
}

/// Whether the adapter counters name an adapter DXGI hasn't listed (one added or enabled since).
fn names_unlisted(raw: &RawCounters, infos: &[AdapterInfo]) -> bool {
    raw.adapter_dedicated
        .iter()
        .flatten()
        .filter_map(|(n, _)| super::names::parse_adapter(n))
        .any(|l| !infos.iter().any(|i| i.luid == l))
}

/// The kernel's adapter-type query (`D3DKMTQueryAdapterInfo` with `KMTQAITYPE_ADAPTERTYPE`, gdi32): what Windows says an
/// adapter is. Declared here, with the few types it takes, rather than through a binding crate: three calls and three
/// structs, laid out as `d3dkmthk.h` does.
mod kmt {
    use super::AdapterType;

    #[repr(C)]
    struct Luid {
        low: u32,
        high: i32,
    }

    #[repr(C)]
    struct OpenAdapterFromLuid {
        adapter_luid: Luid,
        h_adapter: u32,
    }

    #[repr(C)]
    struct QueryAdapterInfo {
        h_adapter: u32,
        /// `KMTQAITYPE_ADAPTERTYPE`.
        kind: u32,
        private_driver_data: *mut core::ffi::c_void,
        private_driver_data_size: u32,
    }

    #[repr(C)]
    struct CloseAdapter {
        h_adapter: u32,
    }

    #[link(name = "gdi32", kind = "raw-dylib")]
    extern "system" {
        fn D3DKMTOpenAdapterFromLuid(open: *mut OpenAdapterFromLuid) -> i32;
        fn D3DKMTQueryAdapterInfo(query: *mut QueryAdapterInfo) -> i32;
        fn D3DKMTCloseAdapter(close: *mut CloseAdapter) -> i32;
    }

    const KMTQAITYPE_ADAPTERTYPE: u32 = 15;

    /// What Windows says the adapter with this id is; `None` when it won't say (the query failed: the adapter went away, or a
    /// driver without the query), in which case nothing is assumed of it.
    pub fn adapter_type(high: i32, low: u32) -> Option<AdapterType> {
        let mut open = OpenAdapterFromLuid { adapter_luid: Luid { low, high }, h_adapter: 0 };
        // SAFETY: the structs are the kernel's own layouts, owned here for the call; the handle opened is closed below.
        unsafe {
            if D3DKMTOpenAdapterFromLuid(&mut open) != 0 {
                return None;
            }
            let mut value: u32 = 0;
            let mut query = QueryAdapterInfo {
                h_adapter: open.h_adapter,
                kind: KMTQAITYPE_ADAPTERTYPE,
                private_driver_data: (&mut value as *mut u32).cast(),
                private_driver_data_size: core::mem::size_of::<u32>() as u32,
            };
            let status = D3DKMTQueryAdapterInfo(&mut query);
            let mut close = CloseAdapter { h_adapter: open.h_adapter };
            D3DKMTCloseAdapter(&mut close);
            (status == 0).then_some(AdapterType(value))
        }
    }
}

/// The adapters DXGI lists: their names and memory, in the spelling of an id the counters use.
fn dxgi_adapters() -> Vec<AdapterInfo> {
    let mut out = Vec::new();
    // SAFETY: plain DXGI calls; every interface is released when it drops.
    unsafe {
        let Ok(factory) = CreateDXGIFactory1::<IDXGIFactory1>() else { return out };
        let mut i = 0;
        while let Ok(adapter) = factory.EnumAdapters1(i) {
            i += 1;
            let Ok(d) = adapter.GetDesc1() else { continue };
            let end = d.Description.iter().position(|&c| c == 0).unwrap_or(d.Description.len());
            // What the kernel says the adapter is, which DXGI's description (a name and a memory size) can't: a virtual display
            // adapter is listed by DXGI under the name and memory of the GPU that draws for it.
            let kind = kmt::adapter_type(d.AdapterLuid.HighPart, d.AdapterLuid.LowPart);
            out.push(AdapterInfo {
                luid: luid(d.AdapterLuid.HighPart, d.AdapterLuid.LowPart),
                name: String::from_utf16_lossy(&d.Description[..end]).trim().to_string(),
                software: d.Flags & DXGI_SOFTWARE != 0 || kind.is_some_and(AdapterType::is_software),
                virtual_display: kind.is_some_and(AdapterType::is_virtual_display),
                dedicated_total: d.DedicatedVideoMemory as u64,
                shared_total: d.SharedSystemMemory as u64,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Opens the counters on whatever machine runs the tests, which may have none (a CI runner without a GPU):
    /// either answer is fine, it must not panic or hang. The cost of a read is measured by `gpu::cost_tests`.
    #[test]
    fn the_counters_open_or_say_why_they_cannot() {
        match Gpu::open() {
            Ok(mut gpu) => {
                assert!(matches!(gpu.read(true), Ok(Read::Warming)), "the first read is a collection to compare with");
                std::thread::sleep(Duration::from_millis(150));
                assert!(gpu.read(true).is_ok());
            }
            Err(why) => assert!(!why.is_empty()),
        }
    }

    /// The kernel answers for every adapter DXGI lists on this machine (or, for one that went away between the two calls,
    /// says nothing): never a panic, and never "virtual" for the only adapter a machine has unless it truly is one, which
    /// `aggregate` keeps anyway.
    #[test]
    fn the_kernel_says_what_each_listed_adapter_is() {
        for a in dxgi_adapters() {
            assert!(!(a.software && a.virtual_display), "an adapter is one or the other: {a:?}");
        }
        // An id no adapter has: no answer, not a made-up one.
        assert!(kmt::adapter_type(0x7fff_ffff, 0xdead_beef).is_none());
    }

    #[test]
    fn dxgi_lists_adapters_in_the_counters_spelling() {
        for a in dxgi_adapters() {
            assert!(super::super::names::parse_adapter(&format!("luid_{}_phys_0", a.luid)).is_some(), "{a:?}");
        }
    }
}
