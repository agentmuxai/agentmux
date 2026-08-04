# Spec: Virtual Address Space Reduction for CEF Subprocesses

**Status:** Draft (empirically validated)
**Date:** 2026-04-07
**Author:** AgentA
**Affects:** `agentmux-cef` (CEF host), `main.rs`, `app.rs`

---

## Problem Statement

Each CEF subprocess reserves ~100GB of virtual address space (VA) on Windows. A single AgentMux instance (v0.33.58) spawns 4 subprocesses, consuming **401.7GB VA total** while using only **63.4MB committed memory**. The system has 87.7GB total VA ceiling (32GB RAM + 56GB page file), with only **2.9GB free VA** during normal operation — one more subprocess or a second instance will trigger OOM crashes (`0xe0000008`).

---

## Empirical Measurements (2026-04-07)

### Per-Process VA Breakdown

Measured on running AgentMux v0.33.58 portable instance:

| PID   | Process Type      | Virtual (GB) | Committed (MB) | Working Set (MB) |
|-------|-------------------|-------------|-----------------|-------------------|
| 22440 | browser (main)    | 100.56      | 38.3            | 100.9             |
| 33052 | gpu-process       | 100.38      | 7.9             | 23.1              |
| 25052 | utility (network) | 100.40      | 9.2             | 28.1              |
| 2412  | utility (storage) | 100.39      | 8.0             | 17.9              |
| 27500 | agentmux-srv      | 4.17        | 23.7            | 34.2              |
| **Total** | | **405.9 GB** | **87.1 MB** | **204.2 MB** |

**No renderer process is present** — the frontend runs in the browser process (Alloy mode). The 100GB reservation appears in ALL Chromium processes regardless of type.

### System VA Budget

| Metric | Value |
|--------|-------|
| Physical RAM | 31.9 GB |
| Page file | 55.8 GB (system-managed) |
| Total VA ceiling | 87.7 GB |
| Free VA | 2.9 GB |
| CEF VA consumption | 401.7 GB |

The 401.7GB VA exceeds the 87.7GB ceiling by 4.6x. Windows allows this because `MEM_RESERVE` does not count against commit charge — only `MEM_COMMIT` does. The system works until committed memory approaches the ceiling, at which point new reservations or commits fail with OOM.

### VA Memory Map (per subprocess)

Every CEF subprocess shows the **identical** reservation pattern:

```
Region                Size     State    Type
0x????????????0000    32.00 GB RESERVE  PRIVATE   ← PartitionAlloc GigaCage
0x????????????0000    16.00 GB RESERVE  PRIVATE   ← PartitionAlloc regular pool
0x????????????0000    16.00 GB RESERVE  PRIVATE   ← PartitionAlloc BRP pool
0x????????????0000    16.00 GB RESERVE  PRIVATE   ← PartitionAlloc configurable pool
0x????????????0000    16.00 GB RESERVE  PRIVATE   ← PartitionAlloc thread-isolated pool
0x7FF4????????0000     4.00 GB RESERVE  PRIVATE   ← V8 pointer compression cage
0x7FFCA6801000         0.21 GB COMMIT   IMAGE     ← libcef.dll (250 MB)
                     --------
Total:              ~100.2 GB reserved + 0.3 GB committed
```

### What Each Reservation Is

1. **32 GB — PartitionAlloc GigaCage**: The umbrella address space region that houses all PA pools. Reserves 32GB contiguous VA for address masking (fast "is this pointer from PA?" checks via single bitwise AND).

2. **4x 16 GB — PartitionAlloc Pools**: Each pool is a separate VA region:
   - **Regular pool**: General allocations not protected by BackupRefPtr
   - **BRP pool**: Allocations protected by BackupRefPtr (use-after-free mitigation)
   - **Configurable pool**: Used by V8 Sandbox when enabled; still reserved when disabled
   - **Thread-isolated pool**: Per-thread memory with pkey isolation (x64 only)

3. **4 GB — V8 Pointer Compression Cage**: V8 uses pointer compression (storing 32-bit offsets instead of 64-bit pointers). This requires a 4GB region where all V8 heap objects live, so any object can be addressed as `base + 32bit_offset`.

4. **0.21 GB — libcef.dll**: The CEF/Chromium binary itself, memory-mapped.

**Total: 32 + 64 + 4 + 0.21 = ~100.2 GB per subprocess**

### Critical Finding: V8 Sandbox Is Already DISABLED

Despite the earlier hypothesis, the V8 Sandbox (1TB memory cage) is **not the cause** of the 100GB reservation. Evidence:

1. **CEF disables V8 Sandbox since M103**: CEF set `v8_enable_sandbox=false` starting in Chromium 103 because the sandbox broke `CefV8Value::CreateArrayBuffer` (external memory can't be passed into the cage). Source: [CEF issue #3332](https://bitbucket.org/chromiumembedded/cef/issues/3332).

2. **No 1TB region observed**: If the V8 sandbox were enabled, we'd see a single ~1TB `MEM_RESERVE` region. We don't — the largest single region is 32GB.

3. **Non-renderer processes have the same 100GB**: The GPU process and utility processes (which don't run V8/JS at all) also reserve 100GB. This proves the reservation comes from **PartitionAlloc**, which is used by ALL Chromium processes, not from V8.

4. **The `v8-sandbox` string in libcef.dll** is just code/symbol references (the feature detection code exists regardless of whether the feature is compiled in). It does NOT prove the sandbox is enabled.

**The "100GB → 4GB by disabling V8 sandbox" claim from the earlier conversation was wrong.** The V8 sandbox is already disabled. The 100GB comes from PartitionAlloc's pool architecture, which cannot be disabled without rebuilding Chromium with a different allocator.

---

## What Actually Matters

Since the 100GB is from PartitionAlloc (not V8), the mitigation options are different:

### Option 1: `--in-process-gpu` — Eliminate GPU subprocess (saves 100GB)

Merges the GPU process into the browser process. One fewer 100GB subprocess.

```rust
// In app.rs on_before_command_line_processing:
let key = CefString::from("in-process-gpu");
cmd.append_switch(Some(&key));
```

**Impact:** 4 → 3 subprocesses = ~300GB instead of ~400GB.
**Risk:** GPU driver crash kills the whole app instead of just restarting the GPU process. Low risk for a local desktop app.

### Option 2: `--renderer-process-limit=1` — Cap renderer processes

Even though we currently have 0 renderers (Alloy mode), DevTools and popups can spawn them. Capping to 1 prevents runaway renderer spawning.

```rust
let key = CefString::from("renderer-process-limit");
let val = CefString::from("1");
cmd.append_switch_with_value(Some(&key), Some(&val));
```

**Impact:** Prevents future surprise 100GB subprocesses when opening DevTools.
**Risk:** None for single-origin app.

### Option 3: Increase page file (system setting)

**Current:** 56GB page file → 87.7GB VA ceiling
**Target:** 120GB page file → ~152GB VA ceiling

This doesn't reduce per-process VA but raises the ceiling. With `--in-process-gpu` (300GB VA, only ~60MB committed), the system runs comfortably below the commit charge limit.

### Option 4: Custom Chromium/CEF build with reduced PA pool sizes

PartitionAlloc pool sizes are compile-time constants in `partition_alloc_constants.h`. The 16GB per pool and 32GB GigaCage are hardcoded. Reducing them (e.g., to 4GB per pool) would require:
- Building CEF from source (~8 hours on fast hardware)
- Patching PartitionAlloc constants
- Regression testing (PA uses pool size for address masking — smaller pools may break fast-path checks)

**This is the nuclear option.** Not recommended unless Options 1-3 are insufficient.

### Option 5: `--no-untrusted-code-mitigations` — Free performance (no VA impact)

Disables V8 Spectre JIT mitigations. Up to 15% perf improvement for compute-heavy JS. No VA impact, but a free win for trusted content.

```rust
let key = CefString::from("no-untrusted-code-mitigations");
cmd.append_switch(Some(&key));
```

---

## Recommended Plan

1. **Apply Options 1, 2, 5** in `app.rs:225` (4 lines of code total)
2. **Bump, build, measure:** Expected result: 3 subprocesses × 100GB = ~300GB VA, ~50MB committed
3. **Increase page file** to 120GB (system setting) for headroom
4. **Validate:** Run two instances for 4+ hours, monitor VA

**Expected final state:** Two instances = ~600GB VA reserved, ~100MB committed, against a ~152GB commit charge ceiling. Since `MEM_RESERVE` doesn't count against commit charge, this should run indefinitely without OOM.

---

## Verification Script

```powershell
# Check VA per AgentMux subprocess
Get-Process | Where-Object { $_.ProcessName -like '*agentmux*' } |
  Select-Object Id, ProcessName,
    @{N='VA_GB';E={[math]::Round($_.VirtualMemorySize64/1GB,1)}},
    @{N='Commit_MB';E={[math]::Round($_.PrivateMemorySize64/1MB,1)}},
    @{N='WS_MB';E={[math]::Round($_.WorkingSet64/1MB,1)}} |
  Format-Table -AutoSize

# Check system VA headroom
$os = Get-CimInstance Win32_OperatingSystem
"Free VA: {0:F1} GB / {1:F1} GB total" -f ($os.FreeVirtualMemory/1MB), ($os.TotalVirtualMemorySize/1MB)
```

**Success criteria:**
- Single instance: 3 subprocesses (not 4), ~300GB VA, no GPU process
- Two instances: ~600GB VA, > 5GB free VA remaining
- No OOM crashes after 4+ hours

---

## References

- [V8 Sandbox](https://v8.dev/blog/sandbox) — 1TB VA cage (NOT the cause here — disabled in CEF since M103)
- [CEF Issue #3332](https://bitbucket.org/chromiumembedded/cef/issues/3332) — CEF disabled V8 sandbox for ArrayBuffer compatibility
- [VSCode 1TB VA](https://afana.me/archive/2023/06/15/vscode-high-virtual-memory/) — Electron (with V8 sandbox enabled) shows 1TB; CEF does not
- [PartitionAlloc Design](https://chromium.googlesource.com/chromium/src/+/master/base/allocator/partition_allocator/PartitionAlloc.md) — Pool architecture, GigaCage
- [PartitionAlloc Glossary](https://chromium.googlesource.com/chromium/src/+/HEAD/base/allocator/partition_allocator/glossary.md) — Pool types: regular, BRP, configurable, thread-isolated
- [GigaCage Implementation](https://issues.chromium.org/issues/40132577) — Original Chromium issue for GigaCage
- [Chromium OOM Investigation](https://chromium.googlesource.com/chromium/src/+/refs/tags/134.0.6960.0/docs/memory/oom.md) — PartitionAlloc VA overcounting
- [V8 Untrusted Code Mitigations](https://v8.dev/docs/untrusted-code-mitigations) — Spectre JIT mitigations (separate from sandbox)
- [CEF Forum: renderer-process-limit](https://magpcss.org/ceforum/viewtopic.php?p=44500) — Limiting renderer processes
- [CEF Forum: GPU Process](https://www.magpcss.org/ceforum/viewtopic.php?f=6&t=11953) — in-process-gpu switch
- [Chromium Command Line Switches](https://peter.sh/experiments/chromium-command-line-switches/) — Full switch reference
