// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Human-readable byte size, 1024-based: "812 B", "812 KB", "1.8 MB", "1.0 GB".
 * Whole numbers below MB, one decimal from MB up.
 */
export function formatBytes(bytes: number): string {
    if (!Number.isFinite(bytes) || bytes < 0) return "0 B";
    const KB = 1024;
    const MB = KB * 1024;
    const GB = MB * 1024;
    if (bytes >= GB) return `${(bytes / GB).toFixed(1)} GB`;
    if (bytes >= MB) return `${(bytes / MB).toFixed(1)} MB`;
    if (bytes >= KB) return `${Math.round(bytes / KB)} KB`;
    return `${Math.round(bytes)} B`;
}
