// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * MemoryPressureBanner — a non-modal, dismissible, app-wide banner that warns
 * the user when the host detects low system memory, BEFORE the cliff.
 * SPEC_MEMORY_PRESSURE_SUPERVISION_2026_06_16 §5.F.
 *
 * The host (`memory_heartbeat` → `mem_pressure`, #1494) emits a `memory-pressure`
 * event carrying the debounced level on every transition; this renders a warning
 * (Warn) / error (Critical) banner and clears it when the level returns to
 * normal. It is **purely informational** — the user (closing a window or another
 * app) is the most reliable lever; the banner never touches renderers or windows.
 *
 * Dismiss is sticky per *severity*: dismissing at Warn keeps it hidden until
 * pressure escalates to Critical or a fresh episode begins (level returns to
 * Normal then rises again), so it can't nag while the user is already acting.
 *
 * The show/dismiss decision is a pure function (`shouldShow`) so it's unit-tested
 * without a renderer. End-to-end the banner can be exercised from DevTools:
 *   window.dispatchEvent(new CustomEvent('agentmux-event',
 *     { detail: { event: 'memory-pressure', payload: { level: 'warn' } } }))
 */

import { createSignal, onCleanup, onMount, Show } from "solid-js";
import { listenEvent } from "@/app/platform/ipc";
import "./memory-pressure-banner.scss";

export type PressureLevel = "normal" | "warn" | "critical";

interface MemoryPressurePayload {
    level: PressureLevel;
    commit_free_mb?: number;
}

/** Ordinal severity, so escalation (warn→critical) can re-show a dismissed banner. */
export function severity(level: PressureLevel): number {
    return level === "critical" ? 2 : level === "warn" ? 1 : 0;
}

/** Show the banner iff there is pressure AND it is more severe than the level the
 *  user last dismissed at. (`dismissedAt === "normal"` means "not dismissed".) */
export function shouldShow(level: PressureLevel, dismissedAt: PressureLevel): boolean {
    return severity(level) > 0 && severity(level) > severity(dismissedAt);
}

const MESSAGE: Record<Exclude<PressureLevel, "normal">, string> = {
    warn: "System memory is running low. Closing some windows or other applications will keep AgentMux responsive.",
    critical:
        "System memory is critically low — an out-of-memory crash is imminent. Close some windows or other apps now to keep your work safe.",
};

export const MemoryPressureBanner = () => {
    const [level, setLevel] = createSignal<PressureLevel>("normal");
    // The level at which the user last dismissed; "normal" = not dismissed.
    const [dismissedAt, setDismissedAt] = createSignal<PressureLevel>("normal");

    onMount(() => {
        let unsub: (() => void) | undefined;
        void listenEvent<MemoryPressurePayload>("memory-pressure", (p) => {
            const next: PressureLevel = p?.level ?? "normal";
            setLevel(next);
            // A return to Normal ends the episode → re-arm so the next episode
            // shows even if the user had dismissed the previous one.
            if (next === "normal") setDismissedAt("normal");
        }).then((u) => {
            unsub = u;
        });
        onCleanup(() => unsub?.());
    });

    const visible = () => shouldShow(level(), dismissedAt());

    return (
        <Show when={visible()}>
            <div
                class={`memory-pressure-banner memory-pressure-banner--${level()}`}
                role="status"
                aria-live="polite"
            >
                <span class="memory-pressure-banner-icon" aria-hidden="true">
                    {level() === "critical" ? "⚠" : "▲"}
                </span>
                <span class="memory-pressure-banner-text">
                    {level() === "critical" ? MESSAGE.critical : MESSAGE.warn}
                </span>
                <button
                    class="memory-pressure-banner-dismiss"
                    type="button"
                    title="Dismiss"
                    aria-label="Dismiss low-memory warning"
                    onClick={() => setDismissedAt(level())}
                >
                    ×
                </button>
            </div>
        </Show>
    );
};

MemoryPressureBanner.displayName = "MemoryPressureBanner";
