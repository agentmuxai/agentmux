// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The GPU's use and memory, for the status bar's GPU meter and its panel
//! (SPEC_STATUS_BAR_GPU_PANEL_2026_11_15.md).
//!
//! **One sampler, and it is srv's:** [`GpuSampler::global`] holds the one open set of counters. The status bar's
//! compact figure asks it every few seconds from the telemetry thread ([`CompactPoller`]); the panel asks it every
//! two seconds while it is open (`gpu.sample`). A read newer than the asker's reuse window is handed back instead
//! of made again, so two askers never read twice in a row. Per-process names are not made here: the panel's handler
//! takes them from Tower's own sample, which is the one place that names a process.
//!
//! **Cost:** reading the counters is the cost, so it is bounded. A read that takes longer than its budget
//! ([`Budget`]) widens the interval to the next read, and the panel's footer says so; reads never run back to back.
//! `cost_tests` measures a read (run in CI, and on a machine of its own: `cargo test -p agentmux-srv-core gpu::cost
//! -- --ignored --nocapture`).
//!
//! Windows reads the PDH counters (`pdh`). Linux reads what the kernel and the driver publish (`linux`: AMD's sysfs files,
//! NVIDIA through NVML loaded at run time, `fdinfo` for the processes, Intel's memory only). macOS is a later phase and
//! reports [`GpuSampleState::Unsupported`].

pub mod aggregate;
#[cfg(any(target_os = "linux", test))]
pub mod drm;
#[cfg(target_os = "linux")]
mod linux;
pub mod names;
#[cfg(target_os = "linux")]
mod nvml;
#[cfg(windows)]
mod pdh;

use crate::backend::rpc_types::{GpuOthers, GpuProcess, GpuSampleState, TowerProcessKind};
use aggregate::{memory_share, ProcessFigures, Reading};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// A read slower than this widens the interval to the next one.
pub const READ_BUDGET: Duration = Duration::from_millis(50);
/// How often the status bar's figure is read, and the longest it is stretched to.
pub const COMPACT_EVERY: Duration = Duration::from_secs(5);
const COMPACT_MAX: Duration = Duration::from_secs(60);
/// How often the open panel is read, and the longest it is stretched to.
pub const PANEL_EVERY: Duration = Duration::from_secs(2);
const PANEL_MAX: Duration = Duration::from_secs(10);
/// A read this recent answers the status bar's tick, and the panel's.
const COMPACT_REUSE: Duration = Duration::from_secs(4);
const PANEL_REUSE: Duration = Duration::from_millis(1000);
/// A computer with no counters is asked again this often (a driver installed, a GPU plugged in).
const UNAVAILABLE_RETRY: Duration = Duration::from_secs(300);
/// Reads in a row that fail before the counters are given up on for [`UNAVAILABLE_RETRY`].
const FAILURES_ALLOWED: u32 = 3;
/// The biggest other users of the GPU that are listed by name.
pub const OTHERS_LISTED: usize = 5;

/// An interval that stretches when a read costs too much and comes back when it doesn't.
#[derive(Debug, Clone)]
pub struct Budget {
    base: Duration,
    max: Duration,
    current: Duration,
}

impl Budget {
    pub fn new(base: Duration, max: Duration) -> Self {
        Self { base, max, current: base }
    }

    /// Note that a read took `took`; returns the interval to the next one.
    pub fn record(&mut self, took: Duration) -> Duration {
        if took > READ_BUDGET {
            self.current = (self.current * 2).min(self.max);
        } else if took < READ_BUDGET / 2 {
            self.current = (self.current / 2).max(self.base);
        }
        self.current
    }

    pub fn interval(&self) -> Duration {
        self.current
    }

    /// Whether the interval is longer than it would be if reads were cheap.
    pub fn widened(&self) -> bool {
        self.current > self.base
    }
}

/// A read, as the sampler hands it out.
#[derive(Debug, Clone)]
pub struct Sample {
    pub state: GpuSampleState,
    pub reading: Reading,
    /// How long the read took (the one this was made from).
    pub took: Duration,
    /// It includes the processes' memory.
    full: bool,
}

impl Sample {
    fn without_gpu(state: GpuSampleState) -> Self {
        Self { state, reading: Reading::default(), took: Duration::ZERO, full: true }
    }
}

struct Inner {
    #[cfg(windows)]
    gpu: Option<pdh::Gpu>,
    #[cfg(target_os = "linux")]
    gpu: Option<linux::Gpu>,
    /// Counters could not be opened or read: not before this.
    retry_at: Option<Instant>,
    failures: u32,
    last: Option<(Instant, Sample)>,
    panel: Budget,
    compact: Budget,
}

/// The one open set of GPU counters (module doc).
pub struct GpuSampler {
    inner: Mutex<Inner>,
}

impl Default for GpuSampler {
    fn default() -> Self {
        Self::new()
    }
}

impl GpuSampler {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                #[cfg(any(windows, target_os = "linux"))]
                gpu: None,
                retry_at: None,
                failures: 0,
                last: None,
                panel: Budget::new(PANEL_EVERY, PANEL_MAX),
                compact: Budget::new(COMPACT_EVERY, COMPACT_MAX),
            }),
        }
    }

    pub fn global() -> &'static GpuSampler {
        static GLOBAL: OnceLock<GpuSampler> = OnceLock::new();
        GLOBAL.get_or_init(GpuSampler::new)
    }

    /// The panel's sample: adapters, processes and their memory. BLOCKING (module doc); call it off the async workers.
    /// Returns the interval to ask again after, and whether it was widened.
    pub fn sample_for_panel(&self) -> (Sample, Duration, bool) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let sample = inner.read(true, PANEL_REUSE);
        (sample, inner.panel.interval(), inner.panel.widened())
    }

    /// The status bar's figures from a read, or the panel's read if it is new enough, and the interval to the next:
    /// `None` while the panel is reading (`try_lock`: a telemetry tick never waits for it), `Some(None)` for a
    /// computer with no GPU counters.
    fn read_compact(&self) -> Option<(Option<Compact>, Duration)> {
        let mut inner = self.inner.try_lock().ok()?;
        let sample = inner.read(false, COMPACT_REUSE);
        Some((compact_of(&sample.reading), inner.compact.interval()))
    }
}

impl Inner {
    /// A read, or the last one if it is newer than `reuse` (and has the processes' memory when `full` wants it).
    fn read(&mut self, full: bool, reuse: Duration) -> Sample {
        if let Some((at, s)) = &self.last {
            if at.elapsed() < reuse && (s.full || !full) {
                return s.clone();
            }
        }
        if self.retry_at.is_some_and(|t| Instant::now() < t) {
            return Sample::without_gpu(GpuSampleState::Unavailable);
        }
        let started = Instant::now();
        let outcome = self.read_now(full);
        let took = started.elapsed();
        let budget = if full { &mut self.panel } else { &mut self.compact };
        let widened = budget.record(took);
        tracing::trace!(took_ms = took.as_millis() as u64, next_ms = widened.as_millis() as u64, full, "gpu: read");
        let sample = match outcome {
            Ok((state, reading)) => {
                self.failures = 0;
                Sample { state, reading, took, full }
            }
            Err(why) => {
                self.failures += 1;
                tracing::debug!(error = %why, failures = self.failures, "gpu: could not read the counters");
                if self.failures >= FAILURES_ALLOWED {
                    self.retry_at = Some(Instant::now() + UNAVAILABLE_RETRY);
                    self.drop_counters();
                }
                Sample { took, ..Sample::without_gpu(GpuSampleState::Unavailable) }
            }
        };
        self.last = Some((Instant::now(), sample.clone()));
        sample
    }

    #[cfg(any(windows, target_os = "linux"))]
    fn drop_counters(&mut self) {
        self.gpu = None;
    }

    #[cfg(not(any(windows, target_os = "linux")))]
    fn drop_counters(&mut self) {}

    #[cfg(windows)]
    fn read_now(&mut self, full: bool) -> Result<(GpuSampleState, Reading), String> {
        if self.gpu.is_none() {
            match pdh::Gpu::open() {
                Ok(gpu) => self.gpu = Some(gpu),
                Err(why) => {
                    self.retry_at = Some(Instant::now() + UNAVAILABLE_RETRY);
                    tracing::debug!(error = %why, "gpu: no counters on this computer");
                    return Ok((GpuSampleState::Unavailable, Reading::default()));
                }
            }
        }
        let gpu = self.gpu.as_mut().expect("opened above");
        match gpu.read(full)? {
            pdh::Read::Warming => Ok((GpuSampleState::Warming, Reading::default())),
            pdh::Read::Counters(raw, infos) => Ok((GpuSampleState::Ok, aggregate::aggregate(&raw, &infos))),
        }
    }

    #[cfg(target_os = "linux")]
    fn read_now(&mut self, full: bool) -> Result<(GpuSampleState, Reading), String> {
        if self.gpu.is_none() {
            match linux::Gpu::open() {
                Ok(gpu) => self.gpu = Some(gpu),
                Err(why) => {
                    self.retry_at = Some(Instant::now() + UNAVAILABLE_RETRY);
                    tracing::debug!(error = %why, "gpu: no counters on this computer");
                    return Ok((GpuSampleState::Unavailable, Reading::default()));
                }
            }
        }
        let gpu = self.gpu.as_mut().expect("opened above");
        match gpu.read(full)? {
            linux::Read::Warming => Ok((GpuSampleState::Warming, Reading::default())),
            linux::Read::Reading(reading) => Ok((GpuSampleState::Ok, reading)),
        }
    }

    #[cfg(not(any(windows, target_os = "linux")))]
    fn read_now(&mut self, _full: bool) -> Result<(GpuSampleState, Reading), String> {
        Ok((GpuSampleState::Unsupported, Reading::default()))
    }
}

/// The status bar's two figures: the busiest adapter's use, and the fullest adapter's memory, percent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Compact {
    pub use_pct: f64,
    pub mem_pct: Option<f64>,
}

/// The status bar's figures from a read; `None` when no adapter's use is known (nothing readable: the meter is
/// hidden, not shown as 0% or an error).
pub fn compact_of(reading: &Reading) -> Option<Compact> {
    let use_pct = reading.adapters.iter().filter_map(|a| a.use_pct).fold(None, |m: Option<f64>, v| Some(m.map_or(v, |m| m.max(v))))?;
    let mem_pct = reading.adapters.iter().filter_map(memory_share).fold(None, |m: Option<f64>, v| Some(m.map_or(v, |m| m.max(v))));
    Some(Compact { use_pct, mem_pct })
}

/// The telemetry tick's view of the sampler: asks it every [`COMPACT_EVERY`] (stretched when reads are slow) and
/// keeps the answer for the ticks between.
#[derive(Default)]
pub struct CompactPoller {
    next: Option<Instant>,
    shown: Option<Compact>,
}

impl CompactPoller {
    /// The figures to put in this tick's event, `None` while nothing is readable.
    pub fn poll(&mut self, sampler: &GpuSampler) -> Option<Compact> {
        let now = Instant::now();
        if self.next.is_none_or(|t| now >= t) {
            if let Some((shown, wait)) = sampler.read_compact() {
                self.shown = shown;
                self.next = Some(now + wait);
            } else {
                // The panel is reading: its read is the answer in a moment.
                self.next = Some(now + Duration::from_millis(500));
            }
        }
        self.shown
    }
}

/// Puts the status bar's GPU figures, `gpu` and `gpu:mem`, in a telemetry tick's values: read every few seconds by the
/// one sampler (module doc) and repeated in the ticks between. Nothing is put in while nothing is readable, so the
/// meter is hidden, not shown as 0%.
pub fn add_compact_values(values: &mut std::collections::HashMap<String, f64>) {
    static POLLER: OnceLock<Mutex<CompactPoller>> = OnceLock::new();
    let mut poller = POLLER.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    if let Some(c) = poller.poll(GpuSampler::global()) {
        values.insert("gpu".to_string(), c.use_pct);
        if let Some(mem) = c.mem_pct {
            values.insert("gpu:mem".to_string(), mem);
        }
    }
}

/// What Tower knows of a process by its id: an image name, and for AgentMux's own what it is.
#[derive(Debug, Clone)]
pub struct Named {
    pub name: String,
    pub label: Option<String>,
    pub kind: Option<TowerProcessKind>,
}

/// The most one process can hold: the biggest adapter's dedicated memory and the most shared memory the OS lets the GPU
/// have. `None` where the adapters said nothing, so nothing is dropped.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MemoryLimits {
    pub dedicated: Option<u64>,
    pub shared: Option<u64>,
}

impl MemoryLimits {
    pub fn of(adapters: &[crate::backend::rpc_types::GpuAdapter]) -> Self {
        Self {
            dedicated: adapters.iter().filter_map(|a| a.dedicated_total).max(),
            shared: adapters.iter().filter_map(|a| a.shared_total).max(),
        }
    }
}

/// Whether a process is worth a row: it uses the GPU, or holds a megabyte of its memory. Where the use is not known at
/// all (Intel's GPUs say none), the machine's memory it holds is the one thing there is to show.
fn active(f: &ProcessFigures) -> bool {
    const MEMORY: u64 = 1 << 20;
    f.use_pct.is_some_and(|u| u >= 0.5)
        || f.dedicated.unwrap_or(0) >= MEMORY
        || (f.use_pct.is_none() && f.shared.unwrap_or(0) >= MEMORY)
}

/// The rows of the panel: AgentMux's own processes and the biggest [`OTHERS_LISTED`] others, sorted by use and then
/// memory (AgentMux's first where those are equal), and the rest of the others added up.
///
/// A row's memory is what it holds on the card, and only where it holds none there, what it holds of the machine's.
/// A figure over what the adapters have is the OS's own mistake (it reports 113 GB for a compositor on a 12 GB card, and
/// 23 TB for another process), and is not shown as a number: see [`MemoryLimits`].
pub fn shape_processes(
    figures: &[ProcessFigures],
    limits: MemoryLimits,
    name_of: impl Fn(u32) -> Option<Named>,
) -> (Vec<GpuProcess>, Option<GpuOthers>) {
    let sane = |v: Option<u64>, limit: Option<u64>| v.filter(|v| limit.is_none_or(|l| *v <= l));
    let figures: Vec<ProcessFigures> = figures
        .iter()
        .map(|f| ProcessFigures {
            dedicated: sane(f.dedicated, limits.dedicated),
            shared: sane(f.shared, limits.shared),
            ..f.clone()
        })
        .collect();
    let row = |f: &ProcessFigures| {
        let n = name_of(f.pid);
        GpuProcess {
            pid: f.pid,
            name: n.as_ref().map_or_else(|| format!("pid {}", f.pid), |n| n.name.clone()),
            label: n.as_ref().and_then(|n| n.label.clone()),
            kind: n.as_ref().and_then(|n| n.kind),
            use_pct: f.use_pct,
            dedicated: f.dedicated,
            shared: f.shared.filter(|_| f.dedicated.unwrap_or(0) == 0),
        }
    };
    let (mut own, mut others): (Vec<GpuProcess>, Vec<GpuProcess>) =
        figures.iter().filter(|f| active(f)).map(row).partition(|r| r.label.is_some());
    let memory = |r: &GpuProcess| r.dedicated.unwrap_or(0) + r.shared.unwrap_or(0);
    // A tie is a tie as the panel prints it: whole percent.
    let by_use_then_memory = |a: &GpuProcess, b: &GpuProcess| {
        let pct = |r: &GpuProcess| r.use_pct.unwrap_or(0.0).round() as i64;
        pct(b).cmp(&pct(a)).then(memory(b).cmp(&memory(a)))
    };
    others.sort_by(|a, b| by_use_then_memory(a, b).then(a.pid.cmp(&b.pid)));
    let rest = others.split_off(others.len().min(OTHERS_LISTED));
    let sum = |f: fn(&GpuProcess) -> Option<f64>| rest.iter().filter_map(f).reduce(|a, b| a + b);
    let tail = (!rest.is_empty()).then(|| GpuOthers {
        processes: rest.len() as u32,
        use_pct: sum(|r| r.use_pct),
        dedicated: sum(|r| r.dedicated.map(|v| v as f64)).map(|v| v as u64),
        shared: sum(|r| r.shared.map(|v| v as f64)).map(|v| v as u64),
    });
    own.append(&mut others);
    // Own rows were pushed first, so the stable sort keeps them first inside a tie.
    own.sort_by(by_use_then_memory);
    (own, tail)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod cost_tests;
