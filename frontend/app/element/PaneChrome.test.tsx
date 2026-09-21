// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tests for `renderPaneChromeShell` — the ONE `ViewModel.renderPaneChrome`
 * every pane type uses. Scoped to THIS file's own wiring — the shared tab
 * model (pane-tab-model.tsx) applied to `blockStack`, rename, and the
 * activate/close/add handlers' delegation to `layoutStack.ts`'s
 * primitives — not re-testing those
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
// Keyed <For>, like the real PaneTabStrip, so DOM-identity assertions hold.
vi.mock("./PaneHeaderTabStrip", async () => {
    const { For } = await import("solid-js");
    return {
        PaneHeaderTabStrip: (props: any) => {
            headerCalls.push(props);
            return (
                <div data-testid="pane-header-tab-strip">
                    <For each={props.tabs}>
                        {(t: any) => (
                            <span data-testid={`tab-${props.getId(t)}`}>
                                {props.getIcon?.(t)}
                                {props.renderLabel ? props.renderLabel(t) : props.getLabel(t)}
                            </span>
                        )}
                    </For>
                </div>
            );
        },
    };
});

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
    // Meta-driven (not a fixed stub) so tabColors' per-block regression test
    // below can prove each tab's own color survives independently — a fixed
    // stub would pass even with reagent's P1 bug (PR #3484: every pill
    // reading the SAME shared atoms.tabAtom() collapsed every underline to
    // one tab-wide value).
    computeBlockActiveBorderColor: (meta: any) =>
        meta?.["frame:hue"] != null ? `underline-${meta["frame:hue"]}` : undefined,
    computeBlockTabPillBg: (meta: any) => (meta?.["frame:hue"] != null ? `bg-${meta["frame:hue"]}` : undefined),
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
    // tabColors' theme-polarity read (PaneChrome.tsx) — no test here cares
    // about theme, so a fixed "no theme set" accessor is enough.
    getSettingsKeyAtom: () => () => undefined,
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
// SPEC_PANE_CHROME_LAYOUT_MODEL_TAB_BINDING_2026_09_18.md — the real bug this
// spy exists to catch: `renderPaneChromeShell` used to resolve its own
// `layoutModel` via `getLayoutModelForStaticTab()` (whichever tab is
// GLOBALLY active right now), not via `nodeModel.layoutModel` (the tab THIS
// pane actually lives in). Those two silently diverge whenever a pane's
// chrome first constructs before its own tab becomes active — e.g. every
// new tab's default agent pane, seeded by `applyTabPreset` while the tab is
// still `activate: false` (`tab-actions.ts`'s `createTab()`). The
// production fix removed the import outright; this spy is the regression
// guard — it must never be called again, from any test in this file,
// including every pre-existing one above (none of them mock it a "right"
// answer, so a reintroduced call would immediately return `undefined` and
// throw when dereferenced).
const getLayoutModelForStaticTabSpy = vi.fn();
vi.mock("@/layout/index", () => ({
    addWidgetAsPaneTab: (...args: any[]) => addWidgetAsPaneTab(...args),
    closeBlockInStack: (...args: any[]) => closeBlockInStack(...args),
    setActiveBlockInStack: (...args: any[]) => setActiveBlockInStack(...args),
    getLayoutModelForStaticTab: (...args: any[]) => getLayoutModelForStaticTabSpy(...args),
}));

vi.mock("@/layout/lib/layoutNode", () => ({
    findNode: (root: any) => root,
}));

import { fireEvent } from "@solidjs/testing-library";
import { renderPaneChromeShell } from "./PaneChrome";
import { registerPaneTabDescriptor } from "./pane-tab-model";

function fakeNodeModel(overrides: Record<string, any> = {}): any {
    return {
        nodeId: "node-1",
        // `renderPaneChromeShell` reads `nodeModel.layoutModel` directly now
        // (not `getLayoutModelForStaticTab()` — see that field's own doc
        // comment, layout/lib/types.ts, and
        // SPEC_PANE_CHROME_LAYOUT_MODEL_TAB_BINDING_2026_09_18.md), so every
        // fake NodeModel must carry whichever `mockLayoutModel` its own test
        // already set up. Every call site below assigns `mockLayoutModel`
        // before calling `fakeNodeModel()`, so this always picks up the
        // right one.
        layoutModel: mockLayoutModel,
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
    getLayoutModelForStaticTabSpy.mockClear();
    capturedOnSelect = undefined;
});

afterEach(() => cleanup());

describe("renderPaneChromeShell — tab derivation", () => {
    function labels() {
        const h = headerCalls.at(-1);
        return h.tabs.map((id: string) => h.getLabel(id));
    }
    function iconClasses() {
        return screen.getAllByTestId(/^tab-/).map((el) => el.querySelector("i")?.className ?? "");
    }

    it("a lone tab is still a pill, so the header looks the same at one tab as at many", () => {
        mockLayoutModel = fakeLayoutModel(["b1"]);
        render(() => renderPaneChromeShell(fakeNodeModel(), <div>content</div>) as any);

        expect(headerCalls[0].tabs).toEqual(["b1"]);
    });

    it("a pane with no stack yet shows its own active block as the one tab", () => {
        mockLayoutModel = fakeLayoutModel([]);
        render(() => renderPaneChromeShell(fakeNodeModel({ activeBlockId: () => "b1" }), <div>content</div>) as any);

        expect(headerCalls[0].tabs).toEqual(["b1"]);
    });

    it("a 2+ member stack yields one tab per member, label from frame:title falling back to blockViewToName", () => {
        setObjectValue("block:b1", { meta: { "frame:title": "My Title" } });
        setObjectValue("block:b2", { meta: { view: "browser" } });
        mockLayoutModel = fakeLayoutModel(["b1", "b2"]);
        render(() => renderPaneChromeShell(fakeNodeModel(), <div>content</div>) as any);

        expect(headerCalls[0].tabs).toEqual(["b1", "b2"]);
        expect(labels()).toEqual(["My Title", "View:browser"]);
        expect(screen.getByText("My Title")).toBeInTheDocument();
        expect(screen.getByText("View:browser")).toBeInTheDocument();
    });

    it("gives every tab an icon: frame:icon first, falling back to blockViewToIcon(view)", () => {
        setObjectValue("block:b1", { meta: { "frame:icon": "rocket" } });
        setObjectValue("block:b2", { meta: { view: "browser" } });
        mockLayoutModel = fakeLayoutModel(["b1", "b2"]);
        render(() => renderPaneChromeShell(fakeNodeModel(), <div>content</div>) as any);

        const classes = iconClasses();
        expect(classes[0]).toContain("fa-rocket");
        expect(classes[1]).toContain("fa-icon-browser");
    });

    it("keeps each tab's icon when another tab is added", () => {
        setObjectValue("block:b1", { meta: { view: "browser" } });
        const [stack, setStack] = createSignal(["b1"]);
        mockLayoutModel = {
            localTreeStateAtom: () => stack(),
            treeState: {
                get rootNode() {
                    return { data: { blockStack: stack() } };
                },
            },
        };
        render(() => renderPaneChromeShell(fakeNodeModel(), <div>content</div>) as any);
        const before = screen.getByTestId("tab-b1").querySelector("i");
        expect(before?.className).toContain("fa-icon-browser");

        setObjectValue("block:b2", { meta: { view: "sysinfo" } });
        setStack(["b1", "b2"]);

        // Same element, not a rebuilt one: nothing about b1's icon changed.
        expect(screen.getByTestId("tab-b1").querySelector("i")).toBe(before);
        expect(screen.getByTestId("tab-b2").querySelector("i")?.className).toContain("fa-icon-sysinfo");
    });

    it("uses a registered descriptor's label and icon, numbering by position among the same view type", () => {
        registerPaneTabDescriptor("test-shell", {
            label: ({ ordinal }) => `Shell ${ordinal}`,
            icon: () => ({ kind: "fa", name: "terminal" }),
        });
        setObjectValue("block:b1", { meta: { view: "test-shell" } });
        setObjectValue("block:b2", { meta: { view: "browser" } });
        setObjectValue("block:b3", { meta: { view: "test-shell" } });
        mockLayoutModel = fakeLayoutModel(["b1", "b2", "b3"]);
        render(() => renderPaneChromeShell(fakeNodeModel(), <div>content</div>) as any);

        expect(labels()).toEqual(["Shell 1", "View:browser", "Shell 2"]);
        expect(iconClasses()[2]).toContain("fa-terminal");
    });

    it("remembers the active tab's live name once it's no longer active", () => {
        setObjectValue("block:b1", { meta: { view: "browser" } });
        setObjectValue("block:b2", { meta: { view: "browser" } });
        const [active, setActive] = createSignal("b1");
        mockLayoutModel = fakeLayoutModel(["b1", "b2"]);
        render(() =>
            renderPaneChromeShell(
                fakeNodeModel({
                    activeBlockId: active,
                    activeViewModel: () => (active() === "b1" ? { viewName: () => "Example Domain" } : null),
                }),
                <div>content</div>
            ) as any
        );
        expect(labels()[0]).toBe("Example Domain");

        setActive("b2");

        expect(labels()[0]).toBe("Example Domain");
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

// reagent P1, PR #3484: tabColors used to call computeFocusRingBorderColor
// with the SAME shared atoms.tabAtom() meta for every pill — if that tab
// had a tab-wide bg:activebordercolor override, every pill's underline
// collapsed to that one shared value instead of each block's own color.
// Fixed by switching to computeBlockActiveBorderColor, a pure per-block
// helper that never consults tab-level meta at all.
describe("renderPaneChromeShell — per-tab pane color", () => {
    it("each stack member's own color survives independently — no collapse to a shared value", () => {
        setObjectValue("block:b1", { meta: { "frame:hue": 10 } });
        setObjectValue("block:b2", { meta: { "frame:hue": 20 } });
        setObjectValue("block:b3", { meta: {} }); // no color of its own
        mockLayoutModel = fakeLayoutModel(["b1", "b2", "b3"]);
        render(() => renderPaneChromeShell(fakeNodeModel({ activeBlockId: () => "b1" }), <div>content</div>) as any);

        const h = headerCalls.at(-1);
        expect(h.getColor("b1")).toEqual({ underline: "underline-10", background: "bg-10" });
        expect(h.getColor("b2")).toEqual({ underline: "underline-20", background: "bg-20" });
        expect(h.getColor("b3")).toBeUndefined();
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
        // 4th arg is the destination pane's own newTabMeta contribution —
        // undefined here, since this pane supplies no PaneChromeModel.
        expect(addWidgetAsPaneTab).toHaveBeenCalledWith(mockLayoutModel, "node-1", { meta: { view: "browser" } }, undefined);
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

    it("extraTabs are appended after the stack, deduped against it, with their own label", () => {
        setObjectValue("block:b1", { meta: { "frame:title": "One" } });
        setObjectValue("block:b2", { meta: { "frame:title": "Two" } });
        setObjectValue("block:x", { meta: { view: "browser" } });
        renderWithModel({
            extraTabs: () => [
                { blockId: "b2", label: "Ignored: already in this pane" },
                { blockId: "x", label: "Elsewhere" },
            ],
        });
        const h = headerCalls.at(-1);
        expect(h.tabs).toEqual(["b1", "b2", "x"]);
        expect(h.getLabel("b2")).toBe("Two");
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

    it("newTabMeta reaches the add path, so a pane can carry context into the new tab", () => {
        // Terminal's cwd inheritance rides on this: the picker stays
        // view-agnostic and the destination pane contributes the params.
        renderWithModel({
            newTabMeta: (view: string) => (view === "term" ? { "cmd:cwd": "/tmp/here" } : undefined),
        });

        headerCalls[0].onAdd({ clientX: 1, clientY: 1 } as unknown as MouseEvent);
        capturedOnSelect!({ meta: { view: "term" } });

        expect(addWidgetAsPaneTab).toHaveBeenCalledWith(mockLayoutModel, "node-1", { meta: { view: "term" } }, { "cmd:cwd": "/tmp/here" });
    });

    it("contributes nothing for a view type the pane doesn't recognise", () => {
        renderWithModel({
            newTabMeta: (view: string) => (view === "term" ? { "cmd:cwd": "/tmp/here" } : undefined),
        });

        headerCalls[0].onAdd({ clientX: 1, clientY: 1 } as unknown as MouseEvent);
        capturedOnSelect!({ meta: { view: "browser" } });

        expect(addWidgetAsPaneTab).toHaveBeenCalledWith(mockLayoutModel, "node-1", { meta: { view: "browser" } }, undefined);
    });

    it("a view type that opts out entirely (no paneChromeModel) keeps every default", () => {
        setObjectValue("block:b1", { meta: { "frame:title": "One" } });
        setObjectValue("block:b2", { meta: { "frame:title": "Two" } });
        mockLayoutModel = fakeLayoutModel(["b1", "b2"]);
        render(() => renderPaneChromeShell(fakeNodeModel(), <div>content</div>) as any);

        expect(headerCalls[0].addTitle).toBe("Add tab");
        expect(headerCalls[0].tabs).toEqual(["b1", "b2"]);
        expect(screen.getByText("One")).toBeInTheDocument();
        expect(screen.getByText("Two")).toBeInTheDocument();
    });
});

describe("renderPaneChromeShell — rename", () => {
    function startRename(id: string) {
        headerCalls.at(-1).onTabDoubleClick(id);
        return screen.getByTestId(`tab-${id}`).querySelector("input") as HTMLInputElement;
    }

    it("only tabs whose descriptor offers a renamer can be renamed", () => {
        setObjectValue("block:b1", { meta: { view: "browser" } });
        mockLayoutModel = fakeLayoutModel(["b1"]);
        render(() => renderPaneChromeShell(fakeNodeModel(), <div>content</div>) as any);

        expect(startRename("b1")).toBeNull();
    });

    it("shows the new name immediately and calls the descriptor's rename", async () => {
        const rename = vi.fn().mockResolvedValue(undefined);
        registerPaneTabDescriptor("test-renamable", { label: () => "Old", renamer: () => rename });
        setObjectValue("block:b1", { meta: { view: "test-renamable" } });
        mockLayoutModel = fakeLayoutModel(["b1"]);
        render(() => renderPaneChromeShell(fakeNodeModel(), <div>content</div>) as any);

        const input = startRename("b1");
        fireEvent.input(input, { target: { value: "New" } });
        fireEvent.keyDown(input, { key: "Enter" });

        expect(rename).toHaveBeenCalledWith("New");
        expect(headerCalls.at(-1).getLabel("b1")).toBe("New");
    });

    it("rolls the name back when the rename fails", async () => {
        const rename = vi.fn().mockRejectedValue(new Error("nope"));
        registerPaneTabDescriptor("test-failing", { label: () => "Old", renamer: () => rename });
        setObjectValue("block:b1", { meta: { view: "test-failing" } });
        mockLayoutModel = fakeLayoutModel(["b1"]);
        render(() => renderPaneChromeShell(fakeNodeModel(), <div>content</div>) as any);

        const input = startRename("b1");
        fireEvent.input(input, { target: { value: "New" } });
        fireEvent.keyDown(input, { key: "Enter" });
        await Promise.resolve();
        await Promise.resolve();

        expect(headerCalls.at(-1).getLabel("b1")).toBe("Old");
    });
});

// SPEC_PANE_CHROME_LAYOUT_MODEL_TAB_BINDING_2026_09_18.md — regression
// coverage for a real, live bug: opening a brand-new tab and trying to add
// a pane tab to its default agent pane was a silent no-op. Root cause: this
// pane's chrome constructed while `applyTabPreset` was still seeding the new
// tab's default layout — BEFORE `setActiveTab` ever ran (`tab-actions.ts`'s
// `createTab()` deliberately keeps a new tab `activate: false` until its
// preset finishes, per SPEC_TAB_CREATION_REVEAL_ARCHITECTURE_2026_09_16.md).
// `renderPaneChromeShell` used to resolve its own `layoutModel` via
// `getLayoutModelForStaticTab()` (whichever tab is GLOBALLY active at that
// moment — the OLD tab), latched forever per the chrome-stability design
// (SPEC_PANE_TAB_SWITCH_CHROME_STABILITY_2026_09_07.md) — so every
// subsequent "+"/close/switch on that exact pane silently targeted the
// wrong tab's tree: `pane.open` created a real block server-side every
// time, but the pane's own `findNode` lookup could never find it (wrong
// tree), so it went straight to the "orphaned, delete it" cleanup path
// with no error surfaced to the user. Fixed by giving `NodeModel` its own
// `layoutModel` field (layout/lib/types.ts), populated at construction from
// whichever `LayoutModel` actually built it (`getNodeModel`,
// layoutNodeModels.ts) — correct by construction, no timing dependency.
describe("renderPaneChromeShell — resolves the OWNING tab's LayoutModel, never the globally-active one", () => {
    // The direct repro: the node's own tab (`nodeModel.layoutModel`) is a
    // DIFFERENT object than whatever "globally active tab" lookup would
    // return — simulating a pane whose chrome constructed before its own
    // tab became active. Every mutating action must operate on the node's
    // own tab regardless.
    function distinctWrongModel(): any {
        return {
            localTreeStateAtom: () => {},
            treeState: { rootNode: { data: { blockStack: ["WRONG-TAB-BLOCK"] } } },
        };
    }

    it("never calls getLayoutModelForStaticTab at all — not on render, not on add/close/activate", () => {
        mockLayoutModel = fakeLayoutModel(["b1", "b2"]);
        getLayoutModelForStaticTabSpy.mockReturnValue(distinctWrongModel());
        render(() => renderPaneChromeShell(fakeNodeModel(), <div>content</div>) as any);

        headerCalls[0].onActivate("b2");
        headerCalls[0].onClose("b2");
        headerCalls[0].onAdd({ clientX: 1, clientY: 1 } as unknown as MouseEvent);
        capturedOnSelect!({ meta: { view: "browser" } });

        expect(getLayoutModelForStaticTabSpy).not.toHaveBeenCalled();
    });

    it("onActivate targets the node's own layoutModel even when the global lookup would return a different one", () => {
        mockLayoutModel = fakeLayoutModel(["b1", "b2"]);
        const wrongModel = distinctWrongModel();
        getLayoutModelForStaticTabSpy.mockReturnValue(wrongModel);
        render(() => renderPaneChromeShell(fakeNodeModel({ activeBlockId: () => "b1" }), <div>content</div>) as any);

        headerCalls[0].onActivate("b2");

        expect(setActiveBlockInStack).toHaveBeenCalledWith(mockLayoutModel, "node-1", "b2");
        // Direct identity check against the exact sentinel the global lookup
        // was configured to return — captured in its own variable rather
        // than read back off the spy, since the whole point of the OTHER
        // test in this suite is that the spy is never even called (so
        // `.mock.results` would be empty). An `objectContaining({ treeState:
        // ... })` shape check would be useless here regardless: both the
        // right and wrong model have a `treeState`, so it could never
        // actually distinguish them.
        expect(setActiveBlockInStack.mock.calls[0][0]).not.toBe(wrongModel);
    });

    it("onClose targets the node's own layoutModel even when the global lookup would return a different one", () => {
        mockLayoutModel = fakeLayoutModel(["b1", "b2"]);
        getLayoutModelForStaticTabSpy.mockReturnValue(distinctWrongModel());
        render(() => renderPaneChromeShell(fakeNodeModel(), <div>content</div>) as any);

        headerCalls[0].onClose("b2");

        expect(closeBlockInStack).toHaveBeenCalledWith(mockLayoutModel, "node-1", "b2");
    });

    it("onAdd (the '+' widget picker) targets the node's own layoutModel even when the global lookup would return a different one — the exact user-facing bug", () => {
        mockLayoutModel = fakeLayoutModel(["b1"]);
        getLayoutModelForStaticTabSpy.mockReturnValue(distinctWrongModel());
        render(() => renderPaneChromeShell(fakeNodeModel(), <div>content</div>) as any);

        headerCalls[0].onAdd({ clientX: 1, clientY: 1 } as unknown as MouseEvent);
        capturedOnSelect!({ meta: { view: "agent" } });

        expect(addWidgetAsPaneTab).toHaveBeenCalledWith(mockLayoutModel, "node-1", { meta: { view: "agent" } }, undefined);
    });
});
