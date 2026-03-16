# Spec: GPU Monitoring & Extended System Metrics

**Goal:** Add GPU utilization/memory/temperature to the status bar and sysinfo view, plus expose additional system metrics (per-core CPU, disk I/O, temperatures, swap, battery) for the sysinfo pane plots.

**Status:** Design phase. Ready for implementation after review.

---

## Current State

The sysinfo collector (`agentmuxsrv-rs/src/backend/sysinfo.rs`) runs a configurable-interval loop (0.2–2.0s, default 1s) using the `sysinfo` crate v0.34. It publishes `TimeSeriesData` via WPS events:

| Metric | Key | Unit | Source |
|--------|-----|------|--------|
| CPU (aggregate) | `cpu` | % (0–100) | `sysinfo::System::global_cpu_usage()` |
| CPU per-core | `cpu:0` … `cpu:31` | % | `sysinfo::System::cpus()` |
| Memory used | `mem:used` | GB | `sysinfo::System::used_memory()` |
| Memory total | `mem:total` | GB | `sysinfo::System::total_memory()` |
| Memory free | `mem:free` | GB | `sysinfo::System::free_memory()` |
| Memory available | `mem:available` | GB | `sysinfo::System::available_memory()` |
| Network sent | `net:bytessent` | MB/s | delta from `sysinfo::Networks` |
| Network received | `net:bytesrecv` | MB/s | delta from `sysinfo::Networks` |
| Network total | `net:bytestotal` | MB/s | sum of sent + recv |

**Not collected:** GPU, disk I/O, temperatures, swap, battery, load average.

---

## Part 1: GPU Metrics

### The Problem

GPU monitoring has no single cross-platform solution. Each vendor has its own API:

| Vendor | API | Rust Crate | Platforms | Quality |
|--------|-----|-----------|-----------|---------|
| **NVIDIA** | NVML (NVIDIA Management Library) | `nvml-wrapper` (v0.10+) | Win/Linux | Mature, well-maintained, wraps official C API |
| **AMD** | ADL (AMD Display Library) / ROCm SMI | None stable | Win/Linux | No maintained Rust crate; `amdgpu-sysfs` for Linux only |
| **Intel** (integrated) | — | — | — | No public utilization API on Windows; Linux has `i915` perf counters |
| **Apple** (M-series) | IOKit / Metal Performance Shaders | — | macOS | No Rust crate; requires raw `IOKit` FFI |

### Recommended Approach: Tiered Provider Model

Don't try to build one abstraction. Instead, implement a provider trait with vendor-specific backends that are compiled conditionally.

```rust
/// Common GPU snapshot — all fields optional since not every vendor reports everything.
pub struct GpuSnapshot {
    pub name: String,                    // e.g. "NVIDIA RTX 4090", "AMD RX 7900"
    pub utilization_pct: Option<f64>,    // 0–100
    pub memory_used_mb: Option<f64>,
    pub memory_total_mb: Option<f64>,
    pub temperature_c: Option<f64>,
    pub power_watts: Option<f64>,
    pub fan_pct: Option<f64>,
    pub encoder_pct: Option<f64>,        // video encode utilization
    pub decoder_pct: Option<f64>,        // video decode utilization
}

pub trait GpuProvider: Send + Sync {
    fn refresh(&mut self) -> Vec<GpuSnapshot>;
}
```

### Implementation: NVIDIA First (Highest Coverage)

**Phase 1: NVIDIA via `nvml-wrapper`**

```toml
[dependencies]
nvml-wrapper = { version = "0.10", optional = true }

[features]
default = ["gpu-nvidia"]
gpu-nvidia = ["dep:nvml-wrapper"]
```

NVML is a dynamic library — `nvml-wrapper` loads `nvml.dll` (Windows) or `libnvidia-ml.so` (Linux) at runtime. If the driver isn't installed, initialization returns `Err` and we gracefully degrade (no GPU metrics, no crash).

```rust
#[cfg(feature = "gpu-nvidia")]
mod nvidia_provider {
    use nvml_wrapper::Nvml;

    pub struct NvidiaProvider {
        nvml: Nvml,
        device_count: u32,
    }

    impl NvidiaProvider {
        pub fn try_new() -> Option<Self> {
            let nvml = Nvml::init().ok()?;
            let count = nvml.device_count().ok()?;
            Some(Self { nvml, device_count: count })
        }
    }

    impl GpuProvider for NvidiaProvider {
        fn refresh(&mut self) -> Vec<GpuSnapshot> {
            (0..self.device_count)
                .filter_map(|i| {
                    let dev = self.nvml.device_by_index(i).ok()?;
                    let util = dev.utilization_rates().ok()?;
                    let mem = dev.memory_info().ok()?;
                    Some(GpuSnapshot {
                        name: dev.name().ok()?,
                        utilization_pct: Some(util.gpu as f64),
                        memory_used_mb: Some(mem.used as f64 / 1_048_576.0),
                        memory_total_mb: Some(mem.total as f64 / 1_048_576.0),
                        temperature_c: dev.temperature(TemperatureSensor::Gpu).ok().map(|t| t as f64),
                        power_watts: dev.power_usage().ok().map(|mw| mw as f64 / 1000.0),
                        fan_pct: dev.fan_speed(0).ok().map(|f| f as f64),
                        encoder_pct: dev.encoder_utilization().ok().map(|(u, _)| u as f64),
                        decoder_pct: dev.decoder_utilization().ok().map(|(u, _)| u as f64),
                    })
                })
                .collect()
        }
    }
}
```

**Phase 2: AMD (Linux only, via sysfs)**

Linux exposes AMD GPU metrics via `/sys/class/drm/card*/device/`:
- `gpu_busy_percent` — utilization %
- `mem_info_vram_used` / `mem_info_vram_total` — VRAM
- `hwmon/hwmon*/temp1_input` — temperature (millidegrees)
- `hwmon/hwmon*/power1_average` — power (microwatts)

This requires no external crate — just `std::fs::read_to_string` on sysfs paths. Compile with `#[cfg(target_os = "linux")]`.

**Phase 3: Apple Silicon (macOS, via IOKit FFI)**

Requires `core-foundation` + `IOKit` bindings. Lower priority — macOS users typically don't monitor GPU for terminal workloads.

**Phase 4: Intel Arc (future)**

Intel's `oneAPI Level Zero` has GPU telemetry but no Rust crate. Windows WMI has `Win32_VideoController` but it only reports static info (name, VRAM), not utilization. Defer until ecosystem matures.

### Graceful Degradation

The GPU provider is initialized once at startup. If no provider succeeds (no NVIDIA driver, no AMD sysfs), `gpu_provider` is `None` and the collector simply skips GPU metrics. The frontend handles missing keys gracefully — `GpuSnapshot` fields are all `Option`.

```rust
// In sysinfo.rs startup:
let gpu_provider: Option<Box<dyn GpuProvider>> = {
    #[cfg(feature = "gpu-nvidia")]
    if let Some(p) = nvidia_provider::NvidiaProvider::try_new() {
        Some(Box::new(p))
    } else { None }
    // ... try AMD, etc.
};
```

### WPS Event Keys

For multi-GPU systems, index by device number. Single GPU gets unindexed keys for convenience:

| Key | Unit | Description |
|-----|------|-------------|
| `gpu` | % | Primary GPU utilization |
| `gpu:mem:used` | MB | Primary GPU VRAM used |
| `gpu:mem:total` | MB | Primary GPU VRAM total |
| `gpu:temp` | °C | Primary GPU temperature |
| `gpu:power` | W | Primary GPU power draw |
| `gpu:0`, `gpu:1` … | % | Per-GPU utilization (multi-GPU) |
| `gpu:0:mem:used` … | MB | Per-GPU VRAM (multi-GPU) |
| `gpu:0:temp` … | °C | Per-GPU temp (multi-GPU) |

### Status Bar Integration

Add GPU and Disk I/O to status bar. Final layout:

```
● 1:05:23 | CPU 12% | GPU 45% | Mem 3.2G/16G | ↑1.2M ↓340K
```

| Slot | Min-width | Shown when | Color thresholds |
|------|-----------|------------|-----------------|
| Uptime | `7ch` | Always | Green dot (running), amber (connecting), red (crashed) |
| CPU | `8ch` | Always | >80% amber, >95% red |
| GPU | `8ch` | GPU data available | >80% amber, >95% red |
| Mem | `14ch` | Always | >90% amber |
| Disk I/O | `12ch` | Always | None (informational) |

- GPU gracefully hidden if no GPU provider detected
- Disk I/O shows read/write rates: `↑1.2M ↓340K` (arrows for write/read)
- All sections separated by `|` at 20% opacity
- Hover tooltip on GPU shows GPU name, VRAM usage, temperature

### Sysinfo Pane Plot Types

Add to `PlotTypes` in `sysinfo-types.ts`:

```typescript
"GPU": ["gpu"],
"GPU Mem": ["gpu:mem:used"],
"GPU + CPU": ["gpu", "cpu"],
"GPU Temp": ["gpu:temp"],
"All GPUs": ["gpu:0", "gpu:1", ...],  // auto-filtered like "All CPU"
```

---

## Part 2: Extended System Metrics

These are metrics `sysinfo` 0.34 already supports but we don't currently collect. Adding them requires only backend changes in `sysinfo.rs` and frontend `PlotTypes` registration.

### 2.1 Disk I/O

**Already available in sysinfo:** `sysinfo::Disks` for space, `sysinfo::System::processes()` for I/O rates.

| Key | Unit | Source |
|-----|------|--------|
| `disk:read` | MB/s | `process.disk_usage().read_bytes` delta |
| `disk:write` | MB/s | `process.disk_usage().written_bytes` delta |
| `disk:total` | MB/s | sum of read + write |
| `disk:used` | GB | `sysinfo::Disks::list()` sum of used |
| `disk:total_space` | GB | `sysinfo::Disks::list()` sum of total |

**Plot types:** `"Disk I/O"`, `"Disk I/O (R/W)"`, `"Disk Space"`

### 2.2 CPU Temperatures

**Available via:** `sysinfo::Components` (reads `hwmon` on Linux, WMI on Windows, IOKit on macOS).

| Key | Unit | Source |
|-----|------|--------|
| `temp:cpu` | °C | First component matching "CPU" / "Core" |
| `temp:cpu:0` … | °C | Per-core temps if available |

**Plot types:** `"CPU Temp"`, `"All Temps"`

**Caveat:** Not all systems expose CPU temps. Windows requires admin or specific WMI providers. Gracefully skip if unavailable.

### 2.3 Swap Memory

**Available via:** `sysinfo::System::total_swap()` / `used_swap()`.

| Key | Unit | Source |
|-----|------|--------|
| `swap:used` | GB | `system.used_swap()` |
| `swap:total` | GB | `system.total_swap()` |

**Plot types:** `"Mem + Swap"` → `["mem:used", "swap:used"]`

### 2.4 Load Average (Unix only)

**Available via:** `sysinfo::System::load_average()`.

| Key | Unit | Source |
|-----|------|--------|
| `load:1` | — | 1-minute load average |
| `load:5` | — | 5-minute load average |
| `load:15` | — | 15-minute load average |

**Plot types:** `"Load Average"` → `["load:1", "load:5", "load:15"]`

Not available on Windows. Skip gracefully.

### 2.5 Per-Core CPU in Status Bar (Optional)

Currently per-core is only in the sysinfo pane plot ("All CPU"). Could add a compact sparkline or mini bar chart to the status bar hover tooltip. Defer to phase 2.

---

## Implementation Plan

### Phase 1: GPU (NVIDIA) + Status Bar (Target: v0.33.x)

| Step | File | Change |
|------|------|--------|
| 1 | `agentmuxsrv-rs/Cargo.toml` | Add `nvml-wrapper = { version = "0.10", optional = true }`, feature `gpu-nvidia` |
| 2 | `agentmuxsrv-rs/src/backend/gpu.rs` | New module: `GpuSnapshot`, `GpuProvider` trait, `NvidiaProvider` |
| 3 | `agentmuxsrv-rs/src/backend/sysinfo.rs` | Initialize GPU provider, collect + publish GPU keys per tick |
| 4 | `frontend/app/statusbar/SystemStats.tsx` | Add GPU % display (conditional on data presence) |
| 5 | `frontend/app/statusbar/StatusBar.scss` | Add `.stat-gpu` min-width |
| 6 | `frontend/app/view/sysinfo/sysinfo-types.ts` | Add GPU plot types + meta |

### Phase 2: Extended Metrics (Target: v0.33.x)

| Step | File | Change |
|------|------|--------|
| 7 | `agentmuxsrv-rs/src/backend/sysinfo.rs` | Collect disk I/O, swap, temperatures, load average |
| 8 | `frontend/app/view/sysinfo/sysinfo-types.ts` | Add plot types for disk, temp, swap, load |
| 9 | `docs/specs/status-bar-redesign.md` | Update spec with new metrics |

### Phase 3: AMD GPU (Target: v0.34.x)

| Step | File | Change |
|------|------|--------|
| 10 | `agentmuxsrv-rs/src/backend/gpu_amd.rs` | AMD sysfs provider (Linux only) |

### Phase 4: Apple Silicon + Intel Arc (Target: v0.35.x+)

Defer until ecosystem has stable Rust bindings.

---

## Design Principles

1. **Feature-gated:** GPU crates behind Cargo features. Default build includes NVIDIA. CI can disable for lighter builds.
2. **Graceful degradation:** Missing driver/hardware = no metrics, no crash. Frontend hides absent data.
3. **No new event types:** GPU and extended metrics pack into the existing `TimeSeriesData` values map. Same WPS event (`sysinfo`), same scope (`local`), same subscriber model.
4. **Consistent naming:** `{category}:{sub}` pattern — `gpu:mem:used`, `disk:read`, `temp:cpu:0`, `swap:used`.
5. **Performance:** GPU queries (NVML, sysfs) are sub-millisecond. No impact on the 1s collection interval. `nvml-wrapper` is non-blocking.
6. **Multi-GPU:** Index by device number. Status bar shows primary (index 0). Sysinfo pane shows all via "All GPUs" plot type.

---

## Testing

1. **NVIDIA present:** GPU % shows in status bar, sysinfo pane has GPU plot type
2. **No GPU / no driver:** Status bar omits GPU section, no errors in logs
3. **Multi-GPU:** Sysinfo pane shows all GPUs, status bar shows primary
4. **AMD Linux:** sysfs provider reads correct paths (test with mock sysfs)
5. **Extended metrics:** Disk I/O, temps, swap all appear in sysinfo plot types
6. **Performance:** Collection interval stays within 1s even with all providers active

---

## Dependencies

| Crate | Version | Size Impact | Purpose |
|-------|---------|-------------|---------|
| `nvml-wrapper` | 0.10+ | ~50KB (links to system NVML DLL at runtime) | NVIDIA GPU metrics |
| `hardware-query` | latest | ~40KB | Cross-platform GPU detection (NVIDIA/AMD/Intel/Apple), good for discovery + fallback |

No new dependencies for: disk I/O, swap, temps, load average, AMD sysfs (all from `sysinfo` or `std::fs`).

### Crate Selection Rationale

- **`nvml-wrapper`**: Production-grade, millions of downloads, wraps official NVIDIA C API. Safe Rust. Serde support. MSRV 1.60. Best-in-class for NVIDIA.
- **`hardware-query`**: Cross-platform GPU detection with CUDA/ROCm/DirectML capability reporting. Use as discovery layer to detect which vendor-specific provider to activate.
- **`amdgpu_top`** (reference): Mature AMD monitoring tool for Linux. We borrow its sysfs approach rather than depending on it directly.
- **WMI (`wmi-rs`)**: NOT recommended for GPU metrics — only returns static info (driver version, device ID), not utilization/temp/memory.
- **`sysinfo`**: Confirmed no GPU support in 0.34+. Excellent for CPU/memory/disk/temp baseline. Provides `Components` for temperature sensors.
- **`systemstat`**: Alternative for disk I/O rates and battery if `sysinfo` gaps are blocking. Lower priority since sysinfo covers most needs.
