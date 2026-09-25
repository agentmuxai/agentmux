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
vi.mock("@/app/workspace/floater-resize", () => ({ FLOATER_EDGE_RESIZE_BORDER: 4 }));
// The one visibility signal (Pane Tab contract Phase 3), driven by the test.
const [visibility, setVisibility] = createSignal<"active" | "dormant" | "windowHidden">("active");
vi.mock("@/app/block/pane-tab-visibility", () => ({ usePaneTabVisibility: () => visibility }));
vi.mock("@/app/platform/pane-rect-registry", () => ({ registerPaneRect: () => {}, unregisterPaneRect: () => {} }));
vi.mock("@/app/platform/pane-anim", () => ({ paneReflowActive: () => false, notifyPaneReflow: () => {} }));

import { usePaneRectSync } from "./use-pane-rect-sync";

// jsdom has no ResizeObserver; the hook only needs observe/disconnect.
globalThis.ResizeObserver ??= class {
    observe() {}
    disconnect() {}
    unobserve() {}
} as any;

function Mount(props: { blockId: string }) {
    let ph: HTMLDivElement | undefined;
    const model = { blockId: props.blockId, closed: false, urlAtom: () => "https://agentmux.ai/" } as any;
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
});

describe("usePaneRectSync — the native page follows the tab's visibility", () => {
    const lastResize = () => calls.filter((c) => c.cmd === "browser_pane_resize").at(-1)?.args;

    it("collapses the page while its tab is dormant or its window tab is hidden, and restores it", async () => {
        // jsdom lays nothing out; give the placeholder a real rect.
        vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({ x: 10, y: 20, width: 300, height: 200 } as DOMRect);
        render(() => <Mount blockId="v1" />);
        pendingCreates[0]();
        await flush();

        setVisibility("dormant");
        expect(lastResize()).toMatchObject({ width: 0, height: 0 });
        setVisibility("active");
        expect(lastResize()).toMatchObject({ x: 10, y: 20, width: 300, height: 200 });
        setVisibility("windowHidden");
        expect(lastResize()).toMatchObject({ width: 0, height: 0 });
        vi.restoreAllMocks();
    });
});

describe("usePaneRectSync — a block's native page belongs to its latest mount", () => {
    it("a mount whose create returns after it unmounted does not close a newer mount's page", async () => {
        const a = render(() => <Mount blockId="b1" />);
        expect(pendingCreates).toHaveLength(1);
        a.unmount();
        render(() => <Mount blockId="b1" />);
        expect(pendingCreates).toHaveLength(2);

        pendingCreates[1](); // B: host says already live, B adopts the page
        pendingCreates[0](); // A's create returns after A unmounted
        await flush();

        expect(closes()).toHaveLength(0);
    });

    it("still closes the orphan when no newer mount wants the page", async () => {
        const a = render(() => <Mount blockId="b2" />);
        a.unmount();
        pendingCreates[0]();
        await flush();

        expect(closes()).toHaveLength(1);
    });

    it("an older mount's unmount does not close the page a newer mount now owns", async () => {
        const a = render(() => <Mount blockId="b3" />);
        pendingCreates[0]();
        await flush();
        render(() => <Mount blockId="b3" />); // newer mount claims the page
        pendingCreates[1]();
        await flush();
        a.unmount();

        expect(closes()).toHaveLength(0);
    });

    it("the owning mount's unmount closes its page", async () => {
        const a = render(() => <Mount blockId="b4" />);
        pendingCreates[0]();
        await flush();
        a.unmount();

        expect(closes()).toHaveLength(1);
    });
});
