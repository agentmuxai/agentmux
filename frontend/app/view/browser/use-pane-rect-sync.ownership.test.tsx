// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The native browser page is keyed by block id on the host, but a block's view
 * can mount more than once in quick succession (a split, a layout rebuild, a
 * hot reload). Replays the sequence logged in `task dev`
 * (REPORT_SYSINFO_PLOT_TYPE_AND_BROWSER_PREVIEW_VM_2026_09_25.md §2.4): mount A
 * requests the page and unmounts before the request returns; mount B requests
 * it (the host answers `create-already-live` and B adopts A's page); then A's
 * request returns and A "closed the orphan" — B's page — leaving B black.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

const calls: { cmd: string; args: any }[] = [];
const pendingCreates: (() => void)[] = [];
vi.mock("@/app/platform/ipc", () => ({
    invokeCommand: (cmd: string, args: any) => {
        calls.push({ cmd, args });
        if (cmd === "browser_pane_create") {
            return new Promise<void>((resolve) => pendingCreates.push(resolve));
        }
        return Promise.resolve();
    },
}));
vi.mock("@/app/workspace/floater-resize", () => ({ FLOATER_BROWSER_EDGE_INSET: 4 }));
// The one visibility signal (Pane Tab contract Phase 3), driven by the test.
const [visibility, setVisibility] = createSignal<"active" | "dormant" | "windowHidden">("active");
vi.mock("@/app/block/pane-tab-visibility", () => ({ usePaneTabVisibility: () => visibility }));
vi.mock("@/app/platform/pane-rect-registry", () => ({ registerPaneRect: () => {}, unregisterPaneRect: () => {} }));
vi.mock("@/app/platform/pane-anim", () => ({ paneReflowActive: () => false, notifyPaneReflow: () => {} }));
// The page waits for its pane to hold still before it is made (settled-pane-rect.ts, tested on its own); here a test
// decides what that wait answers. By default it answers at once with the rect as measured.
let settleImpl: ((measure: () => any) => Promise<any>) | null = null;
vi.mock("./settled-pane-rect", () => ({ settledPaneRect: (measure: () => any) => (settleImpl ? settleImpl(measure) : Promise.resolve(measure())) }));

import { usePaneRectSync } from "./use-pane-rect-sync";
import { installCefWireHost } from "../../../test/cef-wire-host";

installCefWireHost();

// jsdom has no ResizeObserver; the hook only needs observe/disconnect.
globalThis.ResizeObserver ??= class {
    observe() {}
    disconnect() {}
    unobserve() {}
} as any;

function Mount(props: { blockId: string; driver?: string; allowed?: string[] }) {
    let ph: HTMLDivElement | undefined;
    const model = {
        blockId: props.blockId,
        setPageLive: () => {},
        closed: false,
        urlAtom: () => "https://agentmux.ai/",
        driverAgentAtom: () => props.driver,
        allowedOriginsAtom: () => props.allowed ?? [],
        identityAtom: () => undefined,
    } as any;
    usePaneRectSync({ model, placeholderRef: () => ph, windowLabel: "main", diag: () => {} });
    return <div ref={ph} />;
}

const closes = () => calls.filter((c) => c.cmd === "browser_pane_close");
const flush = () => new Promise((r) => setTimeout(r, 0));

afterEach(() => {
    cleanup();
    calls.length = 0;
    pendingCreates.length = 0;
    setVisibility("active");
    settleImpl = null;
});

describe("usePaneRectSync — an agent's site limit goes with the page's creation", () => {
    const created = () => calls.find((c) => c.cmd === "browser_pane_create")?.args;

    it("passes the pane's site list while an agent drives it", async () => {
        render(() => <Mount blockId="lim1" driver="lark" allowed={["https://a.example"]} />);
        await flush();
        expect(created()?.allowed_origins).toEqual(["https://a.example"]);
    });

    it("not once the person took it over: the meta key outlives the limit", async () => {
        render(() => <Mount blockId="lim2" allowed={["https://a.example"]} />);
        await flush();
        expect(created()).toBeDefined();
        expect(created()?.allowed_origins).toBeUndefined();
    });
});

describe("usePaneRectSync — the native page follows the tab's visibility", () => {
    // Rects go to the host in batches (pane-rect-batcher.ts): the newest one
    // sent for this block, from whichever command carried it.
    const lastResize = (blockId = "v1") => {
        for (let i = calls.length - 1; i >= 0; i--) {
            const { cmd, args } = calls[i];
            if (cmd === "browser_panes_set_rects") {
                const hit = args.rects.find((r: any) => r.block_id === blockId);
                if (hit) return hit;
            }
            if (cmd === "browser_pane_resize" && args.block_id === blockId) return args;
        }
        return undefined;
    };

    it("collapses the page while its tab is dormant or its window tab is hidden, and restores it", async () => {
        // jsdom lays nothing out; give the placeholder a real rect.
        vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({ x: 10, y: 20, width: 300, height: 200 } as DOMRect);
        render(() => <Mount blockId="v1" />);
        await flush();
        pendingCreates[0]();
        await flush();

        setVisibility("dormant");
        await flush();
        expect(lastResize()).toMatchObject({ width: 0, height: 0 });
        setVisibility("active");
        await flush();
        expect(lastResize()).toMatchObject({ x: 10, y: 20, width: 300, height: 200 });
        setVisibility("windowHidden");
        await flush();
        expect(lastResize()).toMatchObject({ width: 0, height: 0 });
        vi.restoreAllMocks();
    });
});

describe("usePaneRectSync — a block's native page belongs to its latest mount", () => {
    it("a mount whose create returns after it unmounted does not close a newer mount's page", async () => {
        const a = render(() => <Mount blockId="b1" />);
        await flush();
        expect(pendingCreates).toHaveLength(1);
        a.unmount();
        render(() => <Mount blockId="b1" />);
        await flush();
        expect(pendingCreates).toHaveLength(2);

        pendingCreates[1](); // B: host says already live, B adopts the page
        pendingCreates[0](); // A's create returns after A unmounted
        await flush();

        expect(closes()).toHaveLength(0);
    });

    it("still closes the orphan when no newer mount wants the page", async () => {
        const a = render(() => <Mount blockId="b2" />);
        await flush();
        a.unmount();
        pendingCreates[0]();
        await flush();

        expect(closes()).toHaveLength(1);
    });

    it("an older mount's unmount does not close the page a newer mount now owns", async () => {
        const a = render(() => <Mount blockId="b3" />);
        await flush();
        pendingCreates[0]();
        await flush();
        render(() => <Mount blockId="b3" />); // newer mount claims the page
        await flush();
        pendingCreates[1]();
        await flush();
        a.unmount();

        expect(closes()).toHaveLength(0);
    });

    it("the owning mount's unmount closes its page", async () => {
        const a = render(() => <Mount blockId="b4" />);
        await flush();
        pendingCreates[0]();
        await flush();
        a.unmount();

        expect(closes()).toHaveLength(1);
    });
});

describe("usePaneRectSync — the page is made at its pane's final size", () => {
    const created = () => calls.find((c) => c.cmd === "browser_pane_create")?.args;

    it("makes nothing while its pane is still taking its place, then makes it at the settled rect", async () => {
        vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({ x: 0, y: 0, width: 686, height: 900 } as DOMRect);
        let settle!: (r: { x: number; y: number; width: number; height: number }) => void;
        settleImpl = () => new Promise((resolve) => (settle = resolve));
        render(() => <Mount blockId="s1" />);
        await flush();
        expect(created()).toBeUndefined();
        settle({ x: 0, y: 0, width: 1586, height: 900 });
        await flush();
        expect(created()).toMatchObject({ block_id: "s1", width: 1586, height: 900 });
        vi.restoreAllMocks();
    });

    it("makes nothing at all when the view went before its pane settled", async () => {
        let settle!: (r: { x: number; y: number; width: number; height: number }) => void;
        settleImpl = () => new Promise((resolve) => (settle = resolve));
        const a = render(() => <Mount blockId="s2" />);
        await flush();
        a.unmount();
        settle({ x: 0, y: 0, width: 1586, height: 900 });
        await flush();
        expect(created()).toBeUndefined();
        expect(closes()).toHaveLength(0);
    });
});
