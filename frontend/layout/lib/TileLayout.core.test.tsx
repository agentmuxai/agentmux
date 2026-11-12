// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Characterisation tests for `createTileLayout` (TileLayout.core.tsx): what the shared tile layout
 * does with its layout model and the platform hooks. They pin today's behaviour (written against
 * the unsplit function) so it can be split into named pieces without changing it. The layout
 * model, the shared overlay components, pragmatic-dnd's `draggable` and the window-drag events are
 * mocked; the drag-preview helpers, the tile-drag registrar and the drag state are real.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal, type JSX } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from "vitest";

interface DragConfig {
    element: HTMLElement;
    canDrag: (a: { input: { clientX: number; clientY: number } }) => boolean;
    getInitialData: () => unknown;
    onGenerateDragPreview: (a: {
        nativeSetDragImage?: (img: Element | HTMLImageElement, x: number, y: number) => void;
        location: { initial: { input: Record<string, number> } };
    }) => void;
    onDragStart: () => void;
    onDrop: () => void;
}
interface DisplayNodesProps {
    layoutModel: unknown;
    DisplayNode: (p: { layoutModel: unknown; node: unknown }) => JSX.Element;
}
interface OverlayProps {
    effectiveOverlayTransform: () => JSX.CSSProperties | undefined;
}
interface PlaceholderProps {
    style: JSX.CSSProperties;
}
type WindowDragConfig = { kinds: string[]; over: (e: { clientX: number; clientY: number }) => void };

const h = vi.hoisted(() => ({
    model: null as unknown as ReturnType<typeof makeModel>,
    nodeModels: new Map<string, ReturnType<typeof makeNodeModel>>(),
    draggable: [] as DragConfig[],
    unbind: null as unknown as Mock<() => void>,
    windowDrag: null as null | WindowDragConfig,
    stopWindowDrag: null as unknown as Mock<() => void>,
    placeholder: null as PlaceholderProps | null,
    overlay: null as OverlayProps | null,
    displayNodes: null as DisplayNodesProps | null,
    startTileDrag: null as unknown as Mock<(...a: unknown[]) => void>,
    releaseTileDrag: null as unknown as Mock<(...a: unknown[]) => void>,
    toPng: null as unknown as Mock<(...a: unknown[]) => Promise<string>>,
    gating: new Set<string>(),
    resizing: false,
    markBehind: null as unknown as Mock<(...a: unknown[]) => void>,
}));

vi.mock("@/app/motion/motion", () => ({ shouldReduceMotion: () => false }));
vi.mock("@/app/platform/window-resize", () => ({
    windowResizing: () => h.resizing,
    layoutBehind: () => false,
    markLayoutBehind: (...a: unknown[]) => h.markBehind(...a),
}));
vi.mock("@/app/store/tab-reveal", () => ({ gatingNodeIds: () => h.gating }));
vi.mock("@atlaskit/pragmatic-drag-and-drop/element/adapter", () => ({
    draggable: (cfg: DragConfig) => {
        h.draggable.push(cfg);
        return h.unbind;
    },
}));
vi.mock("@/app/drag/window-drag-events", () => ({
    onWindowDrag: (cfg: WindowDragConfig) => {
        h.windowDrag = cfg;
        return h.stopWindowDrag;
    },
}));
vi.mock("@/app/drag/wayland-drag-image", () => ({ hideDragImageOnNativeWayland: () => false }));
vi.mock("html-to-image", () => ({ toPng: (...a: unknown[]) => h.toPng(...a) }));
vi.mock("./layoutModel", () => ({ LayoutModel: class {} }));
vi.mock("./layoutModelHooks", () => ({
    useTileLayout: () => h.model,
    useNodeModel: (_m: unknown, node: { id: string }) => h.nodeModels.get(node.id),
}));
vi.mock("./tilelayout-shared", () => ({
    ClosedPaneGhosts: () => null,
    DisplayNodesWrapper: (p: DisplayNodesProps) => {
        h.displayNodes = p;
        return null;
    },
    MagnifiedPaneOverlay: () => null,
    NodeBackdrops: () => null,
    OverlayNodeWrapper: (p: OverlayProps) => {
        h.overlay = p;
        return null;
    },
    Placeholder: (p: PlaceholderProps) => {
        h.placeholder = p;
        return null;
    },
}));
vi.mock("./tile-drag", () => ({
    startTileDrag: (...a: unknown[]) => h.startTileDrag(...a),
    releaseTileDrag: (...a: unknown[]) => h.releaseTileDrag(...a),
}));

import { DURATION_BASE_MS } from "@/app/motion/tokens";
import { tileItemType } from "@/app/drag/drag-types";
import { createTileLayout, type TileLayoutPlatform } from "./TileLayout.core";
import { DRAG_PREVIEW_HOVER_MS } from "./drag-preview-size";
import { beginPaneCapture, resetPaneCaptureGateForTests } from "@/app/drag/pane-capture-gate";
import { REFLOW_CLIP, REFLOW_TRANSFORM, type ReflowEntry } from "./liveReflow";
import type { SettlingPass } from "./settlingPass";
import { dragState } from "./tilelayout-drag-state";
import { LayoutTreeActionType, type TileLayoutContents } from "./types";

function makeModel() {
    const [activeDrag, setActiveDrag] = createSignal(false);
    const [overlay, setOverlay] = createSignal<JSX.CSSProperties | undefined>({ transform: "a" });
    const [gap, setGap] = createSignal(10);
    const [animTime, setAnimTime] = createSignal(0.2);
    const [magnifiedId, setMagnifiedId] = createSignal<string | undefined>(undefined);
    const [magnifyMount, setMagnifyMount] = createSignal<HTMLElement | undefined>(undefined);
    const [isResizing, setIsResizing] = createSignal(false);
    const [ready, setReady] = createSignal(false);
    const [handles, setHandles] = createSignal<Array<{ id: string }>>([]);
    const [reflow, setReflow] = createSignal(new Map<string, ReflowEntry>());
    const [pass, setPass] = createSignal<SettlingPass | undefined>(undefined);
    return {
        activeDrag,
        settlingPass: pass,
        reflowFor: (id: string) => reflow().get(id),
        liveReflowActive: () => reflow().size > 0,
        overlayTransform: overlay,
        gapSizePx: gap,
        animationTimeS: animTime,
        magnifiedNodeIdAtom: magnifiedId,
        magnifyMount,
        isResizing,
        ready: Object.assign(ready, { _set: vi.fn((v: boolean) => setReady(v)) }),
        resizeHandles: handles,
        displayContainerRef: { current: null as HTMLElement | null },
        treeReducer: vi.fn(),
        renderContent: vi.fn((nm: { content?: () => HTMLElement }) => nm.content?.() ?? document.createElement("div")),
        renderPreview: vi.fn(() => document.createElement("span")),
        set: { setActiveDrag, setOverlay, setGap, setAnimTime, setMagnifiedId, setMagnifyMount, setIsResizing, setHandles, setReflow, setPass },
    };
}

function makeNodeModel() {
    const [ephemeral, setEphemeral] = createSignal(false);
    const [magnified, setMagnified] = createSignal(false);
    const [minimized, setMinimized] = createSignal(false);
    const [props, setProps] = createSignal<Record<string, unknown> | undefined>({ transform: { transform: "translate(1px, 2px)", width: "10px", height: "10px" } });
    return {
        isEphemeral: ephemeral,
        isMagnified: magnified,
        isMinimized: minimized,
        additionalProps: props,
        content: () => {
            const el = document.createElement("div");
            el.className = "content";
            el.innerHTML = '<div data-role="block-header"><div class="gap"></div><div class="pane-tab"></div></div>';
            return el;
        },
        set: { setEphemeral, setMagnified, setMinimized, setProps },
    };
}

type MockPlatform = TileLayoutPlatform & {
    onRootMount: Mock<() => void>;
    onLeafGeometryChange: Mock<() => void>;
    rejectDragAt: Mock<(model: unknown, input: { clientX: number; clientY: number }) => boolean>;
    beforeDragStart: Mock<() => void>;
    afterDragStart: Mock<() => void>;
    beforeDrop: Mock<() => void>;
    afterDrop: Mock<() => void>;
};

function makePlatform(over: Record<string, unknown> = {}): MockPlatform {
    return {
        animateDelayMs: 50,
        boundsCheckRequiresTileDrag: false,
        clearDraggingWhenInactive: false,
        onRootMount: vi.fn(),
        onLeafGeometryChange: vi.fn(),
        rejectDragAt: vi.fn(() => false),
        beforeDragStart: vi.fn(),
        afterDragStart: vi.fn(),
        beforeDrop: vi.fn(),
        afterDrop: vi.fn(),
        ResizeHandle: (p: { resizeHandleProps: { id: string } }) => {
            const el = document.createElement("div");
            el.className = "rh";
            el.dataset.id = p.resizeHandleProps.id;
            return el;
        },
        ...over,
    } as unknown as MockPlatform;
}

const NODE = { id: "n1", data: { blockId: "b1" } };

function mount(platform: MockPlatform, className = "my-layout") {
    const TileLayout = createTileLayout(platform);
    return render(() => <TileLayout tabAtom={() => ({}) as Tab} contents={{ className } as unknown as TileLayoutContents} />);
}

/** Mounts the layout and then the one tile the layout's node wrapper was given. */
function mountWithTile(platform: MockPlatform) {
    const layout = mount(platform);
    const tile = render(() => h.displayNodes!.DisplayNode({ layoutModel: h.model, node: NODE }));
    return { layout, tile, node: h.nodeModels.get("n1")!, el: tile.container.querySelector<HTMLElement>(".tile-node")! };
}

beforeEach(() => {
    resetPaneCaptureGateForTests();
    h.model = makeModel();
    h.nodeModels = new Map([["n1", makeNodeModel()]]);
    h.draggable = [];
    h.unbind = vi.fn();
    h.windowDrag = null;
    h.stopWindowDrag = vi.fn();
    h.placeholder = null;
    h.overlay = null;
    h.displayNodes = null;
    h.startTileDrag = vi.fn();
    h.releaseTileDrag = vi.fn();
    h.toPng = vi.fn(async () => "data:image/png;base64,AA==");
    h.gating = new Set();
    h.resizing = false;
    h.markBehind = vi.fn();
    dragState.nodeId = null;
    document.elementFromPoint = vi.fn<(x: number, y: number) => Element | null>(() => null);
});

afterEach(() => {
    cleanup();
    vi.useRealTimers();
});

describe("createTileLayout: the layout", () => {
    it("renders the tile-layout with the contents' class, the gap and animation time as CSS variables, and no animate class yet", () => {
        const { container } = mount(makePlatform());
        const root = container.querySelector<HTMLElement>(".tile-layout")!;
        expect(root.classList.contains("my-layout")).toBe(true);
        expect(root.classList.contains("animate")).toBe(false);
        expect(root.style.getPropertyValue("--gap-size-px")).toBe("10px");
        expect(root.style.getPropertyValue("--animation-time-s")).toBe("0.2s");
        h.model.set.setGap(14);
        expect(root.style.getPropertyValue("--gap-size-px")).toBe("14px");
    });

    it("hands the display container to the model", () => {
        const { container } = mount(makePlatform());
        expect(h.model.displayContainerRef.current).toBe(container.querySelector(".display-container"));
    });

    it("enables the animate class and flips ready after the platform's delay, and runs the platform's mount hook", () => {
        vi.useFakeTimers();
        const platform = makePlatform({ animateDelayMs: 150 });
        const { container } = mount(platform);
        expect(platform.onRootMount).toHaveBeenCalledTimes(1);
        vi.advanceTimersByTime(149);
        expect(container.querySelector(".tile-layout")!.classList.contains("animate")).toBe(false);
        expect(h.model.ready._set).not.toHaveBeenCalled();
        vi.advanceTimersByTime(1);
        expect(container.querySelector(".tile-layout")!.classList.contains("animate")).toBe(true);
        expect(h.model.ready._set).toHaveBeenCalledWith(true);
    });

    it("renders a resize handle for each handle of the model, by id", () => {
        const { container } = mount(makePlatform());
        expect(container.querySelectorAll(".rh").length).toBe(0);
        h.model.set.setHandles([{ id: "h1" }, { id: "h2" }]);
        expect([...container.querySelectorAll<HTMLElement>(".rh")].map((e) => e.dataset.id)).toEqual(["h1", "h2"]);
        h.model.set.setHandles([{ id: "h2" }]);
        expect([...container.querySelectorAll<HTMLElement>(".rh")].map((e) => e.dataset.id)).toEqual(["h2"]);
    });

    it("gives the display nodes wrapper a DisplayNode, and the overlay wrapper the effective transform", () => {
        mount(makePlatform());
        expect(typeof h.displayNodes!.DisplayNode).toBe("function");
        expect(h.displayNodes!.layoutModel).toBe(h.model);
        expect(h.overlay!.effectiveOverlayTransform()).toEqual({ transform: "a" });
    });

    describe("the overlay transform around a drag", () => {
        it("places the placeholder off-screen under the live transform", () => {
            mount(makePlatform());
            expect(h.placeholder!.style).toEqual({ top: "10000px", transform: "a" });
            h.model.set.setOverlay({ transform: "b" });
            expect(h.placeholder!.style).toEqual({ top: "10000px", transform: "b" });
        });

        it("holds the last in-drag transform for the exit fade after the drag ends, then lets go", () => {
            vi.useFakeTimers();
            mount(makePlatform());
            h.model.set.setActiveDrag(true);
            h.model.set.setOverlay({ transform: "b" });
            expect(h.overlay!.effectiveOverlayTransform()).toEqual({ transform: "b" });
            h.model.set.setActiveDrag(false);
            h.model.set.setOverlay({ transform: "c" });
            expect(h.overlay!.effectiveOverlayTransform()).toEqual({ transform: "b" });
            expect(h.placeholder!.style).toEqual({ top: "10000px", transform: "b" });
            vi.advanceTimersByTime(DURATION_BASE_MS - 1);
            expect(h.overlay!.effectiveOverlayTransform()).toEqual({ transform: "b" });
            vi.advanceTimersByTime(1);
            expect(h.overlay!.effectiveOverlayTransform()).toEqual({ transform: "c" });
        });

        it("a new drag during the hold takes the live transform again", () => {
            vi.useFakeTimers();
            mount(makePlatform());
            h.model.set.setActiveDrag(true);
            h.model.set.setActiveDrag(false);
            h.model.set.setOverlay({ transform: "c" });
            expect(h.overlay!.effectiveOverlayTransform()).toEqual({ transform: "a" });
            h.model.set.setActiveDrag(true);
            expect(h.overlay!.effectiveOverlayTransform()).toEqual({ transform: "c" });
            vi.advanceTimersByTime(DURATION_BASE_MS * 2);
            expect(h.overlay!.effectiveOverlayTransform()).toEqual({ transform: "c" });
        });

        it("holds nothing when no drag came before", () => {
            vi.useFakeTimers();
            mount(makePlatform());
            h.model.set.setOverlay({ transform: "z" });
            expect(h.overlay!.effectiveOverlayTransform()).toEqual({ transform: "z" });
        });
    });

    describe("clearing a pending drop when the cursor leaves", () => {
        const rect = { x: 100, y: 100, width: 200, height: 200 };
        const setup = (platform = makePlatform()) => {
            vi.useFakeTimers();
            const view = mount(platform);
            const container = view.container.querySelector<HTMLElement>(".display-container")!;
            container.getBoundingClientRect = () => rect as DOMRect;
            return view;
        };

        it("listens for tile drags only, and stops listening on unmount", () => {
            const view = setup();
            expect(h.windowDrag!.kinds).toEqual(["tile"]);
            view.unmount();
            expect(h.stopWindowDrag).toHaveBeenCalledTimes(1);
        });

        it("clears the pending action, after a 100 ms debounce, when the cursor is outside the container", () => {
            setup();
            h.windowDrag!.over({ clientX: 50, clientY: 150 });
            expect(h.model.treeReducer).not.toHaveBeenCalled();
            vi.advanceTimersByTime(100);
            expect(h.model.treeReducer).toHaveBeenCalledWith({ type: LayoutTreeActionType.ClearPendingAction });
        });

        it("clears it on each side of the container, edges included", () => {
            setup();
            for (const [x, y] of [
                [100, 150],
                [300, 150],
                [150, 100],
                [150, 300],
            ]) {
                h.model.treeReducer.mockClear();
                h.windowDrag!.over({ clientX: x, clientY: y });
                vi.advanceTimersByTime(100);
                expect(h.model.treeReducer).toHaveBeenCalledTimes(1);
            }
        });

        it("leaves it when the cursor is inside, and acts only on the last position of a burst", () => {
            setup();
            h.windowDrag!.over({ clientX: 50, clientY: 150 });
            h.windowDrag!.over({ clientX: 150, clientY: 150 });
            vi.advanceTimersByTime(100);
            expect(h.model.treeReducer).not.toHaveBeenCalled();
        });

        it("on a platform that requires a tile drag, does nothing until a tile is being dragged", () => {
            setup(makePlatform({ boundsCheckRequiresTileDrag: true }));
            h.windowDrag!.over({ clientX: 50, clientY: 150 });
            vi.advanceTimersByTime(100);
            expect(h.model.treeReducer).not.toHaveBeenCalled();
            dragState.nodeId = "n1";
            h.windowDrag!.over({ clientX: 50, clientY: 150 });
            vi.advanceTimersByTime(100);
            expect(h.model.treeReducer).toHaveBeenCalledTimes(1);
        });
    });
});

describe("createTileLayout: a tile", () => {
    it("renders the tile node under the node's id, with the leaf content inside and the preview beside it", () => {
        const { el } = mountWithTile(makePlatform());
        expect(el.id).toBe("n1");
        expect(el.querySelector(".tile-leaf .content")).not.toBeNull();
        expect(el.querySelector(".tile-preview-container .tile-preview span")).not.toBeNull();
        expect(h.model.renderContent).toHaveBeenCalledWith(h.nodeModels.get("n1"));
    });

    it("styles the tile with the node's transform, and shows the flags as classes", () => {
        const { el } = mountWithTile(makePlatform());
        const nm = h.nodeModels.get("n1")!;
        expect(el.style.transform).toBe("translate(1px, 2px)");
        expect(el.style.width).toBe("10px");
        for (const c of ["dragging", "tile-hidden", "tile-minimized", "tile-resize-skip", "tile-entering"]) {
            expect(el.classList.contains(c)).toBe(false);
        }
        nm.set.setMinimized(true);
        expect(el.classList.contains("tile-minimized")).toBe(true);
        h.model.set.setMagnifiedId("other");
        expect(el.classList.contains("tile-hidden")).toBe(true);
    });

    it("fades in a pane that came or was restored, for as long as the settling pass names it", () => {
        const { el } = mountWithTile(makePlatform());
        const pass = (enters: string[], gliding: boolean): SettlingPass => ({ moves: new Map(), ghosts: [], enters, gliding });
        h.model.set.setPass(pass(["other"], false));
        expect(el.classList.contains("tile-entering")).toBe(false);
        // While the pass stands at its start the pane is not drawn: its fade starts with the glide.
        h.model.set.setPass(pass(["n1"], false));
        expect(el.classList.contains("tile-entering")).toBe(false);
        expect(el.style.opacity).toBe("0");
        expect(el.style.transition).toBe("");
        h.model.set.setPass(pass(["n1"], true));
        expect(el.classList.contains("tile-entering")).toBe(true);
        expect(el.style.opacity).toBe("1");
        // A second pass that names it while it fades (another pane opened) is gliding too: not started again.
        const sets = vi.spyOn(el.classList, "toggle");
        h.model.set.setPass(pass(["n1", "other"], true));
        expect(el.classList.contains("tile-entering")).toBe(true);
        expect(sets).not.toHaveBeenCalled();
        // The stylesheet's own length (the glide's), with nothing set on the tile.
        expect(el.style.getPropertyValue("animation-duration")).toBe("");
        h.model.set.setPass(undefined);
        expect(el.classList.contains("tile-entering")).toBe(false);
        expect(el.style.opacity).toBe("1");
    });

    it("skips laying out a minimized pane while the window resizes, and says it is behind", () => {
        h.resizing = true;
        h.nodeModels.get("n1")!.set.setMinimized(true);
        const { el } = mountWithTile(makePlatform());
        expect(el.classList.contains("tile-resize-skip")).toBe(true);
        expect(h.markBehind).toHaveBeenCalledWith("n1");
    });

    it("lays out a pane that is not minimized even while the window resizes", () => {
        h.resizing = true;
        const { el } = mountWithTile(makePlatform());
        expect(el.classList.contains("tile-resize-skip")).toBe(false);
        expect(h.markBehind).not.toHaveBeenCalled();
    });

    it("hides a leaf that is mid-remount", () => {
        h.gating = new Set(["n1"]);
        const { el } = mountWithTile(makePlatform());
        expect(el.style.visibility).toBe("hidden");
        expect(el.style.opacity).toBe("0");
    });

    it("shows a leaf that is not mid-remount at full opacity", () => {
        const { el } = mountWithTile(makePlatform());
        expect(el.style.visibility).toBe("");
        expect(el.style.opacity).toBe("1");
    });

    describe("the live reflow of a pane drag (liveReflow.ts)", () => {
        const rect = { left: 300, top: 0, width: 40, height: 20 };
        const entry: ReflowEntry = { rect, box: { width: 40, height: 30 }, moved: false, leaving: false };

        /** The tile's four edges, as its style carries them: left, top, right, bottom. */
        const edgesOf = (el: HTMLElement) => ["l", "t", "r", "b"].map((edge) => el.style.getPropertyValue(`--reflow-${edge}`));

        it("gives a tile its future rect: its four edges glide, its place and crop are read from them, its size is set at once", () => {
            const platform = makePlatform();
            const { el, node } = mountWithTile(platform);
            const before = node.additionalProps();
            h.model.set.setReflow(new Map<string, ReflowEntry>([["n1", entry]]));
            expect(edgesOf(el)).toEqual(["300px", "0px", "340px", "20px"]);
            expect(el.style.transform).toBe(REFLOW_TRANSFORM);
            // The tile is laid out at its box (here still its old height, under the closing crop) and shows its
            // rect's size whatever the box is.
            expect(el.style.width).toBe("40px");
            expect(el.style.height).toBe("30px");
            expect(el.style.getPropertyValue("clip-path")).toBe(REFLOW_CLIP);
            // Only the edges glide. A transition on the transform would run on the compositor, apart from the
            // crop; one on the place and one on the size would start again separately when a candidate changes.
            for (const edge of ["l", "t", "r", "b"]) {
                expect(el.style.transition).toContain(`--reflow-${edge} var(--duration-base) ease-out`);
            }
            expect(el.style.transition).not.toMatch(/(^|, )(transform|clip-path) /);
            // Neither size is a transition: a size that animated would lay the pane out on every frame.
            expect(el.style.transition).not.toContain("width");
            expect(el.style.transition).not.toContain("height");
            // The layout's own rect is untouched (it stays the drag's hit-test snapshot), and the platform's
            // geometry signal, which follows that rect, never fires.
            expect(node.additionalProps()).toBe(before);
            expect(platform.onLeafGeometryChange).not.toHaveBeenCalled();
        });

        it("takes the rect's own size once the glide is over", () => {
            const { el } = mountWithTile(makePlatform());
            h.model.set.setReflow(new Map<string, ReflowEntry>([["n1", entry]]));
            h.model.set.setReflow(new Map<string, ReflowEntry>([["n1", { ...entry, box: { width: 40, height: 20 } }]]));
            expect(el.style.height).toBe("20px");
            expect(edgesOf(el)).toEqual(["300px", "0px", "340px", "20px"]);
        });

        it("shows the dragged pane's own tile in its landing rect, unblurred and never hidden, so no slot is empty", () => {
            const { el } = mountWithTile(makePlatform());
            h.model.set.setReflow(new Map<string, ReflowEntry>([["n1", { ...entry, moved: true }]]));
            expect(el.style.visibility).toBe("");
            expect(edgesOf(el)).toEqual(["300px", "0px", "340px", "20px"]);
            expect(el.style.filter).toBe("none");
            // Another pane keeps whatever the stylesheet gives a dragged tile.
            h.model.set.setReflow(new Map<string, ReflowEntry>([["n1", entry]]));
            expect(el.style.filter).toBe("");
        });

        it("does not show a pane that is being torn off: its neighbours close over its slot (tearOffPreview.ts)", () => {
            const { el } = mountWithTile(makePlatform());
            h.model.set.setReflow(new Map<string, ReflowEntry>([["n1", { ...entry, leaving: true }]]));
            expect(el.style.opacity).toBe("0");
            // It fades over the glide's length, not the quicker fade of a tile that appears.
            expect(el.style.transition).toContain("opacity var(--duration-base)");
            h.model.set.setReflow(new Map<string, ReflowEntry>([["n1", entry]]));
            expect(el.style.opacity).toBe("1");
        });

        it("drops the reflow's transform, size, crop and transition in one step when the reflow ends, so the drop does not glide twice", () => {
            const { el } = mountWithTile(makePlatform());
            h.model.set.setReflow(new Map<string, ReflowEntry>([["n1", entry]]));
            h.model.set.setReflow(new Map());
            expect(el.style.transform).toBe("translate(1px, 2px)");
            expect(el.style.width).toBe("10px");
            expect(el.style.height).toBe("10px");
            expect(el.style.getPropertyValue("clip-path")).toBe("");
            expect(el.style.transition).not.toContain("--reflow-");
            expect(el.style.visibility).toBe("");
        });

        it("carries a tile's own edges at rest, so the first glide starts from them and not from the origin", () => {
            const { el, node } = mountWithTile(makePlatform());
            node.set.setProps({
                transform: { transform: "translate3d(7px,9px, 0)", width: "10px", height: "10px" },
                rect: { left: 7.4, top: 9.6, width: 10, height: 10 },
            });
            expect(edgesOf(el)).toEqual(["7px", "9px", "17px", "19px"]);
            // Its place and size stay the layout's own, and nothing glides.
            expect(el.style.transform).toBe("translate3d(7px,9px, 0)");
            expect(el.style.getPropertyValue("clip-path")).toBe("");
            expect(el.style.transition).not.toContain("--reflow-");
        });
    });

    describe("the platform's geometry hook", () => {
        it("fires when the transform, width or height changes once the layout is ready and not resizing", () => {
            const platform = makePlatform();
            const { node } = mountWithTile(platform);
            h.model.ready._set(true);
            node.set.setProps({ transform: { transform: "translate(1px, 2px)", width: "10px", height: "10px" } });
            expect(platform.onLeafGeometryChange).not.toHaveBeenCalled();
            node.set.setProps({ transform: { transform: "t1", width: "10px", height: "10px" } });
            expect(platform.onLeafGeometryChange).toHaveBeenCalledTimes(1);
            node.set.setProps({ transform: { transform: "t1", width: "20px", height: "10px" } });
            node.set.setProps({ transform: { transform: "t1", width: "20px", height: "30px" } });
            expect(platform.onLeafGeometryChange).toHaveBeenCalledTimes(3);
        });

        it("stays quiet before the layout is ready, and while a resize drag is on", () => {
            const platform = makePlatform();
            const { node } = mountWithTile(platform);
            node.set.setProps({ transform: { transform: "t1" } });
            expect(platform.onLeafGeometryChange).not.toHaveBeenCalled();
            h.model.ready._set(true);
            h.model.set.setIsResizing(true);
            node.set.setProps({ transform: { transform: "t2" } });
            expect(platform.onLeafGeometryChange).not.toHaveBeenCalled();
        });

        it("is not asked for on a platform without the hook", () => {
            const platform = makePlatform({ onLeafGeometryChange: undefined });
            const { node } = mountWithTile(platform);
            h.model.ready._set(true);
            expect(() => node.set.setProps({ transform: { transform: "t9" } })).not.toThrow();
        });
    });

    describe("magnify", () => {
        it("moves the leaf into the magnify overlay's mount while magnified, and back when not", () => {
            const { el } = mountWithTile(makePlatform());
            const nm = h.nodeModels.get("n1")!;
            const mountEl = document.createElement("div");
            const leaf = el.querySelector(".tile-leaf")!;
            h.model.set.setMagnifyMount(mountEl);
            expect(leaf.parentElement).toBe(el);
            nm.set.setMagnified(true);
            expect(leaf.parentElement).toBe(mountEl);
            nm.set.setMagnified(false);
            expect(leaf.parentElement).toBe(el);
            expect(el.firstChild).toBe(leaf);
        });
    });

    describe("the whole-pane drag", () => {
        const start = () => {
            const platform = makePlatform();
            const view = mountWithTile(platform);
            return { platform, ...view, cfg: h.draggable[0] };
        };

        it("is registered on the header that is in the DOM, with the node's id as its data", () => {
            const { el, cfg } = start();
            expect(h.draggable.length).toBe(1);
            expect(cfg.element).toBe(el.querySelector('[data-role="block-header"]'));
            expect(cfg.getInitialData()).toEqual({ nodeId: "n1", type: tileItemType });
        });

        it("unbinds the drag when the tile goes away", () => {
            const { tile } = start();
            tile.unmount();
            expect(h.unbind).toHaveBeenCalledTimes(1);
        });

        it("allows a drag from the empty part of the header", () => {
            const { cfg } = start();
            expect(cfg.canDrag({ input: { clientX: 5, clientY: 5 } })).toBe(true);
            expect(document.elementFromPoint).toHaveBeenCalledWith(5, 5);
        });

        it("refuses it for an ephemeral or a magnified pane", () => {
            const { cfg } = start();
            const nm = h.nodeModels.get("n1")!;
            nm.set.setEphemeral(true);
            expect(cfg.canDrag({ input: { clientX: 1, clientY: 1 } })).toBe(false);
            nm.set.setEphemeral(false);
            nm.set.setMagnified(true);
            expect(cfg.canDrag({ input: { clientX: 1, clientY: 1 } })).toBe(false);
        });

        it("refuses it where the platform vetoes the press", () => {
            const { cfg, platform } = start();
            platform.rejectDragAt.mockReturnValue(true);
            expect(cfg.canDrag({ input: { clientX: 1, clientY: 1 } })).toBe(false);
            expect(platform.rejectDragAt.mock.calls[0][1]).toEqual({ clientX: 1, clientY: 1 });
        });

        it("refuses it for a press on a pane tab pill, which has its own drag", () => {
            const { cfg, el } = start();
            (document.elementFromPoint as Mock).mockReturnValue(el.querySelector(".pane-tab"));
            expect(cfg.canDrag({ input: { clientX: 1, clientY: 1 } })).toBe(false);
        });

        it("runs the platform's hooks around the tile drag, in order, and flags the tile as dragging", () => {
            const { cfg, el, platform } = start();
            const order: string[] = [];
            platform.beforeDragStart.mockImplementation(() => order.push("beforeStart"));
            h.startTileDrag.mockImplementation(() => order.push("start"));
            platform.afterDragStart.mockImplementation(() => order.push("afterStart"));
            platform.beforeDrop.mockImplementation(() => order.push("beforeDrop"));
            h.releaseTileDrag.mockImplementation(() => order.push("release"));
            platform.afterDrop.mockImplementation(() => order.push("afterDrop"));
            cfg.onDragStart();
            expect(el.classList.contains("dragging")).toBe(true);
            expect(h.startTileDrag).toHaveBeenCalledWith(NODE, h.model);
            cfg.onDrop();
            expect(el.classList.contains("dragging")).toBe(false);
            expect(h.releaseTileDrag).toHaveBeenCalledWith(h.model);
            expect(order).toEqual(["beforeStart", "start", "afterStart", "beforeDrop", "release", "afterDrop"]);
        });

        it("blurs the dragged tile only once the pane's picture has been taken, so the picture is of the pane as it stood", () => {
            const { cfg, el } = start();
            const captured = beginPaneCapture();
            cfg.onDragStart();
            expect(el.classList.contains("dragging")).toBe(false);
            captured();
            expect(el.classList.contains("dragging")).toBe(true);
            cfg.onDrop();
        });

        it("works on a platform with no drag hooks", () => {
            const platform = makePlatform({
                beforeDragStart: undefined,
                afterDragStart: undefined,
                beforeDrop: undefined,
                afterDrop: undefined,
                rejectDragAt: undefined,
            });
            mountWithTile(platform);
            const cfg = h.draggable[0];
            expect(() => {
                cfg.onDragStart();
                cfg.onDrop();
            }).not.toThrow();
            expect(cfg.canDrag({ input: { clientX: 1, clientY: 1 } })).toBe(true);
        });

        it("keeps the dragging flag when the drag ends without a drop on a platform that does not clear it", () => {
            const { cfg, el } = start();
            cfg.onDragStart();
            h.model.set.setActiveDrag(true);
            h.model.set.setActiveDrag(false);
            expect(el.classList.contains("dragging")).toBe(true);
        });

        it("clears the dragging flag when the layout's drag ends without a drop, where the platform asks for it", () => {
            const platform = makePlatform({ clearDraggingWhenInactive: true });
            const { el } = mountWithTile(platform);
            const cfg = h.draggable[0];
            h.startTileDrag.mockImplementation(() => h.model.set.setActiveDrag(true));
            cfg.onDragStart();
            expect(el.classList.contains("dragging")).toBe(true);
            h.model.set.setActiveDrag(false);
            expect(el.classList.contains("dragging")).toBe(false);
        });

        it("rebinds to a new header when the tile's DOM changes, but not mid-drag", async () => {
            const { el } = start();
            const swap = () => {
                const header = document.createElement("div");
                header.dataset.role = "block-header";
                el.querySelector(".content")!.replaceChildren(header);
                return header;
            };
            const first = swap();
            await Promise.resolve();
            expect(h.draggable.length).toBe(2);
            expect(h.draggable[1].element).toBe(first);
            expect(h.unbind).toHaveBeenCalledTimes(1);
            h.draggable[1].onDragStart();
            h.model.set.setActiveDrag(true);
            swap();
            await Promise.resolve();
            expect(h.draggable.length).toBe(2);
            h.model.set.setActiveDrag(false);
            expect(h.draggable.length).toBe(3);
        });

        describe("the drag image", () => {
            it("is the header for a minimized pane", () => {
                const { cfg, el } = start();
                h.nodeModels.get("n1")!.set.setMinimized(true);
                const set = vi.fn();
                cfg.onGenerateDragPreview({
                    nativeSetDragImage: set,
                    location: { initial: { input: { clientX: 4, clientY: 6 } } },
                });
                expect(set).toHaveBeenCalledTimes(1);
                expect(set.mock.calls[0][0]).toBe(el.querySelector('[data-role="block-header"]'));
            });

            it("is nothing when the browser gives no way to set it", () => {
                const { cfg } = start();
                expect(() =>
                    cfg.onGenerateDragPreview({ nativeSetDragImage: undefined, location: { initial: { input: {} } } })
                ).not.toThrow();
            });

            it("is the header too when no ghost has been made yet", () => {
                const { cfg, el } = start();
                const set = vi.fn();
                cfg.onGenerateDragPreview({
                    nativeSetDragImage: set,
                    location: { initial: { input: { clientX: 4, clientY: 6 } } },
                });
                expect(set).toHaveBeenCalledTimes(1);
                expect(set.mock.calls[0][0]).toBe(el.querySelector('[data-role="block-header"]'));
            });
        });
    });

    describe("the drag preview", () => {
        it("is rasterised once the pointer rests on the tile, not when it only crosses it", async () => {
            vi.useFakeTimers();
            const { el } = mountWithTile(makePlatform());
            el.dispatchEvent(new MouseEvent("pointerenter"));
            await vi.advanceTimersByTimeAsync(DRAG_PREVIEW_HOVER_MS - 1);
            el.dispatchEvent(new MouseEvent("pointerleave"));
            await vi.advanceTimersByTimeAsync(DRAG_PREVIEW_HOVER_MS * 2);
            expect(h.toPng).not.toHaveBeenCalled();
            el.dispatchEvent(new MouseEvent("pointerenter"));
            await vi.advanceTimersByTimeAsync(DRAG_PREVIEW_HOVER_MS);
            expect(h.toPng).toHaveBeenCalledTimes(1);
            expect(h.toPng.mock.calls[0][0]).toBe(el.querySelector(".tile-preview"));
        });

        it("is rasterised at once by a press on the header, but not by one on a pane tab pill or the body", async () => {
            const { el } = mountWithTile(makePlatform());
            const press = (target: Element) => target.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true }));
            press(el.querySelector(".pane-tab")!);
            press(el.querySelector(".tile-leaf")!);
            await Promise.resolve();
            expect(h.toPng).not.toHaveBeenCalled();
            press(el.querySelector(".gap")!);
            await vi.waitFor(() => expect(h.toPng).toHaveBeenCalledTimes(1));
        });

        it("leaves an image it can't inline blank, and a failed picture is a warning, not an unhandled rejection", async () => {
            const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
            const unhandled = vi.fn();
            process.on("unhandledRejection", unhandled);
            const img = document.createElement("img");
            img.src = "https://example.com/favicon.ico";
            const failed = new Event("error");
            Object.defineProperty(failed, "target", { value: img });
            h.toPng = vi.fn(async () => Promise.reject(failed));
            const { el } = mountWithTile(makePlatform());
            el.querySelector(".gap")!.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true }));
            await vi.waitFor(() => expect(warn).toHaveBeenCalled());
            await new Promise((r) => setTimeout(r, 0));
            const options = h.toPng.mock.calls[0][1] as { imagePlaceholder: string; onImageErrorHandler: () => void };
            expect(options.imagePlaceholder).toMatch(/^data:image\/gif;base64,/);
            expect(options.onImageErrorHandler).toBeTypeOf("function");
            expect(warn.mock.calls[0][0]).toBe(
                "[drag-preview] no picture of the pane: error event on img https://example.com/favicon.ico"
            );
            expect(unhandled).not.toHaveBeenCalled();
            process.off("unhandledRejection", unhandled);
            warn.mockRestore();
        });

        it("is reused until the pane's size changes", async () => {
            const { el } = mountWithTile(makePlatform());
            const press = () => el.querySelector(".gap")!.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true }));
            press();
            await vi.waitFor(() => expect(h.toPng).toHaveBeenCalledTimes(1));
            await new Promise((r) => setTimeout(r, 0));
            press();
            await new Promise((r) => setTimeout(r, 0));
            expect(h.toPng).toHaveBeenCalledTimes(1);
        });

        it("is made again when the pane's size changed since it was made", async () => {
            const { el } = mountWithTile(makePlatform());
            const press = () => el.querySelector(".gap")!.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true }));
            el.getBoundingClientRect = () => ({ width: 400, height: 200 }) as DOMRect;
            press();
            await vi.waitFor(() => expect(h.toPng).toHaveBeenCalledTimes(1));
            await new Promise((r) => setTimeout(r, 0));
            el.getBoundingClientRect = () => ({ width: 200, height: 400 }) as DOMRect;
            press();
            await vi.waitFor(() => expect(h.toPng).toHaveBeenCalledTimes(2));
        });

        it("sizes the hidden preview element from the pane being dragged", async () => {
            const { el } = mountWithTile(makePlatform());
            el.getBoundingClientRect = () => ({ width: 400, height: 200 }) as DOMRect;
            el.querySelector(".gap")!.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true }));
            const preview = el.querySelector<HTMLElement>(".tile-preview")!;
            expect(preview.style.width).not.toBe("");
            expect(preview.style.height).not.toBe("");
            const ratio = parseFloat(preview.style.width) / parseFloat(preview.style.height);
            expect(ratio).toBeCloseTo(2, 1);
        });

        it("drops the result of an older rasterisation that finishes after a newer one", async () => {
            const { el } = mountWithTile(makePlatform());
            const resolvers: Array<(u: string) => void> = [];
            h.toPng.mockImplementation(() => new Promise<string>((r) => resolvers.push(r)));
            const press = () => el.querySelector(".gap")!.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true }));
            el.getBoundingClientRect = () => ({ width: 400, height: 200 }) as DOMRect;
            press();
            await vi.waitFor(() => expect(resolvers.length).toBe(1));
            el.getBoundingClientRect = () => ({ width: 200, height: 400 }) as DOMRect;
            press();
            await vi.waitFor(() => expect(resolvers.length).toBe(2));
            resolvers[1]("data:new");
            await new Promise((r) => setTimeout(r, 0));
            resolvers[0]("data:old");
            await new Promise((r) => setTimeout(r, 0));
            const cfg = h.draggable[0];
            const set = vi.fn();
            cfg.onGenerateDragPreview({
                nativeSetDragImage: set,
                location: { initial: { input: { clientX: 1, clientY: 1 } } },
            });
            expect(set).toHaveBeenCalledTimes(1);
            expect((set.mock.calls[0][0] as HTMLImageElement).src).toBe("data:new");
        });

        it("stops its hover timer when the tile goes away", async () => {
            vi.useFakeTimers();
            const { el, tile } = mountWithTile(makePlatform());
            el.dispatchEvent(new MouseEvent("pointerenter"));
            tile.unmount();
            await vi.advanceTimersByTimeAsync(DRAG_PREVIEW_HOVER_MS * 2);
            expect(h.toPng).not.toHaveBeenCalled();
        });

        it("keeps pointer-over from reaching the tiles beneath", () => {
            const { el } = mountWithTile(makePlatform());
            const outer = vi.fn();
            window.addEventListener("pointerover", outer);
            el.dispatchEvent(new MouseEvent("pointerover", { bubbles: true }));
            window.removeEventListener("pointerover", outer);
            expect(outer).not.toHaveBeenCalled();
        });
    });
});
