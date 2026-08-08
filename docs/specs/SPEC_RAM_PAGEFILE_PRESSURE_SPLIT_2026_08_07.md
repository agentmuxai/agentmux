# Split the low-memory banner into independent RAM and Page File warnings

**Date:** 2026-08-07
**Status:** Draft — design proposal, not yet implemented
**Affected:** `agentmux-cef` (memory heartbeat, pressure classifier, banner emit) and
`frontend` (banner component), Windows only.

> **Read first — this builds on prior work, it does not replace it:**
> - `docs/specs/SPEC_MEMORY_PRESSURE_SUPERVISION_2026_06_16.md` — the original
>   `PressureTracker`/banner design this spec splits in two.
> - `docs/specs/SPEC_WIN10_PAGEFILE_OOM_CRASH_2026_06_29.md` — established that free
>   disk on the pagefile volume, and whether the pagefile is system-managed, gates
>   whether Windows can actually grow it. §5.2 P0 ("track free disk on the pagefile
>   volume ... warn when free disk is low *and* page file is system-managed") is the
>   item this spec finally wires up into a user-facing signal.
> - `docs/specs/SPEC_MEMORY_COMMIT_ATTRIBUTION_CORRECTION_2026_07_02.md` and
>   `docs/retro/retro-commit-restart-reclaim-2026-07-16.md` — background on why the
>   pressure classifier is ratio-based, not absolute-MB.
>
> **Already shipped** (confirmed in tree, reused as-is by this spec): `PressureTracker`
> + hysteresis classifier (`agentmux-cef/src/memory_pressure.rs`); commit-free/total
> sampling every 20s (`agentmux-cef/src/memory_heartbeat.rs`); the `memory-pressure`
> banner (`frontend/app/notification/memory-pressure-banner.tsx`); pagefile-volume free
> disk + system-managed detection (`agentmux-srv/src/backend/sysinfo.rs::get_pagefile_volume_data`,
> `pagefile_watch_target`, `read_paging_files_registry`) — currently cosmetic
> StatusBar telemetry only, not wired into any warning.

---

## 1. The bug

The banner text says **"System memory is critically low"**, but the signal driving it
is `commit_free_mb` / `commit_total_mb` — Windows' **commit charge**, which is
`physical RAM + page file` combined (`GlobalMemoryStatusEx`'s `ullAvailPageFile` /
`ullTotalPageFile`, read in `memory_heartbeat.rs`). On a machine with plenty of free
RAM but a page file pinned near its ceiling (exactly the failure mode
`SPEC_WIN10_PAGEFILE_OOM_CRASH_2026_06_29.md` documented), this banner fires and says
"system memory" when the actual constraint is virtual-memory/page-file headroom — the
user goes looking for a RAM problem that isn't there.

There is currently **one** tracker, **one** classifier instance, and **one** banner
message for a combined metric that conflates two different resources with two
different remediations:

- **RAM low, commit healthy:** Windows pages more aggressively — the machine slows
  down, but the page file absorbs the overflow. Not urgent; closing apps helps
  performance, not survival.
- **Commit (RAM + page file) low:** an allocation can fail outright — the crash mode
  `SPEC_WIN10_PAGEFILE_OOM_CRASH_2026_06_29.md §2` documented
  (`0xE0000008`/`base::TerminateBecauseOutOfMemory`) or a silent Windows OOM-kill. This
  is the genuinely urgent one, and today's banner is *only* this signal, mislabeled.

## 2. Goal

Split into two independently-tracked, independently-worded signals:

1. **RAM pressure** — physical memory free/total (`ullAvailPhys`/`ullTotalPhys`),
   softer framing ("performance may degrade").
2. **Page File pressure** — commit free/total (today's existing signal, correctly
   relabeled), sharper framing ("crash risk"), **extended with the pagefile-volume
   disk-space + system-managed signal already computed in `agentmux-srv/src/backend/sysinfo.rs`**
   so the message correctly reflects whether Windows can self-heal by growing the page
   file, or is structurally stuck (§4).

Both trackers reuse the existing `PressureTracker`/`classify()` machinery unchanged —
this is a signal-split and a wiring change, not a new classifier design.

## 3. RAM tracker — new, parallel to the existing commit tracker

`memory_heartbeat.rs`'s `log_memory_stats()` already computes `avail_phys_gb` /
`total_phys_gb` from the same `GlobalMemoryStatusEx` call it uses for the page-file
numbers (lines 195-196) — it just never publishes or acts on them. No new syscall is
needed, only two new atomics alongside the existing `COMMIT_FREE_MB`/`COMMIT_TOTAL_MB`:

```rust
static PHYS_FREE_MB: AtomicU64 = AtomicU64::new(u64::MAX);
static PHYS_TOTAL_MB: AtomicU64 = AtomicU64::new(0);

pub fn phys_free_mb() -> u64 { PHYS_FREE_MB.load(Ordering::Relaxed) }
pub fn phys_total_mb() -> u64 { PHYS_TOTAL_MB.load(Ordering::Relaxed) }
```

`log_memory_stats()` stores into these two alongside its existing `COMMIT_FREE_MB`
store (line 204) — same `mem` struct, zero extra cost. `start()`'s tick loop
instantiates a second `PressureTracker` (`ram_pressure`, alongside the existing
`commit_pressure` — rename `pressure` for clarity) and calls
`ram_pressure.observe(phys_free_mb(), phys_total_mb())` right after the existing
`commit_pressure.observe(...)` call.

**Thresholds:** start with the same ratios the commit tracker already uses (Warn <15%
free, Critical <5% free, 3-point hysteresis) rather than inventing new unvalidated
numbers. `PressureTracker`'s thresholds are currently module-level `const`s shared by
every instance — this spec makes them a field on the tracker
(`PressureThresholds { warn_enter, critical_enter, hysteresis }`, with a `::default()`
matching today's values) so RAM and Page File can diverge later once there's real
signal to tune against, without another classifier rewrite.

**Note on severity semantics:** RAM running low is not equally dangerous as commit
running low (§1) — that distinction belongs in the *banner copy and framing*, not in
different Warn/Critical ratios. Keeping the ratios equal for now avoids conflating "we
picked a different number" with "we know this number is right"; nothing here has been
measured against real low-RAM-but-healthy-commit machines yet.

## 4. Page File tracker — existing commit tracker, extended with the disk/OS-managed signal

This is the "may the OS handle it, or not" part. `agentmux-srv/src/backend/sysinfo.rs`
already computes, once per tick, exactly what's needed and currently only feeds the
cosmetic StatusBar gauge:

- `disk:pagefile_volume:free_gb` / `free_pct` — free disk space on the drive backing
  the page file (`drive_free_total_gb`, via `GetDiskFreeSpaceExW`).
- `disk:pagefile_system_managed` — whether Windows controls the page file's size
  (`pagefile_watch_target` / `read_paging_files_registry`, reading
  `HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Memory Management\PagingFiles`).
  `initial == 0 && maximum == 0` for an entry means system-managed; no entries at all
  means "fully auto," which also resolves to system-managed.

This matters because the *meaning* of "page file commit is tight" depends entirely on
whether the OS can respond:

| System-managed? | Free disk on pagefile volume | What it means | Message tone |
|---|---|---|---|
| Yes | Healthy (≥ ~20% free / several GB) | Windows can grow the page file on demand — pressure may self-resolve | Softer: "Windows can expand virtual memory automatically, but performance may dip." |
| Yes | Low | **The one `SPEC_WIN10_PAGEFILE_OOM_CRASH_2026_06_29` documented** — Windows *wants* to grow it but physically can't | Sharpest: "Your page file can't grow because disk space is low. Free up disk space now to avoid a crash." |
| No (fixed size) | n/a | Windows will **never** grow it — the ceiling is a hard, known limit | Sharp, immediate: "Your page file has a fixed size and won't grow. Free up disk space or increase its size in Windows settings." |

**Wiring:** `memory_heartbeat.rs` needs its own copy of the disk-free + system-managed
check — `agentmux-cef` and `agentmux-srv` are separate processes with no shared
channel for `sysinfo.rs`'s per-tick `HashMap` today, and duplicating a single
`GlobalMemoryStatusEx` call between the two crates already has precedent (`commit_free_mb()`
here vs. `get_commit_data()` in `sysinfo.rs`, independently implemented since
`SPEC_MEMORY_PRESSURE_SUPERVISION_2026_06_16`). But the registry-parsing +
disk-free logic is ~120 lines, not one syscall — copy-pasting it risks exactly the kind
of silent two-pipeline drift `memory_pressure.rs`'s own doc comment already flags as a
past bug (issue #2218, the ratio-vs-absolute-MB mismatch between the classifier and
`SystemStats.tsx`). **Recommended: extract `pagefile_watch_target` /
`read_paging_files_registry` / `drive_free_total_gb` / `PagingFileEntry` from
`agentmux-srv/src/backend/sysinfo.rs` into `agentmux-common`** (currently has no
Windows-specific module — this would be its first) and have both `sysinfo.rs` and the
new `memory_heartbeat.rs` code call the shared version. Lower-effort fallback if
`agentmux-common` churn is out of scope right now: duplicate it, but leave a comment
in both places pointing at each other so a future drift is at least discoverable by
grep.

The registry read is already `OnceLock`-cached in `sysinfo.rs` (only re-read once per
process lifetime — the registry doesn't change without a reboot); the disk-free check
(`GetDiskFreeSpaceExW`) is cheap enough to run every heartbeat tick (20s) or can be
decoupled to a slower cadence (e.g. every 5th tick, ~100s) if profiling shows otherwise
— not expected to matter at this frequency.

The Page File `PressureTracker`'s Warn/Critical *entry* thresholds stay exactly as
today (commit free-ratio 0.15/0.05) — the disk/system-managed signal does not change
*when* it fires, only *what it says* once it has. Concretely: `PressureLevel` from
`classify()` is combined with a `PageFileContext { system_managed: bool, disk_free_pct: f64 }`
snapshot at emit time to select which of the three message variants above to send.

## 5. Frontend banner changes

`MemoryPressurePayload` gains a `kind: "ram" | "pagefile"` field, and for `pagefile`
payloads, the `system_managed` / `disk_free_pct` needed to pick a message variant:

```ts
interface MemoryPressurePayload {
    kind: "ram" | "pagefile";
    level: PressureLevel;
    // RAM: informational only today.
    phys_free_mb?: number;
    // Page File: drives which of the 3 message variants renders.
    commit_free_mb?: number;
    system_managed?: boolean;
    disk_free_pct?: number;
}
```

`MemoryPressureBanner` becomes two independent instances (`<MemoryPressureBanner kind="ram" />`
/ `<MemoryPressureBanner kind="pagefile" />`, or one component keyed by `kind` with its
own `level`/`dismissedAt` signals per instance) — RAM and Page File pressure are
uncorrelated enough that both could be true at once (e.g. ample RAM, starved commit)
and the user needs to see both, not have one silently overwrite the other. Each keeps
its own sticky-per-severity dismiss state, unchanged from today's logic
(`shouldShow`/`severity` are reused verbatim, they're already generic over "a pressure
level," not RAM/commit-specific).

`MESSAGE` becomes a `kind → level → string` map. Example copy (not final, needs a
copy pass, but demonstrates the 2×2(+1) shape from §3/§4):

```ts
const MESSAGE = {
  ram: {
    warn: "System RAM is running low. Performance may degrade; closing some windows or apps will help.",
    critical: "System RAM is critically low. Closing some windows or other apps will keep AgentMux responsive.",
  },
  pagefile: {
    warn: "Virtual memory (page file) is running low.",
    critical: "Virtual memory (page file) is critically low — an out-of-memory crash is imminent.",
  },
} as const;

// pagefile critical + !system_managed → append the fixed-size guidance from §4's table
// pagefile critical + system_managed + low disk_free_pct → append the "can't grow, free disk now" guidance
// pagefile critical + system_managed + healthy disk_free_pct → append the "expanding automatically" guidance
```

## 6. Non-goals

- **Non-Windows.** `commit_free_mb()`/`phys_free_mb()` are already `u64::MAX`-stubbed
  on non-Windows (feature is effectively Windows-only today); this spec keeps that
  scope, consistent with `SPEC_WIN10_PAGEFILE_OOM_CRASH_2026_06_29`'s own framing
  ("why Windows 11 is unaffected" — the underlying issue is Windows-specific commit
  accounting, not a cross-platform concern).
- **Changing Warn/Critical entry ratios.** Both trackers start on the existing
  0.15/0.05 commit-derived ratios (§3); tuning RAM-specific thresholds is future work
  once there's real data to tune against, not part of this split.
- **The `0xE0000008` gated-recovery question** flagged as open in
  `SPEC_WIN10_PAGEFILE_OOM_CRASH_2026_06_29.md §5.2` (P0, "confirm caught by gated
  renderer recovery") — orthogonal to the banner; not addressed here.
- **Proactive shedding on RAM pressure.** Today only commit pressure triggers pane-pool
  eviction/refill-suppression (`memory_heartbeat.rs` lines ~99-104,
  `commands/window_pool.rs`). This spec does not extend shedding to the new RAM
  tracker — RAM pressure alone (with healthy commit) doesn't risk a crash the way
  commit pressure does, so there's no clear case for shedding on it yet. Worth
  revisiting once the new banner has been live long enough to know how often RAM-only
  episodes actually happen.

## 7. Recommendations

| Pri | Item | Why | Where |
|-----|------|-----|-------|
| **P0** | Publish `PHYS_FREE_MB`/`PHYS_TOTAL_MB` atomics + a second `PressureTracker` instance for RAM, wired parallel to the existing commit tracker. | Zero-cost (numbers already computed each tick); this is the core of the split. | `agentmux-cef/src/memory_heartbeat.rs` |
| **P0** | Make `PressureTracker` thresholds a field (`PressureThresholds`), not module consts, defaulting to today's values. | Lets RAM and Page File diverge later without another classifier rewrite; §3. | `agentmux-cef/src/memory_pressure.rs` |
| **P0** | Wire `disk:pagefile_volume:*` + `disk:pagefile_system_managed` (already computed in `sysinfo.rs`) into the Page File banner's message selection. | This is the literal P0 item `SPEC_WIN10_PAGEFILE_OOM_CRASH_2026_06_29 §5.2` asked for and that never got a user-facing consumer — currently StatusBar-only. | New code in `agentmux-cef`, extracting shared logic per §4 |
| **P1** | Extract `pagefile_watch_target`/`read_paging_files_registry`/`drive_free_total_gb`/`PagingFileEntry` into `agentmux-common`. | Avoids a second independently-drifting copy of ~120 lines of registry/disk logic — the exact class of bug issue #2218 already caused once between two pipelines computing "the same" ratio differently. | `agentmux-common` (new module), `agentmux-srv/src/backend/sysinfo.rs`, `agentmux-cef/src/memory_heartbeat.rs` |
| **P1** | Split `MemoryPressureBanner` into per-`kind` instances with independent dismiss state and a `kind`-aware `MESSAGE` map (§5). | RAM and Page File pressure are uncorrelated; today's single banner can only ever show one framing at a time. | `frontend/app/notification/memory-pressure-banner.tsx` |
| **P2** | Relabel the existing StatusBar `commitColor` gauge tooltip/label if it currently says "memory" anywhere ambiguous, for consistency with the banner's new correct wording. | Not blocking, but leaving the StatusBar using old wording while the banner uses new wording would be a fresh (smaller) version of the same mislabeling bug this spec fixes. | `frontend/app/statusbar/SystemStats.tsx` |

## 8. Verification

- **Unit tests** (mirrors `memory_pressure.rs`'s existing `#[cfg(test)]` module):
  independent transitions for two trackers fed different free/total pairs; confirm
  `PressureThresholds` defaults reproduce every existing test's expected transitions
  bit-for-bit (regression safety on the const → field refactor).
- **`system_managed` classification:** unit tests against `resolve_pagefile_watch_target`
  (already covers empty/system-managed/fixed-size cases per existing `sysinfo.rs`
  logic) — confirm the shared/extracted version behaves identically.
- **Manual — RAM-only pressure (commit healthy):** on a machine with a large page
  file, open enough apps to drop free physical RAM under 15%/5% while commit stays
  comfortably above its own thresholds. Expect only the RAM banner to appear, correctly
  worded, no Page File banner.
- **Manual — Page File pressure, system-managed, healthy disk:** artificially cap free
  disk on the pagefile volume to a value still comfortably above ~20% free while commit
  is tight. Expect the softer "expanding automatically" copy.
- **Manual — Page File pressure, system-managed, low disk (the original bug):**
  reproduce `SPEC_WIN10_PAGEFILE_OOM_CRASH_2026_06_29`'s repro (free C: down to ~20 GB)
  and confirm the banner now says "page file" / "virtual memory," not "system memory,"
  and includes the "can't grow, free disk now" guidance.
- **Manual — fixed-size page file:** set an explicit fixed-size page file in Windows
  settings, drop commit into Warn/Critical. Expect the "fixed size, won't grow"
  guidance regardless of free disk.
