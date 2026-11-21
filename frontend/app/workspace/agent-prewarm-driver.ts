// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The window's prewarm scheduler with its real inputs (`store/agent-prewarm.ts`
 * holds the policy). Started by `workspace.tsx`, which owns the window tabs.
 */
import { getApi } from "@/util/app-api";
import { createPrewarmScheduler, setPrewarmLease, type Pressure } from "@/app/store/agent-prewarm";
import { intentLine, setSwitchReporter, tabIntentId } from "@/app/store/tab-intent";
import { tabSwitching } from "@/app/store/tab-reveal";
import { windowResizing } from "@/app/platform/window-resize";
import { scheduleOnSettle } from "@/app/util/settle-detector";
import { getSettingsKeyAtom } from "@/store/global";
import { createEffect, createRoot, on } from "solid-js";

/** How long after the last keystroke, click or scroll the window counts as idle. */
export const PREWARM_QUIET_MS = 2000;
const TICK_MS = 3000;
/** A draw on intent starts no pane within this long of a keystroke: typing in the displayed tab must not stall. */
export const INTENT_KEY_QUIET_MS = 500;

let lastInputAt = performance.now();
let lastKeyAt = Number.NEGATIVE_INFINITY;
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
    document.addEventListener(
        "keydown",
        () => {
            lastKeyAt = performance.now();
        },
        { passive: true, capture: true }
    );
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
    // A hover is a pointer move, so the intent draw does not wait for the pointer to be still.
    intentEnabled: () => getSettingsKeyAtom("window:drawtabonhover")() !== false,
    intentIdle: () =>
        !document.hidden && !tabSwitching() && !windowResizing() && performance.now() - lastKeyAt >= INTENT_KEY_QUIET_MS,
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
    // The panes of the tab the pointer rests on draw ahead of the switch (store/tab-intent.ts).
    setSwitchReporter((tabId, lead) => {
        const { drawn, of } = agentPrewarm.tabDrawn(tabId);
        console.info(intentLine(lead, drawn, of));
    });
    const disposeIntent = createRoot((dispose) => {
        createEffect(on(tabIntentId, (tabId) => agentPrewarm.intent(tabId)));
        return dispose;
    });
    return () => {
        clearInterval(timer);
        disposeIntent();
        setSwitchReporter(null);
        agentPrewarm.intent(null);
    };
}
