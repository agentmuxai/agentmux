// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Pure helpers behind the status bar's Disk readout + per-drive popover.
// Kept free of Solid/DOM so the parsing/formatting is unit-testable
// (same split as backend-uptime.ts and backend-dot.ts).

import { t } from "@/app/i18n/i18n";

export interface DiskVolume {
    /** Display label — mount point with a trailing backslash trimmed ("C:" from "C:\"). */
    label: string;
    freeGb: number;
    totalGb: number;
    /** True for the volume the status-bar Disk % refers to (the backend's watch target). */
    isWatch: boolean;
    /** True when the watch volume is also %SystemDrive% — usually true, but a
     *  custom PagingFiles entry can place the page file on a different
     *  volume. Only meaningful when `isWatch` is true. */
    isSystemDrive: boolean;
}

// Matches disk:vol:<mount>:<field> — the mount itself may contain colons and
// backslashes ("C:\"), so the field suffix anchors the parse and the mount is
// whatever sits between the fixed prefix and the last-colon suffix.
const VOL_KEY = /^disk:vol:(.+):(free_gb|total_gb|watch|is_system_drive)$/;

/** Trim a mount point for display: "C:\" → "C:", "/home/" stays "/home", "/" stays "/". */
function displayLabel(mount: string): string {
    if (mount.length > 1 && (mount.endsWith("\\") || mount.endsWith("/"))) {
        return mount.slice(0, -1);
    }
    return mount;
}

/**
 * Parse the sysinfo event's `disk:vol:*` keys into a sorted volume list.
 * Volumes missing either free or total (torn/partial tick) are dropped
 * rather than rendered with a fabricated 0.
 */
export function parseDiskVolumes(vals: Record<string, number>): DiskVolume[] {
    const partial = new Map<string, { freeGb?: number; totalGb?: number; isWatch?: boolean; isSystemDrive?: boolean }>();
    for (const key in vals) {
        const m = VOL_KEY.exec(key);
        if (!m) continue;
        const mount = m[1];
        const entry = partial.get(mount) ?? {};
        if (m[2] === "free_gb") entry.freeGb = vals[key];
        else if (m[2] === "total_gb") entry.totalGb = vals[key];
        else if (m[2] === "watch") entry.isWatch = (vals[key] ?? 0) > 0;
        else entry.isSystemDrive = (vals[key] ?? 0) > 0;
        partial.set(mount, entry);
    }
    const out: DiskVolume[] = [];
    for (const [mount, e] of partial) {
        if (e.freeGb == null || e.totalGb == null || e.totalGb <= 0) continue;
        out.push({
            label: displayLabel(mount),
            freeGb: e.freeGb,
            totalGb: e.totalGb,
            isWatch: !!e.isWatch,
            isSystemDrive: !!e.isSystemDrive,
        });
    }
    out.sort((a, b) => a.label.localeCompare(b.label));
    return out;
}

/**
 * The free space as the status bar's Disk pill words it: "45GB", "8.4GB", "1.2TB". Whole GB from 10 GB up, one decimal
 * under 10 GB, and TB (one decimal) from 1000 GB. `freeGb` is srv's `disk:watch:free_gb`, which is counted in 1024-based
 * units (2^30 bytes) and called GB, as the low-disk notice (`formatFreeGb`) calls it; the TB step divides by 1024 like
 * `formatDiskGb`, so the bar, the drive panel and the notice always agree.
 */
export function formatDiskFreeShort(freeGb: number): string {
    const gb = Math.max(0, freeGb);
    const tb = (): string => `${(gb / 1024).toFixed(1)}TB`;
    if (gb >= 1000) return tb();
    if (gb >= 9.95) {
        const whole = Math.round(gb);
        return whole >= 1000 ? tb() : `${whole}GB`;
    }
    return `${gb.toFixed(1)}GB`;
}

/**
 * The words of the status bar's Disk pill: "Disk 13%/45GB free", or "Disk 13% free" where the free bytes are not
 * known (a host that reports only a percent). The figure is the FREE share of the watched volume, so the word is on
 * the pill: a bare "Disk 98%" reads as 98% used, and a nearly empty disk reads as healthy the other way round.
 * The colour (watchVolumeColor) turns on LOW free space, as before.
 */
export function diskPillLabel(freePct: number, freeGb?: number | null): string {
    const pct = Math.round(freePct);
    if (freeGb == null || !Number.isFinite(freeGb)) return t("statusBar.diskFree", { pct });
    return t("statusBar.diskFreeAmount", { pct, free: formatDiskFreeShort(freeGb) });
}

/** Free-space size for humans: 1.5T / 320.4G / 512M. */
export function formatDiskGb(gb: number): string {
    if (gb >= 1024) return `${(gb / 1024).toFixed(1)}T`;
    if (gb >= 1) return `${gb.toFixed(1)}G`;
    return `${Math.round(gb * 1024)}M`;
}

/**
 * Tooltip for the status-bar Disk % readout. Short and descriptive like the
 * other stat tooltips ("Per-core CPU usage", "Memory used and total", ...):
 * names what the % is a share OF, and nothing else. Deliberately says
 * nothing about the page file — that lives in the PF gauge's own tooltip,
 * not here. Live figures belong in the click-open panel, not the hover.
 *
 * "System drive (C:)" only when the watched volume actually IS
 * %SystemDrive% (the common case). A custom PagingFiles entry can place
 * the page file on a different volume than the system drive (e.g.
 * `E:\pagefile.sys 0 0` with `SystemDrive=C`) — "system drive (E:)" would
 * be self-contradictory there, so that case just names the drive plainly
 * instead of asserting it's the system drive (reagent finding on #2479).
 */
export function diskTooltip(volumes: DiskVolume[]): string {
    const watch = volumes.find((v) => v.isWatch);
    if (!watch) return "Free share of system drive";
    if (watch.isSystemDrive) return `Free share of system drive (${watch.label})`;
    return `Free share of ${watch.label}`;
}

/**
 * Row color by free share: red under 8% free, amber under 15%, muted
 * otherwise — same brackets the Disk readout itself uses
 * (watchVolumeColor below), so a drive that turns the pill amber shows the
 * same amber inside the popover.
 */
export function diskFreeColor(freeGb: number, totalGb: number): string {
    if (totalGb <= 0) return "var(--secondary-text-color)";
    const freePct = (freeGb / totalGb) * 100;
    if (freePct < 8) return "var(--error-text-color)";
    if (freePct < 15) return "var(--warning-text-color)";
    return "var(--secondary-text-color)";
}

/**
 * Color for the status-bar Disk pill.
 *
 * Thresholds come from SPEC_WIN10_PAGEFILE_OOM_CRASH_2026_06_29 §5.2: the
 * commit gauge sees only the SYMPTOM (commit near its limit) and is blind to
 * the cause that spec found — a system-managed page file wants to auto-grow
 * toward min(3xRAM, 1/8 volume) and silently cannot when its volume is low,
 * pinning the commit ceiling below what every other gauge assumes. <15% free
 * is that spec's documented crash-risk line; <8% is where its source incident
 * actually crashed (20.1 GB / 446 GB), used here as the less alarmist error
 * line. They double as sane generic low-disk brackets, which is why the
 * non-Windows pill reuses them unchanged.
 *
 * `systemManaged` is deliberately TRI-STATE, and conflating two of its values
 * is a real bug rather than a style point
 * (SPEC_STATUSBAR_DISK_PILL_CROSS_PLATFORM_2026_09_18 §3.3):
 *
 *   true  — Windows, system-managed page file: growth IS gated by free space.
 *   false — Windows, FIXED-size page file: growth is not gated this way, so
 *           coloring it would be a false alarm. Never colored.
 *   null  — not Windows: there is no page file in the model at all, so the
 *           exception does not apply and the thresholds do.
 *
 * Parsing an absent `disk:pagefile_system_managed` as `false` (what
 * `?? 0 > 0` used to do) collapses null into false, which would render a
 * permanently-muted Linux pill — a disk at 0.5% free showing in the same
 * color as one at 90%.
 */
export function watchVolumeColor(freePct: number | null, systemManaged: boolean | null): string {
    if (freePct == null) return "var(--secondary-text-color)";
    if (systemManaged === false) return "var(--secondary-text-color)";
    if (freePct < 8) return "var(--error-text-color)";
    if (freePct < 15) return "var(--warning-text-color)";
    return "var(--secondary-text-color)";
}
