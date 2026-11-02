// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

const UNITS = ["B", "KB", "MB", "GB", "TB"];

/**
 * The one way the app writes a size: "812 B", "812 KB", "1.8 MB", "2.5 GB", "1.0 TB". Base 1024 with the
 * plain unit names (a "KB" here is 1024 bytes), so a file shows the same size in the Files pane, a download,
 * the history store, a tooltip and the Tower. One decimal under 10 KB and from MB up, whole numbers between. A value
 * that is not a size (negative, NaN) is "0 B"; a caller that wants a dash for "unknown" checks before calling.
 */
export function formatBytes(bytes: number): string {
    if (!Number.isFinite(bytes) || bytes < 0) return "0 B";
    let value = bytes;
    let unit = 0;
    while (value >= 1024 && unit < UNITS.length - 1) {
        value /= 1024;
        unit++;
    }
    // A few KB keep a decimal ("4.8 KB"): a whole number is too coarse for a small file.
    const decimals = unit >= 2 || (unit === 1 && value < 9.95);
    let text = decimals ? value.toFixed(1) : String(Math.round(value));
    // 1023.9 KB rounds to 1024: that is 1.0 MB.
    if (Number(text) >= 1024 && unit < UNITS.length - 1) {
        text = (Number(text) / 1024).toFixed(1);
        unit++;
    }
    return `${text} ${UNITS[unit]}`;
}
