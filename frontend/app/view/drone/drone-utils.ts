// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { CronTrigger, DroneTrigger, DroneRunState } from "./drone-types";

// ── Cron human-readable preview ──────────────────────────────────────────────

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const DAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

export function cronToHuman(expr: string): string {
    if (!expr) return "";
    const parts = expr.trim().split(/\s+/);
    if (parts.length !== 5) return expr;
    const [min, hour, dom, month, dow] = parts;

    // Common shortcuts
    if (expr === "* * * * *") return "Every minute";
    if (min !== "*" && hour === "*" && dom === "*" && month === "*" && dow === "*") {
        const m = parseInt(min);
        if (!isNaN(m) && min.startsWith("*/")) return `Every ${min.slice(2)} minutes`;
        if (!isNaN(m)) return `At minute ${m} of every hour`;
    }
    // Every N minutes
    if (min.startsWith("*/") && hour === "*") {
        return `Every ${min.slice(2)} minutes`;
    }
    // Daily at HH:MM
    if (dom === "*" && month === "*") {
        const h = parseInt(hour);
        const m = parseInt(min);
        if (!isNaN(h) && !isNaN(m)) {
            const timeStr = formatTime(h, m);
            if (dow === "*") return `Daily at ${timeStr}`;
            if (dow === "1-5") return `Weekdays at ${timeStr}`;
            if (dow === "6,0" || dow === "0,6") return `Weekends at ${timeStr}`;
            const dayName = parseDow(dow);
            if (dayName) return `${dayName} at ${timeStr}`;
        }
    }
    // Monthly
    if (month === "*" && dow === "*") {
        const h = parseInt(hour);
        const m = parseInt(min);
        const d = parseInt(dom);
        if (!isNaN(d) && !isNaN(h) && !isNaN(m)) {
            return `Monthly on the ${ordinal(d)} at ${formatTime(h, m)}`;
        }
    }
    return expr;
}

function formatTime(h: number, m: number): string {
    const ampm = h >= 12 ? "PM" : "AM";
    const hour12 = h % 12 === 0 ? 12 : h % 12;
    const minStr = m.toString().padStart(2, "0");
    return `${hour12}:${minStr} ${ampm}`;
}

function parseDow(dow: string): string | null {
    const idx = parseInt(dow);
    if (!isNaN(idx) && idx >= 0 && idx <= 6) return DAYS[idx];
    return null;
}

function ordinal(n: number): string {
    if (n === 1) return "1st";
    if (n === 2) return "2nd";
    if (n === 3) return "3rd";
    return `${n}th`;
}

// ── Trigger label ────────────────────────────────────────────────────────────

export function triggerLabel(trigger: DroneTrigger): string {
    switch (trigger.type) {
        case "cron":
            return cronToHuman(trigger.expr);
        case "event":
            return `⚡ ${trigger.eventName}`;
        case "dependency":
            return `↳ on ${trigger.on}`;
        case "manual":
            return "Manual";
        default:
            return "Unknown";
    }
}

export function triggerIcon(trigger: DroneTrigger): string {
    switch (trigger.type) {
        case "cron":      return "⏰";
        case "event":     return "⚡";
        case "dependency": return "↳";
        case "manual":    return "▶";
        default:          return "?";
    }
}

// ── Status helpers ───────────────────────────────────────────────────────────

export function stateColor(state: DroneRunState | "disabled" | null): string {
    switch (state) {
        case "running":    return "var(--success-color, #22c55e)";
        case "retrying":   return "var(--warning-color, #eab308)";
        case "failed":     return "var(--error-color, #ef4444)";
        case "success":    return "var(--success-color, #22c55e)";
        case "queued":     return "var(--accent-color, #818cf8)";
        case "timed_out":  return "#f97316";
        case "cancelled":  return "var(--secondary-text-color)";
        case "disabled":   return "var(--secondary-text-color)";
        default:           return "var(--secondary-text-color)";
    }
}

export function stateIcon(state: DroneRunState | "disabled" | null): string {
    switch (state) {
        case "running":    return "●";
        case "retrying":   return "↺";
        case "failed":     return "✗";
        case "success":    return "✓";
        case "queued":     return "◔";
        case "timed_out":  return "⌛";
        case "cancelled":  return "⊘";
        case "disabled":   return "⊘";
        default:           return "○";
    }
}

export function stateLabel(state: DroneRunState | "disabled" | null): string {
    if (!state) return "idle";
    return state;
}

// ── Time formatting ──────────────────────────────────────────────────────────

export function relativeTime(tsMs: number): string {
    if (!tsMs) return "";
    const diffMs = Date.now() - tsMs;
    const secs = Math.floor(diffMs / 1000);
    if (secs < 60) return `${secs}s ago`;
    const mins = Math.floor(secs / 60);
    if (mins < 60) return `${mins}m ago`;
    const hours = Math.floor(mins / 60);
    if (hours < 24) return `${hours}h ago`;
    const days = Math.floor(hours / 24);
    return `${days}d ago`;
}

export function durationMs(startMs: number, endMs?: number): string {
    const elapsed = (endMs ?? Date.now()) - startMs;
    const secs = Math.floor(elapsed / 1000);
    if (secs < 60) return `${secs}s`;
    const mins = Math.floor(secs / 60);
    const remSecs = secs % 60;
    return `${mins}m ${remSecs}s`;
}
