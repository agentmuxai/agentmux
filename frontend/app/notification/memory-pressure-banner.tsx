// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * MemoryPressureBanner — a non-modal, dismissible, app-wide banner that warns
 * the user when the host detects low memory, BEFORE the cliff.
 * SPEC_MEMORY_PRESSURE_SUPERVISION_2026_06_16 §5.F,
 * SPEC_RAM_PAGEFILE_PRESSURE_SPLIT_2026_08_07.
 *
 * The host (`memory_heartbeat` → `mem_pressure`, #1494) emits a `memory-pressure`
 * event carrying a `kind` ("ram" | "pagefile") and the debounced level on every
 * transition; this renders a warning (Warn) / error (Critical) banner and clears
 * it when the level returns to normal. It is **purely informational** — the user
 * (closing a window or another app) is the most reliable lever; the banner never
 * touches renderers or windows.
 *
 * RAM and Page File pressure are two independently-tracked signals (originally
 * one combined "commit charge" signal mislabeled "System memory" — see the
 * 2026-08-07 spec for why that was wrong): a machine can be tight on RAM with a
 * healthy page file, or vice versa. Mount one `<MemoryPressureBanner kind="ram" />`
 * and one `<MemoryPressureBanner kind="pagefile" />`; each has its own level and
 * dismiss state and only reacts to events carrying its own `kind`, so both can
 * show at once if both are true.
 *
 * Dismiss is sticky per *severity*: dismissing at Warn keeps it hidden until
 * pressure escalates to Critical or a fresh episode begins (level returns to
 * Normal then rises again), so it can't nag while the user is already acting.
 *
 * The show/dismiss decision is a pure function (`shouldShow`) so it's unit-tested
 * without a renderer. End-to-end the banner can be exercised from DevTools:
 *   window.dispatchEvent(new CustomEvent('agentmux-event',
 *     { detail: { event: 'memory-pressure', payload: { kind: 'pagefile', level: 'warn' } } }))
 */

import { createSignal, onCleanup, onMount, Show } from "solid-js";
import { listenEvent } from "@/app/platform/ipc";
import "./memory-pressure-banner.scss";

export type PressureLevel = "normal" | "warn" | "critical";
export type PressureKind = "ram" | "pagefile";
type ActiveLevel = Exclude<PressureLevel, "normal">;

interface MemoryPressurePayload {
    kind: PressureKind;
    level: PressureLevel;
    // "ram" payloads carry this.
    phys_free_mb?: number;
    // "pagefile" payloads carry these three.
    commit_free_mb?: number;
    system_managed?: boolean;
    disk_free_pct?: number;
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

const RAM_MESSAGE: Record<ActiveLevel, string> = {
    warn: "System RAM is running low. Performance may degrade; closing some windows or apps will help.",
    critical:
        "System RAM is critically low. Closing some windows or other apps will keep AgentMux responsive.",
};

const PAGEFILE_MESSAGE: Record<ActiveLevel, string> = {
    warn: "Virtual memory (page file) is running low.",
    critical:
        "Virtual memory (page file) is critically low — an out-of-memory crash is imminent.",
};

/** Disk-free % below which a system-managed page file is treated as
 *  practically stuck even though Windows would like to grow it — matches the
 *  ~15-20% free-disk framing in SPEC_WIN10_PAGEFILE_OOM_CRASH_2026_06_29 §5.2. */
const PAGEFILE_DISK_LOW_PCT = 20;

/** The disk/OS-managed-aware guidance appended to the Page File message —
 *  the "may the OS handle it, or not" distinction from
 *  SPEC_RAM_PAGEFILE_PRESSURE_SPLIT_2026_08_07 §4. Empty string if the
 *  host didn't report disk context (e.g. a read failure) — no guidance is
 *  better than a guess. */
export function pagefileGuidance(systemManaged?: boolean, diskFreePct?: number): string {
    if (systemManaged === false) {
        return " Your page file has a fixed size and won't grow automatically — free up disk space or increase its size in Windows settings.";
    }
    if (systemManaged === true && diskFreePct !== undefined && diskFreePct < PAGEFILE_DISK_LOW_PCT) {
        return " Windows can't grow your page file because disk space is low — free up disk space now to avoid a crash.";
    }
    if (systemManaged === true) {
        return " Windows can expand virtual memory automatically, but performance may dip in the meantime.";
    }
    return "";
}

/** The full banner text for a given kind/level/payload — RAM never has
 *  disk/OS-managed guidance (that concept doesn't apply to physical RAM). */
export function messageFor(kind: PressureKind, level: ActiveLevel, payload: MemoryPressurePayload): string {
    if (kind === "ram") return RAM_MESSAGE[level];
    return PAGEFILE_MESSAGE[level] + pagefileGuidance(payload.system_managed, payload.disk_free_pct);
}

interface MemoryPressureBannerProps {
    kind: PressureKind;
}

export const MemoryPressureBanner = (props: MemoryPressureBannerProps) => {
    const [level, setLevel] = createSignal<PressureLevel>("normal");
    // The level at which the user last dismissed; "normal" = not dismissed.
    const [dismissedAt, setDismissedAt] = createSignal<PressureLevel>("normal");
    const [payload, setPayload] = createSignal<MemoryPressurePayload>({ kind: props.kind, level: "normal" });

    onMount(() => {
        let unsub: (() => void) | undefined;
        void listenEvent<MemoryPressurePayload>("memory-pressure", (p) => {
            if (!p || p.kind !== props.kind) return;
            const next: PressureLevel = p.level ?? "normal";
            setLevel(next);
            setPayload(p);
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
                    {level() === "critical"
                        ? messageFor(props.kind, "critical", payload())
                        : messageFor(props.kind, "warn", payload())}
                </span>
                <button
                    class="memory-pressure-banner-dismiss"
                    type="button"
                    title="Dismiss"
                    aria-label={props.kind === "ram" ? "Dismiss low-RAM warning" : "Dismiss low-page-file warning"}
                    onClick={() => setDismissedAt(level())}
                >
                    ×
                </button>
            </div>
        </Show>
    );
};

MemoryPressureBanner.displayName = "MemoryPressureBanner";
