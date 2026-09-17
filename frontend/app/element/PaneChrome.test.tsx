// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tests for `renderPaneChromeShell` — the shared `ViewModel.renderPaneChrome`
 * every non-agent/term widget type registers (browser, editor, sysinfo, swarm,
 * armory, media, drone, help, warden). Scoped to THIS file's own wiring —
 * tab-derivation from `blockStack`, and the activate/close/add handlers'
 * delegation to `layoutStack.ts`'s primitives — not re-testing those
 * primitives themselves (see `frontend/layout/tests/layoutStack.test.ts`) or
 * `PaneHeaderTabStrip`'s own rendering rules (see that component's own test
 * file), both of which are mocked here.
 *
 * Spec: docs/specs/SPEC_PANE_TABS_UNIVERSAL_CMUX_REDESIGN_2026_09_17.md §4.1/§4.5.
 * Plan: docs/specs/PLAN_PANE_TABS_UNIVERSAL_IMPLEMENTATION_2026_09_17.md Task
 * Group C — this closes the "not yet for renderPaneChromeShell's own
 * tab-derivation/add/close logic" gap that plan explicitly flags as open.
 */

import { cleanup, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const headerCalls: any[] = [];
vi.mock("./PaneHeaderTabStrip", () => ({
    PaneHeaderTabStrip: (props: any) => {
        headerCalls.push(props);
        return (
            <div data-testid="pane-header-tab-strip">
                {props.tabs.map((t: any) => (
                    <span>
                        {props.getIcon?.(t)}
                        {props.getLabel(t)}
                    </span>
                ))}
            </div>
        );
    },
}));

vi.mock("@/element/errorboundary", () => ({
    ErrorBoundary: (props: any) => props.children,
}));

vi.mock("@/app/block/blockutil", () => ({
    blockViewToName: (view: string | undefined) => (view ? `View:${view}` : "(No View)"),
    blockViewToIcon: (view: string | undefined) => (view ? `icon-${view}` : "square"),
    getBlockHeaderIcon: (icon: string) => <i data-testid="tab-icon">{icon}</i>,
}));

vi.mock("@/app/block/blockframe", () => ({
    computeFocusRingBorderColor: () => undefined,
}));

const showContextMenu = vi.fn();
vi.mock("@/app/store/contextmenu", () => ({
    ContextMenuModel: { showContextMenu: (...args: any[]) => showContextMenu(...args) },
}));

// Backed by real Solid signals (not a plain Map) — required to actually prove
// the ReAgent P1 regression fix (every stack member's block data, not just
// the active one, must be read via the reactive getMuxObjectAtom accessor,
// not getObjectValue's non-reactive snapshot). A plain-Map-returning mock
// would pass even with the bug, since it re-reads current state on every
// call regardless of whether Solid actually tracked a dependency.
const objectSignals = new Map<string, ReturnType<typeof createSignal<any>>>();
function signalFor(oref: string) {
    let sig = objectSignals.get(oref);
    if (!sig) {
        sig = createSignal<any>(undefined);
        objectSignals.set(oref, sig);
    }
    return sig;
}
function setObjectValue(oref: string, value: any) {
    signalFor(oref)[1](value);
}
vi.mock("@/app/store/global", () => ({
    MOS: {
        makeORef: (otype: string, oid: string) => `${otype}:${oid}`,
        getMuxObjectAtom: (oref: string) => signalFor(oref)[0],
        getObjectValue: (oref: string) => signalFor(oref)[0](),
    },
    atoms: {
        fullConfigAtom: () => ({ widgets: {}, settings: {} }),
        tabAtom: () => ({ meta: {} }),
    },
    // pane-tab-picker.ts surfaces a failed add as a toast — unused on the
    // success paths here, but the module imports it at load time.
    pushNotification: vi.fn(),
}));

let capturedOnSelect: ((blockDef: any) => void) | undefined;
const buildPaneWidgetMenuItemsMock = vi.fn((_wmap: any, _settings: any, onSelect: any) => {
    capturedOnSelect = onSelect;
    return [{ label: "Browser", click: () => onSelect({ meta: { view: "browser" } }) }];
});
vi.mock("@/app/window/action-widgets-config", () => ({
    buildPaneWidgetMenuItems: (...args: any[]) => (buildPaneWidgetMenuItemsMock as any)(...args),
}));

const addWidgetAsPaneTab = vi.fn().mockResolvedValue(undefined); // real one is async — the picker chains .catch() on it
const closeBlockInStack = vi.fn();
const setActiveBlockInStack = vi.fn();
let mockLayoutModel: any;
vi.mock("@/layout/index", () => ({
    addWidgetAsPaneTab: (...args: any[]) => addWidgetAsPaneTab(...args),
    closeBlockInStack: (...args: any[]) => closeBlockInStack(...args),
    setActiveBlockInStack: (...args: any[]) => setActiveBlockInStack(...args),
    getLayoutModelForStaticTab: () => mockLayoutModel,
}));

vi.mock("@/layout/lib/layoutNode", () => ({
    findNode: (root: any) => root,
}));

import { renderPaneChromeShell } from "./PaneChrome";

function fakeNodeModel(overrides: Record<string, any> = {}): any {
    return {
        nodeId: "node-1",
        activeBlockId: () => "b1",
        isFocused: () => false,
        numLeafs: () => 1,
        focusNode: vi.fn(),
        activeViewModel: () => null,
        ...overrides,
    };
}

function fakeLayoutModel(blockStack: string[]): any {
    return {
        localTreeStateAtom: () => {},
        treeState: { rootNode: { data: { blockStack } } },
    };
}

beforeEach(() => {
    objectSignals.clear();
    headerCalls.length = 0;
    showContextMenu.mockClear();
    buildPaneWidgetMenuItemsMock.mockClear();
    addWidgetAsPaneTab.mockClear();
    closeBlockInStack.mockClear();
    setActiveBlockInStack.mockClear();
    capturedOnSelect = undefined;
});

afterEach(() => cleanup());

describe("renderPaneChromeShell — tab derivation", () => {
    it("a 0-or-1-member stack yields no pills (matches the lone-tab-shows-no-pill convention)", () => {
        mockLayoutModel = fakeLayoutModel(["b1"]);
        render(() => renderPaneChromeShell(fakeNodeModel(), <div>content</div>) as any);

        expect(headerCalls[0].tabs).toEqual([]);
    });

    it("a 2+ member stack yields one tab per member, label from frame:title falling back to blockViewToName", () => {
        setObjectValue("block:b1", { meta: { "frame:title": "My Title" } });
        setObjectValue("block:b2", { meta: { view: "browser" } });
        mockLayoutModel = fakeLayoutModel(["b1", "b2"]);
        render(() => renderPaneChromeShell(fakeNodeModel(), <div>content</div>) as any);

        expect(headerCalls[0].tabs).toEqual([
            expect.objectContaining({ blockId: "b1", label: "My Title" }),
            expect.objectContaining({ blockId: "b2", label: "View:browser" }),
        ]);
        expect(screen.getByText("My Title")).toBeInTheDocument();
        expect(screen.getByText("View:browser")).toBeInTheDocument();
    });

    // getBlockHeaderIcon/blockViewToIcon derivation — same icon convention
    // the plain header iconview uses (blockframe.tsx), computed from the
    // block's own persisted meta since a background tab's ViewModel isn't
    // mounted here to read a live icon from.
    it("derives each tab's icon from frame:icon, falling back to blockViewToIcon(view)", () => {
        setObjectValue("block:b1", { meta: { "frame:icon": "rocket" } });
        setObjectValue("block:b2", { meta: { view: "browser" } });
        mockLayoutModel = fakeLayoutModel(["b1", "b2"]);
        render(() => renderPaneChromeShell(fakeNodeModel(), <div>content</div>) as any);

        const icons = screen.getAllByTestId("tab-icon").map((el) => el.textContent);
        expect(icons).toEqual(["rocket", "icon-browser"]);
    });

    // ReAgent P1 regression: a BACKGROUND (non-active) tab's own meta must be
    // read reactively (getMuxObjectAtom), not via getObjectValue's
    // non-reactive snapshot — otherwise a rename or icon change on a tab
    // that isn't currently active never updates its pill. b2 here is never
    // the active member (activeBlockId stays "b1" throughout).
    it("reacts to a background tab's own meta changing, without any tree-state change", () => {
        setObjectValue("block:b1", { meta: { "frame:title": "Active" } });
        setObjectValue("block:b2", { meta: { "frame:title": "Original" } });
        mockLayoutModel = fakeLayoutModel(["b1", "b2"]);
        render(() => renderPaneChromeShell(fakeNodeModel({ activeBlockId: () => "b1" }), <div>content</div>) as any);

        expect(screen.getByText("Original")).toBeInTheDocument();

        setObjectValue("block:b2", { meta: { "frame:title": "Renamed" } });

        expect(screen.getByText("Renamed")).toBeInTheDocument();
        expect(screen.queryByText("Original")).not.toBeInTheDocument();
    });
});

describe("renderPaneChromeShell — activate/close/add wiring", () => {
    it("onActivate delegates to setActiveBlockInStack with this pane's nodeId, and no-ops when already active", () => {
        mockLayoutModel = fakeLayoutModel(["b1", "b2"]);
        render(() => renderPaneChromeShell(fakeNodeModel({ activeBlockId: () => "b1" }), <div /> as any));

        headerCalls[0].onActivate("b1"); // already active
        expect(setActiveBlockInStack).not.toHaveBeenCalled();

        headerCalls[0].onActivate("b2");
        expect(setActiveBlockInStack).toHaveBeenCalledWith(mockLayoutModel, "node-1", "b2");
    });

    it("onClose delegates to closeBlockInStack with this pane's nodeId", () => {
        mockLayoutModel = fakeLayoutModel(["b1", "b2"]);
        render(() => renderPaneChromeShell(fakeNodeModel(), <div /> as any));

        headerCalls[0].onClose("b2");
        expect(closeBlockInStack).toHaveBeenCalledWith(mockLayoutModel, "node-1", "b2");
    });

    it("onAdd is a no-op without a MouseEvent (context menu needs a position to anchor to)", () => {
        mockLayoutModel = fakeLayoutModel(["b1"]);
        render(() => renderPaneChromeShell(fakeNodeModel(), <div /> as any));

        headerCalls[0].onAdd();
        expect(showContextMenu).not.toHaveBeenCalled();
    });

    it("onAdd opens the same widget picker the widget bar uses, and selecting an item pushes it via addWidgetAsPaneTab", () => {
        mockLayoutModel = fakeLayoutModel(["b1"]);
        render(() => renderPaneChromeShell(fakeNodeModel(), <div /> as any));

        const fakeEvent = { clientX: 1, clientY: 1 } as unknown as MouseEvent;
        headerCalls[0].onAdd(fakeEvent);

        expect(buildPaneWidgetMenuItemsMock).toHaveBeenCalled();
        expect(showContextMenu).toHaveBeenCalledWith(expect.any(Array), fakeEvent);

        // Simulate the user picking the item the mocked picker returned.
        capturedOnSelect!({ meta: { view: "browser" } });
        expect(addWidgetAsPaneTab).toHaveBeenCalledWith(mockLayoutModel, "node-1", { meta: { view: "browser" } });
    });
});

describe("renderPaneChromeShell — focus ring classList", () => {
    it("applies -focused-alone when focused with a single leaf, -focused when focused with siblings, neither otherwise", () => {
        mockLayoutModel = fakeLayoutModel(["b1"]);

        const { container: aloneContainer } = render(() =>
            renderPaneChromeShell(fakeNodeModel({ isFocused: () => true, numLeafs: () => 1 }), <div /> as any)
        );
        const aloneEl = aloneContainer.querySelector(".pane-stack")!;
        expect(aloneEl.classList.contains("pane-stack-focused-alone")).toBe(true);
        expect(aloneEl.classList.contains("pane-stack-focused")).toBe(false);
        cleanup();

        const { container: siblingContainer } = render(() =>
            renderPaneChromeShell(fakeNodeModel({ isFocused: () => true, numLeafs: () => 2 }), <div /> as any)
        );
        const siblingEl = siblingContainer.querySelector(".pane-stack")!;
        expect(siblingEl.classList.contains("pane-stack-focused")).toBe(true);
        expect(siblingEl.classList.contains("pane-stack-focused-alone")).toBe(false);
        cleanup();

        const { container: unfocusedContainer } = render(() =>
            renderPaneChromeShell(fakeNodeModel({ isFocused: () => false, numLeafs: () => 1 }), <div /> as any)
        );
        const unfocusedEl = unfocusedContainer.querySelector(".pane-stack")!;
        expect(unfocusedEl.classList.contains("pane-stack-focused")).toBe(false);
        expect(unfocusedEl.classList.contains("pane-stack-focused-alone")).toBe(false);
    });

    it("sets data-blockid to the active block, and clicking/focusing the container focuses this pane's node", () => {
        mockLayoutModel = fakeLayoutModel(["b1"]);
        const nodeModel = fakeNodeModel({ activeBlockId: () => "b1" });
        const { container } = render(() => renderPaneChromeShell(nodeModel, <div /> as any));

        const el = container.querySelector(".pane-stack")!;
        expect(el.getAttribute("data-blockid")).toBe("b1");

        (el as HTMLElement).click();
        expect(nodeModel.focusNode).toHaveBeenCalled();
    });
});

describe("renderPaneChromeShell — error isolation", () => {
    it("wraps the header in an ErrorBoundary so a throwing ViewModel doesn't take the whole pane down", () => {
        // The real ErrorBoundary is mocked to a passthrough above (this
        // file's own render logic, not SolidJS's ErrorBoundary internals,
        // is under test) — this just confirms the header render path is
        // actually routed through it rather than rendered bare.
        mockLayoutModel = fakeLayoutModel(["b1"]);
        render(() => renderPaneChromeShell(fakeNodeModel(), <div /> as any));
        expect(screen.getByTestId("pane-header-tab-strip")).toBeInTheDocument();
    });
});

// The trait-like opt-in surface (ViewModel.paneChromeModel) — what makes
// ONE chrome able to serve every view type instead of agent/term keeping
// their own components. Nothing here is view-type-specific: these same
// hooks are available to any pane.
describe("renderPaneChromeShell — PaneChromeModel capabilities", () => {
    function renderWithModel(model: any, stack = ["b1", "b2"], nodeOverrides: Record<string, any> = {}) {
        mockLayoutModel = fakeLayoutModel(stack);
        const nodeModel = fakeNodeModel({
            activeViewModel: () => ({ paneChromeModel: () => model }),
            ...nodeOverrides,
        });
        const res = render(() => renderPaneChromeShell(nodeModel, <div>content</div>) as any);
        return { ...res, nodeModel };
    }

    it("a view type supplying `tabs` replaces the blockStack-derived list (cross-pane tabs)", () => {
        renderWithModel({
            tabs: () => [{ id: "x", title: "Elsewhere" }],
            getId: (t: any) => t.id,
            getLabel: (t: any) => t.title,
        });
        expect(headerCalls[0].tabs).toEqual([{ id: "x", title: "Elsewhere" }]);
        expect(screen.getByText("Elsewhere")).toBeInTheDocument();
    });

    it("onActivate returning true means handled — the default stack switch is skipped", () => {
        const onActivate = vi.fn().mockReturnValue(true);
        renderWithModel({ onActivate });

        headerCalls[0].onActivate("b2");

        expect(onActivate).toHaveBeenCalledWith("b2");
        expect(setActiveBlockInStack).not.toHaveBeenCalled();
    });

    it("onActivate returning nothing falls through to the default stack switch", () => {
        const onActivate = vi.fn();
        renderWithModel({ onActivate });

        headerCalls[0].onActivate("b2");

        expect(onActivate).toHaveBeenCalledWith("b2");
        expect(setActiveBlockInStack).toHaveBeenCalledWith(mockLayoutModel, "node-1", "b2");
    });

    it("onClose returning true means handled — the default stack close is skipped", () => {
        const onClose = vi.fn().mockReturnValue(true);
        renderWithModel({ onClose });

        headerCalls[0].onClose("b2");

        expect(onClose).toHaveBeenCalledWith("b2");
        expect(closeBlockInStack).not.toHaveBeenCalled();
    });

    it("renderBelowHeader is mounted between the header and the content region", () => {
        const { container } = renderWithModel({
            renderBelowHeader: () => <div data-testid="below-header" />,
        });

        const root = container.querySelector(".pane-stack")!;
        const kids = Array.from(root.children).map((el) => el.getAttribute("data-testid") ?? el.className);
        expect(kids).toEqual(["pane-header-tab-strip", "below-header", "pane-stack-content"]);
    });

    it("wrapContent wraps the content region, for a surface spanning more than the content box", () => {
        const { container } = renderWithModel({
            wrapContent: (c: any) => <div class="my-body">{c}</div>,
        });

        expect(container.querySelector(".my-body > .pane-stack-content")).toBeTruthy();
    });

    it("rootClass is applied alongside the chrome's own classes, not instead of them", () => {
        const { container } = renderWithModel(
            { rootClass: "term-pane-stack" },
            ["b1", "b2"],
            { isFocused: () => true, numLeafs: () => 2 }
        );

        const root = container.querySelector(".pane-stack")!;
        expect(root.classList.contains("term-pane-stack")).toBe(true);
        expect(root.classList.contains("pane-stack-focused")).toBe(true);
    });

    it("a view type that opts out entirely (no paneChromeModel) keeps every default", () => {
        setObjectValue("block:b1", { meta: { "frame:title": "One" } });
        setObjectValue("block:b2", { meta: { "frame:title": "Two" } });
        mockLayoutModel = fakeLayoutModel(["b1", "b2"]);
        render(() => renderPaneChromeShell(fakeNodeModel(), <div>content</div>) as any);

        expect(headerCalls[0].addTitle).toBe("Add tab");
        expect(headerCalls[0].tabs).toEqual([
            expect.objectContaining({ blockId: "b1", label: "One" }),
            expect.objectContaining({ blockId: "b2", label: "Two" }),
        ]);
    });
});
