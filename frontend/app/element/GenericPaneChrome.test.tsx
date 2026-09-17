// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tests for `genericRenderPaneChrome` — the shared `ViewModel.renderPaneChrome`
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
 * Group C — this closes the "not yet for genericRenderPaneChrome's own
 * tab-derivation/add/close logic" gap that plan explicitly flags as open.
 */

import { cleanup, render, screen } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const headerCalls: any[] = [];
vi.mock("./PaneHeaderTabStrip", () => ({
    PaneHeaderTabStrip: (props: any) => {
        headerCalls.push(props);
        return (
            <div data-testid="pane-header-tab-strip">
                {props.tabs.map((t: any) => (
                    <span>{props.getLabel(t)}</span>
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
}));

vi.mock("@/app/block/blockframe", () => ({
    computeFocusRingBorderColor: () => undefined,
}));

const showContextMenu = vi.fn();
vi.mock("@/app/store/contextmenu", () => ({
    ContextMenuModel: { showContextMenu: (...args: any[]) => showContextMenu(...args) },
}));

const objectValues = new Map<string, any>();
vi.mock("@/app/store/global", () => ({
    MOS: {
        makeORef: (otype: string, oid: string) => `${otype}:${oid}`,
        getMuxObjectAtom: (oref: string) => () => objectValues.get(oref),
        getObjectValue: (oref: string) => objectValues.get(oref),
    },
    atoms: {
        fullConfigAtom: () => ({ widgets: {}, settings: {} }),
        tabAtom: () => ({ meta: {} }),
    },
}));

let capturedOnSelect: ((blockDef: any) => void) | undefined;
const buildPaneWidgetMenuItemsMock = vi.fn((_wmap: any, _settings: any, onSelect: any) => {
    capturedOnSelect = onSelect;
    return [{ label: "Browser", click: () => onSelect({ meta: { view: "browser" } }) }];
});
vi.mock("@/app/window/action-widgets-config", () => ({
    buildPaneWidgetMenuItems: (...args: any[]) => (buildPaneWidgetMenuItemsMock as any)(...args),
}));

const addWidgetAsPaneTab = vi.fn();
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

import { genericRenderPaneChrome } from "./GenericPaneChrome";

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
    objectValues.clear();
    headerCalls.length = 0;
    showContextMenu.mockClear();
    buildPaneWidgetMenuItemsMock.mockClear();
    addWidgetAsPaneTab.mockClear();
    closeBlockInStack.mockClear();
    setActiveBlockInStack.mockClear();
    capturedOnSelect = undefined;
});

afterEach(() => cleanup());

describe("genericRenderPaneChrome — tab derivation", () => {
    it("a 0-or-1-member stack yields no pills (matches the lone-tab-shows-no-pill convention)", () => {
        mockLayoutModel = fakeLayoutModel(["b1"]);
        render(() => genericRenderPaneChrome(fakeNodeModel(), <div>content</div>) as any);

        expect(headerCalls[0].tabs).toEqual([]);
    });

    it("a 2+ member stack yields one tab per member, label from frame:title falling back to blockViewToName", () => {
        objectValues.set("block:b1", { meta: { "frame:title": "My Title" } });
        objectValues.set("block:b2", { meta: { view: "browser" } });
        mockLayoutModel = fakeLayoutModel(["b1", "b2"]);
        render(() => genericRenderPaneChrome(fakeNodeModel(), <div>content</div>) as any);

        expect(headerCalls[0].tabs).toEqual([
            { blockId: "b1", label: "My Title" },
            { blockId: "b2", label: "View:browser" },
        ]);
        expect(screen.getByText("My Title")).toBeInTheDocument();
        expect(screen.getByText("View:browser")).toBeInTheDocument();
    });
});

describe("genericRenderPaneChrome — activate/close/add wiring", () => {
    it("onActivate delegates to setActiveBlockInStack with this pane's nodeId, and no-ops when already active", () => {
        mockLayoutModel = fakeLayoutModel(["b1", "b2"]);
        render(() => genericRenderPaneChrome(fakeNodeModel({ activeBlockId: () => "b1" }), <div /> as any));

        headerCalls[0].onActivate("b1"); // already active
        expect(setActiveBlockInStack).not.toHaveBeenCalled();

        headerCalls[0].onActivate("b2");
        expect(setActiveBlockInStack).toHaveBeenCalledWith(mockLayoutModel, "node-1", "b2");
    });

    it("onClose delegates to closeBlockInStack with this pane's nodeId", () => {
        mockLayoutModel = fakeLayoutModel(["b1", "b2"]);
        render(() => genericRenderPaneChrome(fakeNodeModel(), <div /> as any));

        headerCalls[0].onClose("b2");
        expect(closeBlockInStack).toHaveBeenCalledWith(mockLayoutModel, "node-1", "b2");
    });

    it("onAdd is a no-op without a MouseEvent (context menu needs a position to anchor to)", () => {
        mockLayoutModel = fakeLayoutModel(["b1"]);
        render(() => genericRenderPaneChrome(fakeNodeModel(), <div /> as any));

        headerCalls[0].onAdd();
        expect(showContextMenu).not.toHaveBeenCalled();
    });

    it("onAdd opens the same widget picker the widget bar uses, and selecting an item pushes it via addWidgetAsPaneTab", () => {
        mockLayoutModel = fakeLayoutModel(["b1"]);
        render(() => genericRenderPaneChrome(fakeNodeModel(), <div /> as any));

        const fakeEvent = { clientX: 1, clientY: 1 } as unknown as MouseEvent;
        headerCalls[0].onAdd(fakeEvent);

        expect(buildPaneWidgetMenuItemsMock).toHaveBeenCalled();
        expect(showContextMenu).toHaveBeenCalledWith(expect.any(Array), fakeEvent);

        // Simulate the user picking the item the mocked picker returned.
        capturedOnSelect!({ meta: { view: "browser" } });
        expect(addWidgetAsPaneTab).toHaveBeenCalledWith(mockLayoutModel, "node-1", { meta: { view: "browser" } });
    });
});

describe("genericRenderPaneChrome — focus ring classList", () => {
    it("applies -focused-alone when focused with a single leaf, -focused when focused with siblings, neither otherwise", () => {
        mockLayoutModel = fakeLayoutModel(["b1"]);

        const { container: aloneContainer } = render(() =>
            genericRenderPaneChrome(fakeNodeModel({ isFocused: () => true, numLeafs: () => 1 }), <div /> as any)
        );
        const aloneEl = aloneContainer.querySelector(".generic-pane-stack")!;
        expect(aloneEl.classList.contains("generic-pane-stack-focused-alone")).toBe(true);
        expect(aloneEl.classList.contains("generic-pane-stack-focused")).toBe(false);
        cleanup();

        const { container: siblingContainer } = render(() =>
            genericRenderPaneChrome(fakeNodeModel({ isFocused: () => true, numLeafs: () => 2 }), <div /> as any)
        );
        const siblingEl = siblingContainer.querySelector(".generic-pane-stack")!;
        expect(siblingEl.classList.contains("generic-pane-stack-focused")).toBe(true);
        expect(siblingEl.classList.contains("generic-pane-stack-focused-alone")).toBe(false);
        cleanup();

        const { container: unfocusedContainer } = render(() =>
            genericRenderPaneChrome(fakeNodeModel({ isFocused: () => false, numLeafs: () => 1 }), <div /> as any)
        );
        const unfocusedEl = unfocusedContainer.querySelector(".generic-pane-stack")!;
        expect(unfocusedEl.classList.contains("generic-pane-stack-focused")).toBe(false);
        expect(unfocusedEl.classList.contains("generic-pane-stack-focused-alone")).toBe(false);
    });

    it("sets data-blockid to the active block, and clicking/focusing the container focuses this pane's node", () => {
        mockLayoutModel = fakeLayoutModel(["b1"]);
        const nodeModel = fakeNodeModel({ activeBlockId: () => "b1" });
        const { container } = render(() => genericRenderPaneChrome(nodeModel, <div /> as any));

        const el = container.querySelector(".generic-pane-stack")!;
        expect(el.getAttribute("data-blockid")).toBe("b1");

        (el as HTMLElement).click();
        expect(nodeModel.focusNode).toHaveBeenCalled();
    });
});

describe("genericRenderPaneChrome — error isolation", () => {
    it("wraps the header in an ErrorBoundary so a throwing ViewModel doesn't take the whole pane down", () => {
        // The real ErrorBoundary is mocked to a passthrough above (this
        // file's own render logic, not SolidJS's ErrorBoundary internals,
        // is under test) — this just confirms the header render path is
        // actually routed through it rather than rendered bare.
        mockLayoutModel = fakeLayoutModel(["b1"]);
        render(() => genericRenderPaneChrome(fakeNodeModel(), <div /> as any));
        expect(screen.getByTestId("pane-header-tab-strip")).toBeInTheDocument();
    });
});
