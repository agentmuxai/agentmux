// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Sleeping a browser pane discards its page in place (SPEC_BROWSER_PANE_HIBERNATION_2026_10_20.md §4).
 * A hidden pane is first parked offscreen at its real size for DISCARD_SETTLE_MS, so its scroll position
 * settles before the discard. A wake can land during that park: the page is then alive and never discarded,
 * so nothing loads again and the sleep record would wait for a load that never comes.
 */
import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

const calls: { cmd: string; args: any }[] = [];
vi.mock("@/app/platform/ipc", () => ({
    invokeCommand: (cmd: string, args: any) => {
        calls.push({ cmd, args });
        if (cmd === "browser_pane_discard") return Promise.resolve({ supported: true, ok: true });
        return Promise.resolve();
    },
}));
vi.mock("@/app/workspace/floater-resize", () => ({ FLOATER_BROWSER_EDGE_INSET: 4 }));
const [visibility, setVisibility] = createSignal<"active" | "dormant" | "windowHidden">("active");
vi.mock("@/app/block/pane-tab-visibility", () => ({ usePaneTabVisibility: () => visibility }));
vi.mock("@/app/platform/pane-rect-registry", () => ({ registerPaneRect: () => {}, unregisterPaneRect: () => {} }));
vi.mock("@/app/platform/pane-anim", () => ({ paneReflowActive: () => false, notifyPaneReflow: () => {} }));

import { usePaneRectSync } from "./use-pane-rect-sync";
import { installCefWireHost } from "../../../test/cef-wire-host";

installCefWireHost();

globalThis.ResizeObserver ??= class {
    observe() {}
    disconnect() {}
    unobserve() {}
} as any;

const model = {
    blockId: "hib1",
    closed: false,
    urlAtom: () => "https://agentmux.ai/",
    driverAgentAtom: () => undefined,
    allowedOriginsAtom: () => [],
    identityAtom: () => undefined,
    sleepAtom: () => ({ since: 1, reason: "manual", url: "https://agentmux.ai/", title: "", loadMs: 0 }),
    endSleep: vi.fn(() => Promise.resolve()),
    markSleepKept: vi.fn(() => Promise.resolve()),
    clearSleepKept: vi.fn(() => Promise.resolve()),
} as any;

const [asleep, setAsleep] = createSignal(false);

function Mount() {
    let ph: HTMLDivElement | undefined;
    usePaneRectSync({ model, placeholderRef: () => ph, windowLabel: "main", diag: () => {}, asleep });
    return <div ref={ph} />;
}

const discards = () => calls.filter((c) => c.cmd === "browser_pane_discard");
// Real timers: the hook polls and settles on its own timers, which a fake clock would spin on.
const tick = (ms: number) => new Promise((r) => setTimeout(r, ms));

afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
    calls.length = 0;
    model.endSleep.mockClear();
    model.markSleepKept.mockClear();
    model.clearSleepKept.mockClear();
    setAsleep(false);
    setVisibility("active");
});

/** A mounted, created pane whose tab is then hidden, the way a hibernating pane is. */
async function hiddenPane() {
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({ x: 10, y: 20, width: 300, height: 200 } as DOMRect);
    render(() => <Mount />);
    await tick(10);
    setVisibility("dormant");
    await tick(10);
}

describe("usePaneRectSync — sleep and wake around the discard park", { timeout: 15_000 }, () => {
    it("discards a hidden pane once the park has settled", async () => {
        await hiddenPane();
        setAsleep(true);
        await tick(100);
        expect(discards()).toHaveLength(0);
        await tick(1900);
        expect(discards().map((c) => c.args.op)).toEqual(["discard"]);
        expect(model.markSleepKept).toHaveBeenCalledTimes(1);
        expect(model.endSleep).not.toHaveBeenCalled();
    });

    it("a wake during the park ends the sleep and discards nothing", async () => {
        await hiddenPane();
        setAsleep(true);
        await tick(100);
        setAsleep(false);
        await tick(1900);
        expect(discards()).toHaveLength(0);
        expect(model.endSleep).toHaveBeenCalledTimes(1);
        expect(model.endSleep).toHaveBeenCalledWith(undefined);
    });
});
