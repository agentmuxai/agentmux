// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Shared CPU-load color helpers for the status-bar CPU readout and the
 * per-core panel. Kept in its own module so both `SystemStats` and
 * `CpuCoresPopover` can import without a circular dependency.
 * Spec: SPEC_STATUSBAR_CPU_CORES_PANEL_2026_06_15.md.
 */

/**
 * Discrete threshold color — used for the aggregate readout text in the
 * status bar and the panel header. Muted until busy, warning >80, error >95.
 */
export function cpuColor(pct: number): string {
    if (pct > 95) return "var(--error-color)";
    if (pct > 80) return "var(--warning-color)";
    return "var(--secondary-text-color)";
}

/**
 * Continuous idle→busy ramp — used for per-core bar fills and the heatmap
 * squares, where a smooth gradient reads as a "heat" picture (which cores are
 * hot) far better than the 3-step threshold. Ramps idle→warning over 0–50%
 * and warning→error over 50–100%. `--cpu-idle-color` is defined in
 * `_cpu-cores-popover.scss`.
 */
export function loadColor(pct: number): string {
    const p = Math.max(0, Math.min(100, pct));
    if (p <= 50) {
        return `color-mix(in srgb, var(--warning-color) ${Math.round(p * 2)}%, var(--cpu-idle-color))`;
    }
    return `color-mix(in srgb, var(--error-color) ${Math.round((p - 50) * 2)}%, var(--warning-color))`;
}
