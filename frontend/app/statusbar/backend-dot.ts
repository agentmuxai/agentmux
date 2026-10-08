// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The status bar's backend dot: its colour and tooltip, as pure functions.
// REPORT_PERFORMANCE_INDICATORS_TO_STATUS_BAR_2026_10_08 §3.4.

import type { HealthLevel } from "@/app/store/health-signals";

/** The dot's colour: srv's connection state, and yellow while srv runs with
 *  a performance signal active (warn or critical alike; red stays "down"). */
export function statusDotColor(status: string | null, worst: HealthLevel): string | null {
    switch (status) {
        case "running":
            return worst === "normal" ? "var(--accent-color)" : "var(--warning-color)";
        case "connecting":
            return "var(--warning-color)";
        case "crashed":
            return "var(--error-color)";
        default:
            return null;
    }
}

/** The trigger's tooltip, naming the active signals. */
export function statusDotTip(labels: string[]): string {
    return labels.length
        ? `Backend status: ${labels.join(", ")}. Click for details`
        : "Backend status, click for details";
}
