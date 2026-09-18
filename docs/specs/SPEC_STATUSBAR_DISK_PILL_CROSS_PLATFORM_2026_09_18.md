# SPEC: the status bar's Disk pill must not be Windows-only

**Author:** Opaz
**Date:** 2026-09-18
**Status:** proposed

---

## 1. Problem

There is no Disk readout in the status bar on Linux or macOS. Only Windows
shows one.

The pill is gated on a single value (`frontend/app/statusbar/SystemStats.tsx`):

```tsx
<Show when={s().pagefileVolumeFreePct != null}>
```

which is fed by `disk:pagefile_volume:free_pct`. That key is produced only by
`get_pagefile_volume_data` (`agentmux-srv/src/backend/sysinfo.rs:558`), whose
entire body sits inside `#[cfg(target_os = "windows")]`. Off Windows the
function is a no-op, the key never appears, the `Show` never fires, and the
click-open per-drive popover is unreachable because it is rendered inside that
same `Show`.

### 1.1 The data is already there

`get_disk_data` (`sysinfo.rs:919`) runs on **every** platform and already emits,
per mount:

```
disk:vol:<mount>:free_gb
disk:vol:<mount>:total_gb
```

So on Linux the per-volume figures the popover renders are already flowing on
every tick and are simply discarded. What is missing is only the *summary*: a
free-percentage for one designated volume, plus the `disk:vol:<mount>:watch`
and `:is_system_drive` markers that say which volume the percentage refers to.
All three are computed inside the Windows-only function.

### 1.2 Root cause: a concept named after the mechanism that motivated it

Watching a volume's free space is not a Windows idea. The *reason* AgentMux
started watching one was (`SPEC_WIN10_PAGEFILE_OOM_CRASH_2026_06_29` §5.2 P0): a
system-managed page file silently fails to grow when its volume is low, pinning
the commit ceiling below what every other gauge assumes.

That rationale is Windows-specific. The resulting gauge — "free share of the
volume that matters" — is not. Because the concept was named
`pagefile_volume`, the `cfg(windows)` gate travelled with the name, and the
generic half was never given a home.

### 1.3 Observed cost

On 2026-09-17 this instance's root filesystem reached **100% (1.9 GB free)**.
Nothing in the UI indicated it. It surfaced when a packaging build failed, and
the preceding hour of investigation was spent on the build rather than on the
disk. The one widget whose job is to warn about this is the one that does not
render on this platform.

Secondary, and the reason a naive fix is wrong — see §3.2: the colour function
is gated on `systemManaged`, so merely emitting the missing metric would render
a pill on Linux that is **permanently muted**, including at 0.5% free.

---

## 2. Goals / non-goals

**Goals**

1. A Disk pill on Linux and macOS with the same behaviour it has on Windows:
   percentage, colour thresholds, tooltip naming the volume, click-open
   per-drive popover.
2. Preserve Windows behaviour exactly, including the fixed-page-file exception.
3. No new syscall per tick on any platform.

**Non-goals**

- Alerting, notifications, or a memory-pressure-style banner for low disk. The
  pill is a readout; escalation is a separate decision.
- Changing the 8% / 15% thresholds. They came from a Windows incident, but they
  are reasonable generic low-disk brackets and re-deriving them is out of scope.
- Per-volume watching of more than one mount.

---

## 3. Design

### 3.1 Platform-neutral wire keys

Rename the summary keys, which are consumed by the status bar and nothing else
(verified: `disk:pagefile_volume:*` appears only in `SystemStats.tsx` and
`disk-volumes.test.ts`; `agentmux-cef`'s low-memory banner computes its own disk
context via `agentmux_common::pagefile`, not from this payload):

| Before | After |
|---|---|
| `disk:pagefile_volume:free_gb` | `disk:watch:free_gb` |
| `disk:pagefile_volume:total_gb` | `disk:watch:total_gb` |
| `disk:pagefile_volume:free_pct` | `disk:watch:free_pct` |

`disk:pagefile_system_managed` keeps its name: it *is* a page-file fact, it is
Windows-only by nature, and §3.2 depends on its absence being meaningful.

`disk:vol:<mount>:watch` and `disk:vol:<mount>:is_system_drive` keep their
names and meaning, and are now emitted on every platform.

srv and the frontend ship in one artifact, so there is no version skew to
stage: the old keys are removed in the same change rather than dual-written.

### 3.2 Which volume is watched

| Platform | Watch target | Rationale |
|---|---|---|
| Windows | the volume backing the page file | unchanged — `SPEC_WIN10_PAGEFILE_OOM_CRASH_2026_06_29` |
| Unix | the mount backing AgentMux's **data dir** | where filling up actually breaks *this app*: the store, logs, and packaged builds all land there |

Unix selection is a longest-prefix match of the data dir against the mount
points `sysinfo` already enumerated — `/home/yas/...` picks `/home` over `/`
when `/home` is its own filesystem, and falls back to `/`. No syscall: it reads
the `Disks` list already refreshed this tick.

`is_system_drive` keeps its meaning — "this volume is also the OS's own" —
resolving to `%SystemDrive%` on Windows and `/` on Unix. The existing tooltip
wording then works unchanged on both.

### 3.3 The colour gate — `systemManaged` must become tri-state

Today:

```ts
if (freePct == null || !systemManaged) return muted;
```

`systemManaged` is parsed as `(vals["disk:pagefile_system_managed"] ?? 0) > 0`,
so an **absent** key (every non-Windows tick) is indistinguishable from an
explicit `false`. A Linux pill would therefore render and never colour, at any
free percentage — the failure in §1.3 would still have been invisible, just
with a number next to it.

The three states are genuinely different and must be kept apart:

| `systemManaged` | Meaning | Colour |
|---|---|---|
| `true` | Windows, system-managed page file — growth is gated by free space | thresholds apply |
| `false` | Windows, **fixed-size** page file — growth is not gated by free space | never coloured (unchanged) |
| `null` | not Windows — no page file in the model at all | thresholds apply |

So the parse becomes `key in vals ? vals[key] > 0 : null`, and the colour
function suppresses only on an explicit `false`.

### 3.4 Where the colour function lives

`pagefileDiskColor` is private to `SystemStats.tsx` and therefore untested.
`disk-volumes.ts` exists precisely so this logic is testable ("Kept free of
Solid/DOM so the parsing/formatting is unit-testable") and already holds
`diskFreeColor`, the popover's identical 8/15 brackets.

Move it there as `watchVolumeColor(freePct, systemManaged)` and test the three
states. This also puts the two functions that must agree side by side — the
popover row and the pill are supposed to turn amber together.

---

## 4. Test plan

- **Rust, pure:** `pick_watch_mount` — longest-prefix wins over `/`; exact
  mount match; no match falls back to `/`; empty mount list yields `None`.
- **Rust, integration-ish:** on Unix, a tick emits `disk:watch:free_pct` in
  `(0, 100]` and exactly one `disk:vol:*:watch`.
- **TS:** `watchVolumeColor` — the three `systemManaged` states at 5% / 12% /
  50%, and `null` free-pct.
- **TS, existing:** `disk-volumes.test.ts` updated to the new key names.

Manual: launch on Linux, confirm the pill renders, matches `df -h`, opens the
popover, and turns amber/red at the thresholds.

---

## 5. Risks

- **Mount selection on exotic layouts.** Overlay/bind mounts can make several
  entries prefix the data dir. Longest-prefix is deterministic and picks the
  most specific, which is the one whose free space actually constrains writes.
  Worst case the pill names a surprising mount; the popover still lists all.
- **macOS** gets the change for free and is untested on hardware here. The code
  path is the same Unix one; the only platform assumption is `/` as fallback.
- **Windows regression risk** is concentrated in §3.3. A `false` that starts
  colouring would re-introduce the false alarm that the fixed-page-file
  exception exists to prevent, which is why it gets an explicit test.
