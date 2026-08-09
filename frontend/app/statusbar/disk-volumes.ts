// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Pure helpers behind the status bar's Disk readout + per-drive popover.
// Kept free of Solid/DOM so the parsing/formatting is unit-testable
// (same split as memory-pressure-banner's pure fns).

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
 * (SystemStats.tsx::pagefileDiskColor), so a drive that turns the pill
 * amber shows the same amber inside the popover.
 */
export function diskFreeColor(freeGb: number, totalGb: number): string {
    if (totalGb <= 0) return "var(--secondary-text-color)";
    const freePct = (freeGb / totalGb) * 100;
    if (freePct < 8) return "var(--error-color)";
    if (freePct < 15) return "var(--warning-color)";
    return "var(--secondary-text-color)";
}
