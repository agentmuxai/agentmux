// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A pane whose parts zoom apart (the Editor's file tree and its document): which part a wheel or a key
 * zooms, that each part keeps its own level, the first-start seed, all-panes zoom and the indicator's name.
 * docs/specs/SPEC_EDITOR_TREE_AND_EDITOR_SEPARATE_ZOOM_2026_11_16.md.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

let blockMetas: Map<string, Record<string, unknown>>;
let blockViewTypes: Map<string, string>;
let focused: string | undefined;

vi.mock("@/app/store/global", () => ({
    MOS: {
        makeORef: (type: string, id: string) => `${type}:${id}`,
        getObjectValue: (oref: string) => ({ meta: blockMetas.get(oref.slice(oref.indexOf(":") + 1)) ?? {} }),
    },
}));
vi.mock("@/app/store/block-component-registry", () => ({
    getBlockComponentModel: (blockId: string) => {
        const vt = blockViewTypes.get(blockId);
        return vt === undefined ? undefined : { viewModel: { viewType: vt } };
    },
    getFocusedBlockId: () => focused,
    getAllBlockComponentModelEntries: () => Array.from(blockViewTypes.entries()).map(([id, vt]) => [id, { viewModel: { viewType: vt } }]),
}));
const setMetaMock = vi.fn((..._args: unknown[]): Promise<undefined> => {
    const opts = _args[1] as { oref: string; meta: Record<string, unknown> };
    const id = opts.oref.slice(opts.oref.indexOf(":") + 1);
    blockMetas.set(id, { ...(blockMetas.get(id) ?? {}), ...opts.meta });
    return Promise.resolve(undefined);
});
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { SetMetaCommand: (...args: unknown[]) => setMetaMock(...args) } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { stubPaneTab } from "@/app/block/pane-tab-test-utils";
import { EDITOR_ZOOM, treeHasFocus } from "@/app/view/editor/editor-zoom-parts";
import { registerPaneTab } from "@/app/store/pane-tab-registry";
import { setPlatform } from "@/util/platformutil";
import { paneZoom, resetPaneZoomForTests } from "./pane-zoom-local";
import {
    zoomAllPanesIn,
    zoomAllPanesReset,
    zoomBlockByWheel,
    zoomBlockIn,
    zoomIn,
    zoomIndicatorLabelAtom,
    zoomIndicatorTargetAtom,
    zoomOut,
    zoomPaneReset,
    zoomReset,
} from "./zoom";
import { paneZoomPatch, setZoomMax } from "./zoom-factor";
import {
    focusIsInPart,
    resetZoomPartFocusForTests,
    splitSubject,
    subjectId,
    zoomLabelKey,
    zoomPartsSeed,
    zoomSubjectAt,
    zoomSubjectForKeys,
    zoomSubjectsOf,
} from "./zoom-parts";

registerPaneTab(stubPaneTab("editor", { capabilities: { paneZoom: EDITOR_ZOOM } }));
registerPaneTab(stubPaneTab("term", { capabilities: { paneZoom: {} } }));

const TREE = subjectId("e1", "tree");
const level = (id: string, key = "term:zoom"): number => (blockMetas.get(id)?.[key] as number | undefined) ?? 1;

/** The pane's DOM: `<div data-blockid><div data-zoom-part=tree>…</div><div data-zoom-part=main>…</div></div>`. */
function mountPane(id: string, opts: { tree?: boolean } = {}): { tree: HTMLElement | null; main: HTMLElement; header: HTMLElement } {
    const pane = document.createElement("div");
    pane.setAttribute("data-blockid", id);
    const header = document.createElement("div");
    pane.append(header);
    let tree: HTMLElement | null = null;
    if (opts.tree !== false) {
        tree = document.createElement("div");
        tree.setAttribute("data-zoom-part", "tree");
        tree.innerHTML = `<button id="row">row</button>`;
        pane.append(tree);
    }
    const main = document.createElement("div");
    main.setAttribute("data-zoom-part", "main");
    main.innerHTML = `<textarea id="text"></textarea>`;
    pane.append(main);
    document.body.append(pane);
    return { tree, main, header };
}

function setBlock(id: string, viewType: string, meta: Record<string, unknown> = {}): void {
    blockViewTypes.set(id, viewType);
    // term:fontsize pinned: a 5% step changes the font size by a pixel, so no step is skipped.
    blockMetas.set(id, { "term:fontsize": 20, ...meta });
}

beforeEach(() => {
    resetPaneZoomForTests();
    resetZoomPartFocusForTests();
    blockMetas = new Map();
    blockViewTypes = new Map();
    focused = undefined;
    document.body.innerHTML = "";
    setMetaMock.mockClear();
    setPlatform("win32");
});
afterEach(() => vi.clearAllMocks());

describe("subjects", () => {
    it("names the pane's own level by its block id and a part by block id and part", () => {
        expect(subjectId("e1")).toBe("e1");
        expect(subjectId("e1", "main")).toBe("e1");
        expect(subjectId("e1", "tree")).toBe("e1#tree");
        expect(splitSubject("e1#tree")).toEqual({ blockId: "e1", partId: "tree" });
        expect(splitSubject("e1")).toEqual({ blockId: "e1", partId: undefined });
    });

    it("lists every level of a pane: its own, then its parts", () => {
        setBlock("e1", "editor");
        setBlock("t1", "term");
        expect(zoomSubjectsOf("e1")).toEqual(["e1", "e1#tree"]);
        expect(zoomSubjectsOf("t1")).toEqual(["t1"]);
    });
});

describe("which part a pointer zooms", () => {
    it("is the part under the pointer, and the pane's own level anywhere else in it", () => {
        setBlock("e1", "editor");
        const { tree, main, header } = mountPane("e1");
        expect(zoomSubjectAt(tree!.querySelector("button"), "e1")).toBe(TREE);
        expect(zoomSubjectAt(tree, "e1")).toBe(TREE);
        expect(zoomSubjectAt(main.querySelector("textarea"), "e1")).toBe("e1");
        expect(zoomSubjectAt(header, "e1")).toBe("e1");
    });

    it("is the pane's level in a pane that declares no parts", () => {
        setBlock("t1", "term");
        const { tree } = mountPane("t1");
        expect(zoomSubjectAt(tree, "t1")).toBe("t1");
    });

    it("zooms only the part under the pointer, with Ctrl+wheel", () => {
        setPlatform("win32");
        setBlock("e1", "editor");
        const { tree, main } = mountPane("e1");
        zoomBlockByWheel(zoomSubjectAt(tree, "e1"), -100);
        expect([level("e1"), level("e1", "editor:tree_zoom")]).toEqual([1, 1.05]);
        zoomBlockByWheel(zoomSubjectAt(main, "e1"), -100);
        zoomBlockByWheel(zoomSubjectAt(main, "e1"), -100);
        expect(level("e1")).toBeCloseTo(1.1, 5);
        expect(level("e1", "editor:tree_zoom")).toBe(1.05);
    });
});

describe("which part the keys zoom", () => {
    const focusIn = (el: Element) => el.dispatchEvent(new FocusEvent("focusin", { bubbles: true }));

    it("is the part that has the keyboard, and the pane's own level for a pane that never had it", () => {
        setBlock("e1", "editor");
        const { tree, main } = mountPane("e1");
        expect(zoomSubjectForKeys("e1")).toBe("e1");
        focusIn(tree!.querySelector("button")!);
        expect(zoomSubjectForKeys("e1")).toBe(TREE);
        focusIn(main.querySelector("textarea")!);
        expect(zoomSubjectForKeys("e1")).toBe("e1");
    });

    it("stays on the last part when focus moves out of the pane (its header, a menu, the palette)", () => {
        setBlock("e1", "editor");
        const { tree, header } = mountPane("e1");
        focusIn(tree!.querySelector("button")!);
        focusIn(header);
        expect(zoomSubjectForKeys("e1")).toBe(TREE);
    });

    it("is the pane's own level once the tree is hidden, and the tree's again when it is shown", () => {
        setBlock("e1", "editor");
        const { tree } = mountPane("e1");
        focusIn(tree!.querySelector("button")!);
        tree!.remove();
        expect(zoomSubjectForKeys("e1")).toBe("e1");
    });

    it("zooms with the keys the part that has the keyboard, and resets only that part", () => {
        setBlock("e1", "editor", { "term:zoom": 1.2, "editor:tree_zoom": 1.2 });
        focused = "e1";
        const { tree } = mountPane("e1");
        focusIn(tree!.querySelector("button")!);
        zoomIn();
        expect(level("e1", "editor:tree_zoom")).toBeGreaterThan(1.2);
        expect(level("e1")).toBe(1.2);
        zoomReset();
        expect(level("e1", "editor:tree_zoom")).toBe(1);
        expect(level("e1")).toBe(1.2);
        zoomOut();
        expect(level("e1", "editor:tree_zoom")).toBeLessThan(1);
    });

    it("tells whether the keyboard is in a part, so a pane that takes focus on a click can leave it there", () => {
        setBlock("e1", "editor");
        const { tree, main } = mountPane("e1");
        const button = tree!.querySelector("button")!;
        button.focus();
        expect(focusIsInPart("e1", "tree")).toBe(true);
        expect(treeHasFocus("e1")).toBe(true);
        main.querySelector("textarea")!.focus();
        expect(focusIsInPart("e1", "tree")).toBe(false);
        expect(focusIsInPart("e1", "main")).toBe(true);
        // A part that is not on screen has nothing in it.
        tree!.remove();
        expect(treeHasFocus("e1")).toBe(false);
    });

    it("zooms a named part from the palette whatever has the keyboard, and resets both together", () => {
        setBlock("e1", "editor", { "term:zoom": 1.2, "editor:tree_zoom": 1.3 });
        focused = "e1";
        mountPane("e1");
        zoomIn(undefined, "tree");
        expect(level("e1", "editor:tree_zoom")).toBeGreaterThan(1.3);
        expect(level("e1")).toBe(1.2);
        zoomPaneReset();
        expect([level("e1"), level("e1", "editor:tree_zoom")]).toEqual([1, 1]);
    });
});

describe("a part's level", () => {
    it("is saved under its own key, with 100% as 1 so it differs from never set; the pane's own stays unset", () => {
        expect(paneZoomPatch(1, "e1#tree", "editor:tree_zoom")).toEqual({ "editor:tree_zoom": 1 });
        expect(paneZoomPatch(1.234, "e1#tree", "editor:tree_zoom")).toEqual({ "editor:tree_zoom": 1.23 });
        expect(paneZoomPatch(1, "e1")).toEqual({ "term:zoom": null });
    });

    it("is held under its own subject, so the pane's and the tree's held levels never overwrite each other", () => {
        setBlock("e1", "editor");
        zoomBlockIn("e1", 0.1);
        zoomBlockIn(TREE, 0.2);
        expect(paneZoom("e1", {})).toBeCloseTo(1.1, 5);
        expect(paneZoom(TREE, {}, "editor:tree_zoom")).toBeCloseTo(1.2, 5);
    });

    it("keeps the shared 50% to 200% range, and never takes the raised top a PDF gave the document", () => {
        setBlock("e1", "editor", { "editor:tree_zoom": 1.98 });
        const undo = setZoomMax("e1", 4);
        zoomBlockIn(TREE, 0.5);
        expect(level("e1", "editor:tree_zoom")).toBe(2);
        zoomBlockIn("e1", 3);
        expect(level("e1")).toBe(4);
        undo();
        setBlock("e2", "editor", { "editor:tree_zoom": 0.6 });
        zoomBlockIn(subjectId("e2", "tree"), -0.5);
        expect(level("e2", "editor:tree_zoom")).toBe(0.5);
    });
});

describe("all panes", () => {
    it("steps both parts of an Editor, each by its own level, and resets both", () => {
        setBlock("e1", "editor", { "term:zoom": 1.5, "editor:tree_zoom": 1 });
        setBlock("t1", "term");
        zoomAllPanesIn();
        expect(level("e1")).toBeCloseTo(1.55, 5);
        expect(level("e1", "editor:tree_zoom")).toBeCloseTo(1.05, 5);
        expect(level("t1")).toBeCloseTo(1.05, 5);
        zoomAllPanesReset();
        expect([level("e1"), level("e1", "editor:tree_zoom"), level("t1")]).toEqual([1, 1, 1]);
    });
});

describe("the first start", () => {
    const parts = EDITOR_ZOOM.parts!;

    it("gives the tree the pane's zoom, once, so it keeps the size it had", () => {
        expect(zoomPartsSeed(parts, { "term:zoom": 1.3 })).toEqual({ "editor:tree_zoom": 1.3 });
    });

    it("writes 100% too, so a pane zoomed later does not seed the tree from that later level", () => {
        expect(zoomPartsSeed(parts, {})).toEqual({ "editor:tree_zoom": 1 });
        expect(zoomPartsSeed(parts, undefined)).toEqual({ "editor:tree_zoom": 1 });
    });

    it("never seeds a pane that has its tree's level", () => {
        expect(zoomPartsSeed(parts, { "term:zoom": 1.5, "editor:tree_zoom": 1 })).toEqual({});
    });
});

describe("the indicator", () => {
    it("names the part while the pane's parts are on screen, and the amount alone when only the pane is", () => {
        setBlock("e1", "editor");
        mountPane("e1");
        zoomBlockIn(TREE, 0.1);
        expect(zoomIndicatorTargetAtom()).toBe(TREE);
        expect(zoomIndicatorLabelAtom()).toBe("zoom.part.fileTree");
        zoomBlockIn("e1", 0.1);
        expect(zoomIndicatorLabelAtom()).toBe("zoom.part.editor");
        document.body.innerHTML = "";
        mountPane("e1", { tree: false });
        expect(zoomLabelKey("e1")).toBeNull();
    });
});
