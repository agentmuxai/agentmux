// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The zoom amount names the part it belongs to while a pane's parts are on screen ("File tree 130%"), and
 * is announced through a live region that is always mounted.
 * docs/specs/SPEC_EDITOR_TREE_AND_EDITOR_SEPARATE_ZOOM_2026_11_16.md §5 cases 14 and 15.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const viewTypes = new Map<string, string>();
vi.mock("@/app/store/global", () => ({ MOS: { makeORef: (t: string, i: string) => `${t}:${i}`, getObjectValue: () => ({ meta: {} }) } }));
vi.mock("@/app/store/block-component-registry", () => ({
    getBlockComponentModel: (id: string) => (viewTypes.has(id) ? { viewModel: { viewType: viewTypes.get(id) } } : undefined),
    getFocusedBlockId: () => undefined,
    getAllBlockComponentModelEntries: () => [],
}));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { SetMetaCommand: () => Promise.resolve(undefined) } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/i18n/i18n", () => ({
    t: (key: string) => ({ "zoom.part.fileTree": "File tree", "zoom.part.editor": "Editor" })[key] ?? key,
}));

// A placement of the amount is a call of zoomIndicatorCenter: counted, and run as it is.
const placements = vi.hoisted(() => ({ n: 0 }));
vi.mock("./zoom-indicator-position", async (importOriginal) => {
    const real = await importOriginal<typeof import("./zoom-indicator-position")>();
    return {
        ...real,
        zoomIndicatorCenter: (...args: Parameters<typeof real.zoomIndicatorCenter>) => {
            placements.n++;
            return real.zoomIndicatorCenter(...args);
        },
    };
});
import { stubPaneTab } from "@/app/block/pane-tab-test-utils";
import { registerPaneTab } from "@/app/store/pane-tab-registry";
import { showZoomIndicator } from "@/app/store/zoom";
import { subjectId } from "@/app/store/zoom-parts";
import { EDITOR_ZOOM } from "@/app/view/editor/editor-zoom-parts";
import { render } from "solid-js/web";
import { ZoomIndicator } from "./zoomindicator";

registerPaneTab(stubPaneTab("editor", { capabilities: { paneZoom: EDITOR_ZOOM } }));

let dispose: () => void;
let host: HTMLElement;

beforeEach(() => {
    viewTypes.set("e1", "editor");
    host = document.createElement("div");
    document.body.append(host);
    dispose = render(() => <ZoomIndicator />, host);
});

afterEach(() => {
    dispose();
    document.body.innerHTML = "";
    viewTypes.clear();
});

/** An Editor pane in the DOM, with or without its file tree. */
function mountPane(withTree: boolean): void {
    const pane = document.createElement("div");
    pane.setAttribute("data-blockid", "e1");
    pane.innerHTML = `${withTree ? `<div data-zoom-part="tree"></div>` : ""}<div data-zoom-part="main"></div>`;
    document.body.append(pane);
}

describe("ZoomIndicator", () => {
    it("is a polite live region from the start, and says the amount with the part's name", () => {
        const live = host.querySelector<HTMLElement>(".zoom-indicator-live");
        expect(live?.getAttribute("role")).toBe("status");
        expect(live?.getAttribute("aria-live")).toBe("polite");
        expect(live?.textContent).toBe("");

        mountPane(true);
        showZoomIndicator("130%", subjectId("e1", "tree"));
        expect(live?.textContent).toBe("File tree 130%");
        expect(host.querySelector(".zoom-indicator-content")?.textContent).toBe("File tree 130%");

        showZoomIndicator("110%", "e1");
        expect(live?.textContent).toBe("Editor 110%");
    });

    it("shows the bare amount when the pane has no other part on screen", () => {
        mountPane(false);
        showZoomIndicator("110%", "e1");
        expect(host.querySelector(".zoom-indicator-content")?.textContent).toBe("110%");
        expect(host.querySelector(".zoom-indicator-live")?.textContent).toBe("110%");
    });
});

/** Lets Solid run the effects a change queued. */
const tick = () => new Promise<void>((r) => setTimeout(r, 0));

describe("ZoomIndicator placement", () => {
    it("is measured once a frame, never inside the wheel tick that changed the amount", async () => {
        const frames: Array<() => void> = [];
        vi.stubGlobal("requestAnimationFrame", (cb: () => void) => frames.push(cb));
        vi.stubGlobal("cancelAnimationFrame", () => {});
        try {
            mountPane(false);
            showZoomIndicator("100%", "e1");
            await tick();
            for (const frame of frames.splice(0)) frame(); // the first amount is placed; the gesture starts from rest
            placements.n = 0;

            // Three ticks of one gesture in one frame: nothing is measured inside them.
            showZoomIndicator("120%", "e1");
            showZoomIndicator("130%", "e1");
            showZoomIndicator("140%", "e1");
            await tick();
            expect(placements.n).toBe(0);
            expect(host.querySelector(".zoom-indicator-content")?.textContent).toBe("140%");
            // The next frame measures once for all three.
            expect(frames).toHaveLength(1);
            frames[0]();
            expect(placements.n).toBe(1);
        } finally {
            vi.unstubAllGlobals();
        }
    });
});
