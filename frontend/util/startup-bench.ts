// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Startup performance benchmark utility.
// Records named milestones with high-resolution timestamps relative to
// performance.timeOrigin so all phases (tauri-api, bootstrap, wave) share
// the same epoch.
//
// Usage:
//   import { benchMark, benchDump } from "@/util/startup-bench";
//   benchMark("my-phase-start");
//   // ... do work ...
//   benchMark("my-phase-done");
//   benchDump(); // log full timeline

export interface BenchEntry {
    name: string;
    /** ms since performance.timeOrigin (page-load epoch) */
    tsMs: number;
}

const _entries: BenchEntry[] = [];

/**
 * Record a named milestone in the startup timeline.
 * Logged immediately as [startup-bench] so it appears in the host log file.
 */
export function benchMark(name: string): void {
    const now = performance.now();
    _entries.push({ name, tsMs: now });
    const prev = _entries.length > 1 ? _entries[_entries.length - 2].tsMs : 0;
    const delta = prev > 0 ? now - prev : 0;
    const logLine = `[startup-bench] ${now.toFixed(1).padStart(8)}ms  (+${delta.toFixed(1).padStart(7)}ms)  ${name}`;
    console.log(logLine);
    // Forward to backend log if window.api is already installed
    try {
        const api = (window as any).api;
        if (api?.sendLog) api.sendLog(logLine);
    } catch {
        // api not ready yet — the log will still appear in the browser console
        // which is piped to the host log file via initLogPipe
    }
}

/**
 * Dump the full startup timeline to console + backend log.
 * Call once after window.show() for a complete summary.
 */
export function benchDump(): void {
    if (_entries.length === 0) return;
    const total = _entries[_entries.length - 1].tsMs;
    const lines = [
        `[startup-bench] ═══════════════════════════════════════════`,
        `[startup-bench]   Startup Timeline  (${total.toFixed(1)}ms to window-show)`,
        `[startup-bench] ───────────────────────────────────────────`,
    ];
    let prev = 0;
    for (const e of _entries) {
        const from = prev > 0 ? `+${(e.tsMs - prev).toFixed(1)}ms` : "   start";
        lines.push(
            `[startup-bench]  ${e.tsMs.toFixed(1).padStart(8)}ms  (${from.padStart(10)})  ${e.name}`
        );
        prev = e.tsMs;
    }
    lines.push(`[startup-bench] ═══════════════════════════════════════════`);
    const report = lines.join("\n");
    console.log(report);
    try {
        const api = (window as any).api;
        if (api?.sendLog) api.sendLog(report);
    } catch {}
}

/** Return all recorded entries (for tests or custom analysis). */
export function benchGetEntries(): readonly BenchEntry[] {
    return _entries;
}
