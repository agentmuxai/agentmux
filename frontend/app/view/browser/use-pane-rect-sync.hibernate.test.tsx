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
// What the host answers a discard with, and after how long (read when the call is made).
const discardPlan = { ok: true, delayMs: 0 };
vi.mock("@/app/platform/ipc", () => ({
    invokeCommand: (cmd: string, args: any) => {
        calls.push({ cmd, args });
        if (cmd === "browser_pane_discard" && args.op === "discard") {
            return new Promise((resolve) =>
                setTimeout(() => resolve({ supported: true, ok: discardPlan.ok }), discardPlan.delayMs)
            );
        }
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

function Mount(props: { asleep?: () => boolean }) {
    let ph: HTMLDivElement | undefined;
    usePaneRectSync({ model, placeholderRef: () => ph, windowLabel: "main", diag: () => {}, asleep: props.asleep ?? asleep });
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
    discardPlan.ok = true;
    discardPlan.delayMs = 0;
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

    it("a view that unmounted during the park does not discard the page its replacement mounted", async () => {
        await hiddenPane();
        setAsleep(true);
        await tick(100); // parked
        cleanup(); // the view goes away mid-park...
        render(() => <Mount asleep={() => false} />); // ...and the same block mounts again, awake
        await tick(1900);
        expect(discards()).toHaveLength(0);
    });

    it("a wake while a refused discard is out ends the sleep too", async () => {
        discardPlan.ok = false;
        discardPlan.delayMs = 400;
        await hiddenPane();
        setAsleep(true);
        await tick(1700); // the park is over and the discard is on its way
        expect(discards().map((c) => c.args.op)).toEqual(["discard"]);
        setAsleep(false);
        await tick(700); // the discard is answered, refused
        expect(model.endSleep).toHaveBeenCalledTimes(1);
        expect(model.endSleep).toHaveBeenCalledWith(undefined);
        expect(discards().map((c) => c.args.op)).toEqual(["discard"]);
    });
});
