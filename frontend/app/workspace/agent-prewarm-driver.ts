// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The window's prewarm scheduler with its real inputs (`store/agent-prewarm.ts`
 * holds the policy). Started by `workspace.tsx`, which owns the window tabs.
 */
import { getApi } from "@/app/store/app-api";
import { createPrewarmScheduler, setPrewarmLease, type Pressure } from "@/app/store/agent-prewarm";
import { tabSwitching } from "@/app/store/tab-reveal";
import { windowResizing } from "@/app/platform/window-resize";
import { scheduleOnSettle } from "@/app/util/settle-detector";
import { getSettingsKeyAtom } from "@/store/global";

/** How long after the last keystroke, click or scroll the window counts as idle. */
export const PREWARM_QUIET_MS = 2000;
const TICK_MS = 3000;

let lastInputAt = performance.now();
let listening = false;
function listenForInput(): void {
    if (listening || typeof document === "undefined") return;
    listening = true;
    const note = () => {
        lastInputAt = performance.now();
    };
    for (const type of ["keydown", "pointerdown", "wheel", "pointermove"]) {
        document.addEventListener(type, note, { passive: true, capture: true });
    }
}

async function hostPressure(): Promise<Pressure> {
    try {
        return (await getApi()?.browserPanes?.memoryPressure?.()) ?? "normal";
    } catch {
        return "normal";
    }
}

export const agentPrewarm = createPrewarmScheduler({
    enabled: () => getSettingsKeyAtom("window:prewarmagenttabs")() === true,
    idle: () =>
        !document.hidden && !tabSwitching() && !windowResizing() && performance.now() - lastInputAt >= PREWARM_QUIET_MS,
    pressure: hostPressure,
    heapBytes: () => (performance as unknown as { memory?: { usedJSHeapSize: number } }).memory?.usedJSHeapSize ?? null,
    lease: setPrewarmLease,
    settle: (done) => {
        let cancelled = false;
        let raf = 0;
        let cancelStage: (() => void) | undefined;
        // A frame first, so the pane has started drawing when the quiet is measured.
        raf = requestAnimationFrame(() => {
            cancelStage = scheduleOnSettle(
                () => {
                    raf = requestAnimationFrame(() => {
                        raf = requestAnimationFrame(() => {
                            if (!cancelled) done();
                        });
                    });
                },
                { settleMs: 150, maxMs: 3000 }
            );
        });
        return () => {
            cancelled = true;
            cancelAnimationFrame(raf);
            cancelStage?.();
        };
    },
    log: (message) => console.info(message),
});

/** Run the scheduler for the life of the window. Returns the stop function. */
export function startAgentPrewarm(): () => void {
    listenForInput();
    const timer = setInterval(() => void agentPrewarm.tick(), TICK_MS);
    return () => clearInterval(timer);
}
