// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * ActionWidgets, the widget bar: the pinned slots and their drag/drop and context-menu
 * state, the More dropdown and the pinned-parent flyouts with their shared hover-intent
 * peers, the item menu, and the measuring mirrors. Its helpers (the responsive hook, the
 * drag hook, the dropdown, the flyout, the menus) have their own homes; here they are probes
 * that record what the bar hands them and let a test call back into it.
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createSignal, type Accessor, type JSX, type Setter } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

type Entry = { key: string; widget: WidgetConfigType };
type Wmap = Record<string, WidgetConfigType>;
type Settings = Record<string, unknown>;
type Pos = { x: number; y: number };
type PointerLike = { clientX: number; clientY: number };
type MenuItem = { label?: string; type?: string; icon?: string; click?: () => void };
type TrackedEl = { getBoundingClientRect(): Record<string, number> };
type ResponsiveOpts = {
    containerRef(): HTMLElement | undefined;
    moreButtonRef(): HTMLElement | undefined;
    pinnedWidgets(): Entry[];
    moreWidgets(): Entry[];
    iconOnly(): boolean;
};
type DragOpts = { containerRef(): HTMLElement | undefined; pinnedWidgets(): Entry[]; savedPins(): string[] };
type HoverProbe = {
    onClose(): void;
    onItemContextMenu(pos: Pos, key: string): void;
    anchor(): HTMLElement | null;
    settings(): Settings;
    wmap(): Wmap;
    onSubmenuEnter(): void;
    onSubmenuLeave(e: PointerLike): void;
    setSubmenuEl(el: TrackedEl | null): void;
    ref?: (el: HTMLDivElement) => void;
};
type MoreProbe = HoverProbe & { widgets(): Entry[]; isItemMenuOpen(): boolean };
type FlyoutProbe = HoverProbe & { widget: WidgetConfigType };
type PopoverProbe = { pos: Pos; items: MenuItem[]; onClose(): void };

const h = vi.hoisted(() => {
    return {
        config: null as unknown as Accessor<{ settings: Settings; widgets: Wmap }>,
        setConfig: null as unknown as Setter<{ settings: Settings; widgets: Wmap }>,
        showLabels: null as unknown as Accessor<boolean>,
        setShowLabels: null as unknown as Setter<boolean>,
        visibleOverride: null as unknown as Accessor<Entry[] | null>,
        setVisibleOverride: null as unknown as Setter<Entry[] | null>,
        clipped: null as unknown as Accessor<Entry[]>,
        setClipped: null as unknown as Setter<Entry[]>,
        clipCount: null as unknown as Accessor<number>,
        setClipCount: null as unknown as Setter<number>,
        draggingKey: null as unknown as Accessor<string | null>,
        setDraggingKey: null as unknown as Setter<string | null>,
        dropIndex: null as unknown as Accessor<number | null>,
        setDropIndex: null as unknown as Setter<number | null>,
        responsiveOpts: null as unknown as ResponsiveOpts,
        dragOpts: null as unknown as DragOpts,
        refs: {} as Record<string, HTMLElement | undefined>,
        more: null as unknown as MoreProbe,
        flyout: null as unknown as FlyoutProbe,
        popover: null as unknown as PopoverProbe,
        pointer: {
            down: vi.fn(),
            move: vi.fn(),
            up: vi.fn(),
            cancel: vi.fn(),
        },
        openWidget: vi.fn(),
        pinWidget: vi.fn(),
        unpinWidget: vi.fn(),
        hideFromWidgetBar: vi.fn(),
        openMenuAt: vi.fn(),
        closeLogoMenu: vi.fn(),
        openActions: vi.fn(),
        runAction: vi.fn(),
    };
});

vi.mock("@/store/global", () => ({ atoms: { fullConfigAtom: () => h.config() } }));
vi.mock("@/app/store/block-layout-actions", () => ({ createBlock: vi.fn() }));
vi.mock("@/app/store/block-component-registry", () => ({ openOrFocusPaneByView: vi.fn() }));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { SetConfigCommand: vi.fn() } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("./action-widgets-config", async (importOriginal) => ({
    ...(await importOriginal<typeof import("./action-widgets-config")>()),
    openWidget: h.openWidget,
    pinWidget: h.pinWidget,
    unpinWidget: h.unpinWidget,
}));
vi.mock("./action-widgets-menu", () => ({ buildWidgetOpenActions: h.openActions }));
vi.mock("./widget-bar-hide", () => ({ hideFromWidgetBar: h.hideFromWidgetBar }));
vi.mock("./logo-menu", () => ({ closeLogoMenu: h.closeLogoMenu }));
vi.mock("@/app/element/ui", () => ({ openMenuAt: h.openMenuAt }));
vi.mock("@/app/element/tooltip", () => ({
    Tooltip: (p: {
        content?: unknown;
        placement?: string;
        divClassName?: string;
        divOnClick?: (e: MouseEvent) => void;
        children?: JSX.Element;
    }) => (
        <div
            data-testid="tip"
            data-placement={p.placement}
            data-tip={typeof p.content === "string" ? p.content : ""}
            class={p.divClassName}
            onClick={(e) => p.divOnClick?.(e)}
        >
            {p.children}
        </div>
    ),
}));
vi.mock("@/app/view/kanban/kanban-needs-badge", () => ({
    KanbanNeedsBadge: () => <i data-testid="kanban-badge" />,
    opensKanban: (w: { label?: string } | undefined) => w?.label === "Kanban",
}));
vi.mock("@/app/element/popover-menu", () => ({
    PopoverMenu: (p: PopoverProbe) => {
        h.popover = p;
        return (
            <div data-testid="popover" class="popover-menu" data-x={p.pos.x} data-y={p.pos.y}>
                {p.items.map((it) =>
                    it.type === "separator" ? <hr /> : <button onClick={() => it.click?.()}>{it.label}</button>
                )}
            </div>
        );
    },
}));
vi.mock("./use-widget-bar-responsive", () => ({
    useWidgetBarResponsive: (opts: ResponsiveOpts) => {
        h.responsiveOpts = opts;
        return {
            showWidgetLabels: h.showLabels,
            visiblePinnedWidgets: () => h.visibleOverride() ?? opts.pinnedWidgets(),
            clippedPinnedWidgets: h.clipped,
            clipCount: h.clipCount,
            setMirrorRef: (el: HTMLElement) => (h.refs.mirror = el),
            setIconMirrorRef: (el: HTMLElement) => (h.refs.iconMirror = el),
            setIconMirrorMoreRef: (el: HTMLElement) => (h.refs.moreProbe = el),
        };
    },
}));
vi.mock("./use-widget-drag-reorder", () => ({
    useWidgetDragReorder: (opts: DragOpts) => {
        h.dragOpts = opts;
        return {
            draggingKey: h.draggingKey,
            dropIndex: h.dropIndex,
            handlePointerDown: h.pointer.down,
            handlePointerMove: h.pointer.move,
            handlePointerUp: h.pointer.up,
            handlePointerCancel: h.pointer.cancel,
        };
    },
}));
vi.mock("./more-dropdown", () => ({
    MoreDropdown: (p: MoreProbe) => {
        h.more = p;
        return (
            <div data-testid="more-dropdown" ref={(el) => p.ref?.(el)}>
                {p
                    .widgets()
                    .map((w) => w.key)
                    .join(",")}
            </div>
        );
    },
}));
vi.mock("./pinned-widget-flyout", () => ({
    PinnedWidgetFlyout: (p: FlyoutProbe) => {
        h.flyout = p;
        return (
            <div data-testid="flyout" ref={(el) => p.ref?.(el)}>
                {p.widget?.label}
            </div>
        );
    },
}));

import { ActionWidgets } from "./action-widgets";

[h.config, h.setConfig] = createSignal<{ settings: Settings; widgets: Wmap }>({ settings: {}, widgets: {} });
[h.showLabels, h.setShowLabels] = createSignal(true);
[h.visibleOverride, h.setVisibleOverride] = createSignal<Entry[] | null>(null);
[h.clipped, h.setClipped] = createSignal<Entry[]>([]);
[h.clipCount, h.setClipCount] = createSignal(0);
[h.draggingKey, h.setDraggingKey] = createSignal<string | null>(null);
[h.dropIndex, h.setDropIndex] = createSignal<number | null>(null);

const leaf = (label: string, order: number, extra: Record<string, unknown> = {}): WidgetConfigType =>
    ({
        label,
        icon: label.toLowerCase(),
        "display:order": order,
        blockdef: { meta: { view: label.toLowerCase() } },
        ...extra,
    }) as unknown as WidgetConfigType;

const WIDGETS: Wmap = {
    "defwidget@agent": leaf("Agent", 1, { "display:pinned": true, description: "Talk to an agent" }),
    "defwidget@term": leaf("Terminal", 2, { "display:pinned": true }),
    "defwidget@msgr": leaf("Messengers", 3, { "display:pinned": true, children: ["discord"] }),
    "defwidget@discord": leaf("Discord", 4),
    "defwidget@notes": leaf("Notes", 5),
    "defwidget@settings": leaf("Settings", 6, { "display:pinned": true }),
};

const SETTINGS: Settings = {};

function setBar(settings: Settings = SETTINGS, widgets: Wmap = WIDGETS) {
    h.setConfig({ settings, widgets });
}

const bar = (): HTMLElement => document.querySelector(".action-widgets[data-testid='action-widgets']")!;
const slots = (): HTMLElement[] => [...bar().querySelectorAll<HTMLElement>(".action-widget-slot")];
const slotOf = (label: string): HTMLElement => slots().find((s) => s.textContent?.includes(label))!;
const tipOf = (label: string): HTMLElement => slotOf(label).querySelector<HTMLElement>("[data-testid='tip']")!;
const triggerOf = (label: string): HTMLElement => tipOf(label).parentElement!;
const moreBtn = (): HTMLElement | null => bar().querySelector<HTMLElement>(".action-widget-more-btn");
const mouseEnter = (el: Element) => fireEvent(el, new MouseEvent("mouseenter"));
const mouseLeave = (el: Element, x = 0, y = 0) =>
    fireEvent(el, new MouseEvent("mouseleave", { clientX: x, clientY: y }));
const flyoutUp = () => screen.queryByTestId("flyout");
const moreUp = () => screen.queryByTestId("more-dropdown");
const tick = (ms: number) => vi.advanceTimersByTime(ms);

/** Hover a pinned parent until its flyout opens. */
function hoverOpenParent(label = "Messengers") {
    mouseEnter(triggerOf(label));
    tick(90);
}

/** Hover the More button until the dropdown opens. */
function hoverOpenMore() {
    mouseEnter(moreBtn()!);
    tick(90);
}

beforeEach(() => {
    vi.useFakeTimers();
    setBar();
    h.setShowLabels(true);
    h.setVisibleOverride(null);
    h.setClipped([]);
    h.setClipCount(0);
    h.setDraggingKey(null);
    h.setDropIndex(null);
    h.refs = {};
    h.more = null;
    h.flyout = null;
    h.popover = null;
    h.openActions.mockImplementation(() => [
        { label: "Open in New Window", icon: "win", run: h.runAction },
        { label: "Open in New Tab", icon: "tab", run: h.runAction },
    ]);
});

afterEach(() => {
    cleanup();
    vi.clearAllMocks();
    vi.useRealTimers();
});

describe("the bar", () => {
    it("shows a slot for each visible pinned widget, in order, numbered", () => {
        render(() => <ActionWidgets />);
        expect(slots().map((s) => s.textContent)).toEqual(["Agent", "Terminal", "Messengers", "Settings"]);
        expect(slots().map((s) => s.getAttribute("data-widget-slot"))).toEqual(["0", "1", "2", "3"]);
        expect(bar().getAttribute("data-drag-region")).toBe("false");
    });

    it("shows what the responsive hook says is visible, not every pinned widget", () => {
        render(() => <ActionWidgets />);
        h.setVisibleOverride([{ key: "defwidget@term", widget: WIDGETS["defwidget@term"] }]);
        expect(slots().map((s) => s.textContent)).toEqual(["Terminal"]);
    });

    it("follows the config", () => {
        render(() => <ActionWidgets />);
        setBar({ "widget:pinned": ["term"] });
        expect(slots().map((s) => s.textContent)).toEqual(["Terminal"]);
    });

    it("gives each widget its description as tooltip, else its label, below it", () => {
        render(() => <ActionWidgets />);
        expect(tipOf("Agent").getAttribute("data-placement")).toBe("bottom");
        expect(tipOf("Agent").getAttribute("data-tip")).toBe("Talk to an agent");
        expect(tipOf("Terminal").getAttribute("data-tip")).toBe("Terminal");
    });

    it("hides the labels when the bar says so, and when a label is blank", () => {
        setBar(SETTINGS, { ...WIDGETS, "defwidget@term": { ...WIDGETS["defwidget@term"], label: "" } });
        render(() => <ActionWidgets />);
        expect(slotOf("Agent").querySelector(".text-xs")?.textContent).toBe("Agent");
        expect(slots()[1].querySelector(".text-xs")).toBeNull();
        expect(slots()[1].querySelector(".widget-icon")).not.toBeNull();
        h.setShowLabels(false);
        expect(slots()[0].querySelector(".text-xs")).toBeNull();
        expect(slots()[0].querySelector(".widget-icon")).not.toBeNull();
    });

    it("marks a group with a chevron and a Kanban widget with its badge", () => {
        setBar(SETTINGS, { ...WIDGETS, "defwidget@term": { ...WIDGETS["defwidget@term"], label: "Kanban" } });
        render(() => <ActionWidgets />);
        expect(slotOf("Messengers").querySelector(".action-widget-parent-chevron")).not.toBeNull();
        expect(slotOf("Agent").querySelector(".action-widget-parent-chevron")).toBeNull();
        expect(slotOf("Kanban").querySelector("[data-testid='kanban-badge']")).not.toBeNull();
        expect(slotOf("Agent").querySelector("[data-testid='kanban-badge']")).toBeNull();
    });

    it("tints the icon from the widget's colour and falls back to a default icon", () => {
        setBar(SETTINGS, {
            ...WIDGETS,
            "defwidget@agent": { ...WIDGETS["defwidget@agent"], icon: undefined, color: "#112233" },
        });
        render(() => <ActionWidgets />);
        const icon = slotOf("Agent").querySelector<HTMLElement>(".widget-icon")!;
        expect(icon.querySelector("i")!.className).toContain("window-maximize");
        expect(icon.getAttribute("style")).toContain("--widget-tint");
        expect(slotOf("Terminal").querySelector(".widget-icon")!.getAttribute("style") ?? "").not.toContain(
            "--widget-tint"
        );
    });

    it("hands the hooks the bar, the More button, the pins and the icon-only setting", () => {
        setBar({ "widget:icononly": true });
        render(() => <ActionWidgets />);
        expect(h.responsiveOpts.containerRef()).toBe(bar());
        expect(h.responsiveOpts.moreButtonRef()).toBe(moreBtn());
        expect(h.responsiveOpts.iconOnly()).toBe(true);
        expect(h.responsiveOpts.pinnedWidgets().map((e) => e.key)).toEqual([
            "defwidget@agent",
            "defwidget@term",
            "defwidget@msgr",
            "defwidget@settings",
        ]);
        expect(h.responsiveOpts.moreWidgets().map((e) => e.key)).toEqual(["defwidget@notes"]);
        expect(h.dragOpts.containerRef()).toBe(bar());
        expect(h.dragOpts.pinnedWidgets().map((e) => e.key)).toEqual(
            h.responsiveOpts.pinnedWidgets().map((e) => e.key)
        );
        expect(h.dragOpts.savedPins()).toEqual(["agent", "term", "msgr", "settings"]);
    });

    it("lets a drag keep a pin that is hidden right now", () => {
        setBar({ "widget:pinned": ["agent", "term", "settings"], "widget:hidden": ["defwidget@term"] });
        render(() => <ActionWidgets />);
        expect(h.dragOpts.pinnedWidgets().map((e) => e.key)).toEqual(["defwidget@agent", "defwidget@settings"]);
        expect(h.dragOpts.savedPins()).toEqual(["agent", "term", "settings"]);
    });

    it("is not icon-only unless the setting says so", () => {
        render(() => <ActionWidgets />);
        expect(h.responsiveOpts.iconOnly()).toBe(false);
    });

    it("renders nothing for an empty config", () => {
        setBar({}, {});
        render(() => <ActionWidgets />);
        expect(slots()).toHaveLength(0);
        expect(moreBtn()).toBeNull();
    });

    it("tolerates a config that has not loaded", () => {
        h.setConfig(undefined as unknown as { settings: Settings; widgets: Wmap });
        render(() => <ActionWidgets />);
        expect(slots()).toHaveLength(0);
    });
});

describe("a slot's pointer and click handling", () => {
    it("passes pointer events to the drag hook, with the slot's key on a press", () => {
        render(() => <ActionWidgets />);
        const slot = slotOf("Terminal");
        fireEvent.pointerDown(slot);
        expect(h.pointer.down).toHaveBeenCalledTimes(1);
        expect(h.pointer.down.mock.calls[0][0]).toBe("defwidget@term");
        fireEvent.pointerMove(slot);
        fireEvent.pointerUp(slot);
        fireEvent.pointerCancel(slot);
        expect(h.pointer.move).toHaveBeenCalledTimes(1);
        expect(h.pointer.up).toHaveBeenCalledTimes(1);
        expect(h.pointer.cancel).toHaveBeenCalledTimes(1);
    });

    it("opens a leaf widget on click, in a new pane on shift-click", () => {
        render(() => <ActionWidgets />);
        fireEvent.click(tipOf("Terminal"));
        expect(h.openWidget).toHaveBeenLastCalledWith(WIDGETS["defwidget@term"], { forceNew: false });
        fireEvent.click(tipOf("Terminal"), { shiftKey: true });
        expect(h.openWidget).toHaveBeenLastCalledWith(WIDGETS["defwidget@term"], { forceNew: true });
    });

    it("marks the slot being dragged", () => {
        render(() => <ActionWidgets />);
        expect(slotOf("Terminal").classList.contains("dragging")).toBe(false);
        h.setDraggingKey("defwidget@term");
        expect(slotOf("Terminal").classList.contains("dragging")).toBe(true);
        expect(slotOf("Agent").classList.contains("dragging")).toBe(false);
    });

    it("draws the drop indicator before the slot at the drop index, never beside the dragged one", () => {
        render(() => <ActionWidgets />);
        const indicators = () => bar().querySelectorAll(".action-widget-drop-indicator");
        expect(indicators()).toHaveLength(0);
        h.setDraggingKey("defwidget@agent");
        h.setDropIndex(2);
        expect(indicators()).toHaveLength(1);
        expect(slotOf("Messengers").previousElementSibling?.className).toBe("action-widget-drop-indicator");
        h.setDropIndex(0);
        expect(indicators()).toHaveLength(0);
        h.setDraggingKey("defwidget@term");
        expect(indicators()).toHaveLength(1);
        expect(slotOf("Agent").previousElementSibling?.className).toBe("action-widget-drop-indicator");
    });

    it("draws the end indicator after the last visible slot when some pins are clipped", () => {
        render(() => <ActionWidgets />);
        h.setVisibleOverride([
            { key: "defwidget@agent", widget: WIDGETS["defwidget@agent"] },
            { key: "defwidget@term", widget: WIDGETS["defwidget@term"] },
        ]);
        h.setDraggingKey("defwidget@agent");
        h.setDropIndex(2);
        const indicators = bar().querySelectorAll(".action-widget-drop-indicator");
        expect(indicators).toHaveLength(1);
        expect(slotOf("Terminal").nextElementSibling).toBe(indicators[0]);
        h.setDropIndex(4);
        expect(bar().querySelectorAll(".action-widget-drop-indicator")).toHaveLength(0);
    });

    it("draws the drop indicator after the last slot for a drop at the end", () => {
        render(() => <ActionWidgets />);
        h.setDraggingKey("defwidget@agent");
        h.setDropIndex(4);
        const indicators = bar().querySelectorAll(".action-widget-drop-indicator");
        expect(indicators).toHaveLength(1);
        expect(slotOf("Settings").nextElementSibling).toBe(indicators[0]);
    });
});

describe("the More button and the measuring mirrors", () => {
    it("shows More while widgets are unpinned or some pinned ones are clipped", () => {
        render(() => <ActionWidgets />);
        expect(moreBtn()).not.toBeNull();
        setBar({ "widget:pinned": ["agent", "term", "msgr", "settings", "notes"] });
        expect(moreBtn()).toBeNull();
        h.setClipCount(1);
        expect(moreBtn()).not.toBeNull();
    });

    it("labels it only while labels show, and flips its chevron when open", () => {
        render(() => <ActionWidgets />);
        expect(moreBtn()!.querySelector(".action-widget-more-label")?.textContent).toBe("more");
        expect(moreBtn()!.querySelector(".fa-chevron-down")).not.toBeNull();
        h.setShowLabels(false);
        expect(moreBtn()!.querySelector(".action-widget-more-label")).toBeNull();
        fireEvent.click(moreBtn()!);
        expect(moreBtn()!.classList.contains("open")).toBe(true);
        expect(moreBtn()!.querySelector(".fa-chevron-up")).not.toBeNull();
        expect(moreBtn()!.querySelector(".fa-chevron-down")).toBeNull();
    });

    it("mounts the three hidden mirrors and gives their elements to the hook", () => {
        render(() => <ActionWidgets />);
        const mirrors = [...document.querySelectorAll(".action-widgets--measure")];
        expect(mirrors).toHaveLength(3);
        for (const m of mirrors) expect(m.getAttribute("aria-hidden")).toBe("true");
        expect(h.refs.mirror).toBe(mirrors[0]);
        expect(h.refs.iconMirror).toBe(mirrors[1]);
        expect(h.refs.moreProbe).toBe(mirrors[2].querySelector(".action-widget-more-btn"));
    });

    it("measures the full pins in both mirrors, labelled in the first and icon-only in the second", () => {
        render(() => <ActionWidgets />);
        h.setShowLabels(false);
        h.setVisibleOverride([]);
        const [labelled, iconOnly] = [...document.querySelectorAll(".action-widgets--measure")];
        expect([...labelled.querySelectorAll(".action-widget-slot")].map((s) => s.textContent)).toEqual([
            "Agent",
            "Terminal",
            "Messengers",
            "Settings",
        ]);
        expect([...iconOnly.querySelectorAll(".action-widget-slot")].map((s) => s.textContent)).toEqual([
            "",
            "",
            "",
            "",
        ]);
        expect(labelled.querySelector(".action-widget-more-label")?.textContent).toBe("more");
        expect(iconOnly.querySelector(".action-widget-more-label")).toBeNull();
        expect(labelled.querySelector(".action-widget-more-btn")).not.toBeNull();
        expect(iconOnly.querySelector(".action-widget-more-btn")).not.toBeNull();
    });

    it("leaves the More button out of the mirrors when nothing is unpinned", () => {
        setBar({ "widget:pinned": ["agent", "term", "msgr", "settings", "notes"] });
        h.setClipCount(2);
        render(() => <ActionWidgets />);
        const [labelled, iconOnly, probe] = [...document.querySelectorAll(".action-widgets--measure")];
        expect(labelled.querySelector(".action-widget-more-btn")).toBeNull();
        expect(iconOnly.querySelector(".action-widget-more-btn")).toBeNull();
        expect(probe.querySelector(".action-widget-more-btn")).not.toBeNull();
        expect(moreBtn()).not.toBeNull();
    });
});

describe("the More dropdown", () => {
    it("opens after the hover delay, closes the AgentMux logo, and lists the clipped pins before the rest", () => {
        h.setClipped([{ key: "defwidget@settings", widget: WIDGETS["defwidget@settings"] }]);
        h.setClipCount(1);
        render(() => <ActionWidgets />);
        mouseEnter(moreBtn()!);
        tick(89);
        expect(moreUp()).toBeNull();
        expect(h.closeLogoMenu).not.toHaveBeenCalled();
        tick(1);
        expect(moreUp()).not.toBeNull();
        expect(h.closeLogoMenu).toHaveBeenCalledTimes(1);
        expect(moreUp()!.textContent).toBe("defwidget@settings,defwidget@notes");
    });

    it("opens at once on a click", () => {
        render(() => <ActionWidgets />);
        fireEvent.click(moreBtn()!);
        expect(moreUp()).not.toBeNull();
        expect(h.closeLogoMenu).toHaveBeenCalledTimes(1);
    });

    it("is anchored to the More button, with the config and the item-menu flag", () => {
        render(() => <ActionWidgets />);
        fireEvent.click(moreBtn()!);
        expect(h.more.anchor()).toBe(moreBtn());
        expect(h.more.settings()).toBe(h.config().settings);
        expect(h.more.wmap()).toBe(h.config().widgets);
        expect(h.more.isItemMenuOpen()).toBe(false);
    });

    it("stays open for the first click after a hover open, and a second click closes it", () => {
        render(() => <ActionWidgets />);
        hoverOpenMore();
        fireEvent.click(moreBtn()!);
        expect(moreUp()).not.toBeNull();
        fireEvent.click(moreBtn()!);
        expect(moreUp()).toBeNull();
    });

    it("closes on a second click after a click opened it", () => {
        render(() => <ActionWidgets />);
        fireEvent.click(moreBtn()!);
        fireEvent.click(moreBtn()!);
        expect(moreUp()).toBeNull();
    });

    it("forgets a click's confirmation once it closes", () => {
        render(() => <ActionWidgets />);
        fireEvent.click(moreBtn()!);
        fireEvent.click(moreBtn()!);
        hoverOpenMore();
        expect(moreUp()).not.toBeNull();
        fireEvent.click(moreBtn()!);
        expect(moreUp()).not.toBeNull();
    });

    it("closes 300 ms after the pointer leaves the button, and not if it comes back", () => {
        render(() => <ActionWidgets />);
        hoverOpenMore();
        mouseLeave(moreBtn()!);
        tick(299);
        expect(moreUp()).not.toBeNull();
        tick(1);
        expect(moreUp()).toBeNull();
        hoverOpenMore();
        mouseLeave(moreBtn()!);
        tick(100);
        mouseEnter(moreBtn()!);
        tick(1000);
        expect(moreUp()).not.toBeNull();
    });

    it("keeps open while the pointer is on the dropdown, and closes after it leaves", () => {
        render(() => <ActionWidgets />);
        hoverOpenMore();
        mouseLeave(moreBtn()!);
        h.more.onSubmenuEnter();
        tick(1000);
        expect(moreUp()).not.toBeNull();
        h.more.onSubmenuLeave({ clientX: 0, clientY: 0 });
        tick(300);
        expect(moreUp()).toBeNull();
    });

    it("hands the dropdown's element to the hover controller so it can track the pointer", () => {
        render(() => <ActionWidgets />);
        hoverOpenMore();
        const el = {
            getBoundingClientRect: vi.fn(() => ({
                left: 100,
                right: 200,
                top: 0,
                bottom: 100,
                width: 100,
                height: 100,
            })),
        };
        h.more.setSubmenuEl(el);
        mouseLeave(moreBtn()!, 50, 50);
        fireEvent.mouseMove(document, { clientX: 400, clientY: 400 });
        expect(el.getBoundingClientRect).toHaveBeenCalled();
        expect(moreUp()).toBeNull();
    });

    it("closes from its own close button", () => {
        render(() => <ActionWidgets />);
        fireEvent.click(moreBtn()!);
        h.more.onClose();
        expect(moreUp()).toBeNull();
    });

    it("closes on a press outside, not on one in the button, the dropdown or a popover menu", () => {
        render(() => <ActionWidgets />);
        fireEvent.click(moreBtn()!);
        fireEvent.mouseDown(moreBtn()!);
        fireEvent.mouseDown(moreUp()!);
        expect(moreUp()).not.toBeNull();
        const popover = document.createElement("div");
        popover.className = "popover-menu";
        const inner = document.createElement("span");
        popover.appendChild(inner);
        document.body.appendChild(popover);
        fireEvent.mouseDown(inner);
        expect(moreUp()).not.toBeNull();
        fireEvent.mouseDown(document.body);
        expect(moreUp()).toBeNull();
        popover.remove();
    });

    it("treats a text node's press by its parent element", () => {
        render(() => <ActionWidgets />);
        fireEvent.click(moreBtn()!);
        const popover = document.createElement("div");
        popover.className = "popover-menu";
        popover.textContent = "label";
        document.body.appendChild(popover);
        fireEvent.mouseDown(popover.firstChild!);
        expect(moreUp()).not.toBeNull();
        popover.remove();
    });

    it("sees a press even when its target stops it from bubbling", () => {
        render(() => <ActionWidgets />);
        fireEvent.click(moreBtn()!);
        const blocker = document.createElement("div");
        blocker.addEventListener("mousedown", (e) => e.stopPropagation());
        document.body.appendChild(blocker);
        fireEvent.mouseDown(blocker);
        expect(moreUp()).toBeNull();
        blocker.remove();
    });

    it("stops tracking the pointer when the bar unmounts mid-leave", () => {
        const { unmount } = render(() => <ActionWidgets />);
        hoverOpenMore();
        h.more.setSubmenuEl({
            getBoundingClientRect: () => ({ left: 100, right: 200, top: 0, bottom: 100, width: 100, height: 100 }),
        });
        mouseLeave(moreBtn()!, 50, 50);
        const remove = vi.spyOn(document, "removeEventListener");
        unmount();
        expect(remove.mock.calls.map(([type]) => type)).toContain("mousemove");
        remove.mockRestore();
    });

    it("stops listening for outside presses once closed", () => {
        render(() => <ActionWidgets />);
        const remove = vi.spyOn(document, "removeEventListener");
        fireEvent.click(moreBtn()!);
        fireEvent.click(moreBtn()!);
        expect(remove.mock.calls.some(([type]) => type === "mousedown")).toBe(true);
        remove.mockRestore();
    });

    it("closes an open parent flyout when More is hovered or clicked open, and vice versa", () => {
        render(() => <ActionWidgets />);
        hoverOpenParent();
        expect(flyoutUp()).not.toBeNull();
        mouseEnter(moreBtn()!);
        expect(flyoutUp()).toBeNull();
        tick(90);
        expect(moreUp()).not.toBeNull();
        mouseEnter(triggerOf("Messengers"));
        expect(moreUp()).toBeNull();
        tick(90);
        expect(flyoutUp()).not.toBeNull();
        fireEvent.click(moreBtn()!);
        expect(flyoutUp()).toBeNull();
        expect(moreUp()).not.toBeNull();
    });

    it("closes the More dropdown when it unmounts the bar", () => {
        const { unmount } = render(() => <ActionWidgets />);
        hoverOpenMore();
        unmount();
        expect(moreUp()).toBeNull();
        tick(1000);
    });
});

describe("a pinned group's flyout", () => {
    it("opens after the hover delay with the group's definition, and closes the More dropdown", () => {
        render(() => <ActionWidgets />);
        mouseEnter(triggerOf("Messengers"));
        tick(89);
        expect(flyoutUp()).toBeNull();
        tick(1);
        expect(flyoutUp()!.textContent).toBe("Messengers");
        expect(h.flyout.widget).toBe(WIDGETS["defwidget@msgr"]);
        expect(h.flyout.wmap()).toBe(h.config().widgets);
        expect(h.flyout.settings()).toBe(h.config().settings);
    });

    it("never opens for a leaf widget", () => {
        render(() => <ActionWidgets />);
        mouseEnter(triggerOf("Terminal"));
        tick(1000);
        expect(flyoutUp()).toBeNull();
    });

    it("is anchored to the group's slot", () => {
        render(() => <ActionWidgets />);
        hoverOpenParent();
        expect(h.flyout.anchor()).toBe(slotOf("Messengers"));
    });

    it("opens at once on a click, and a second click closes it", () => {
        render(() => <ActionWidgets />);
        fireEvent.click(tipOf("Messengers"));
        expect(flyoutUp()).not.toBeNull();
        expect(h.openWidget).not.toHaveBeenCalled();
        fireEvent.click(tipOf("Messengers"));
        expect(flyoutUp()).toBeNull();
    });

    it("stays open for the first click after a hover open, and a second click closes it", () => {
        render(() => <ActionWidgets />);
        hoverOpenParent();
        fireEvent.click(tipOf("Messengers"));
        expect(flyoutUp()).not.toBeNull();
        fireEvent.click(tipOf("Messengers"));
        expect(flyoutUp()).toBeNull();
    });

    it("forgets a click's confirmation once it closes", () => {
        render(() => <ActionWidgets />);
        fireEvent.click(tipOf("Messengers"));
        fireEvent.click(tipOf("Messengers"));
        hoverOpenParent();
        fireEvent.click(tipOf("Messengers"));
        expect(flyoutUp()).not.toBeNull();
    });

    it("closes 300 ms after the pointer leaves, which a click-open does too", () => {
        render(() => <ActionWidgets />);
        fireEvent.click(tipOf("Messengers"));
        mouseLeave(triggerOf("Messengers"));
        tick(299);
        expect(flyoutUp()).not.toBeNull();
        tick(1);
        expect(flyoutUp()).toBeNull();
    });

    it("stays open while the pointer is on the flyout", () => {
        render(() => <ActionWidgets />);
        hoverOpenParent();
        mouseLeave(triggerOf("Messengers"));
        h.flyout.onSubmenuEnter();
        tick(1000);
        expect(flyoutUp()).not.toBeNull();
        h.flyout.onSubmenuLeave({ clientX: 0, clientY: 0 });
        tick(300);
        expect(flyoutUp()).toBeNull();
    });

    it("tracks the flyout's element for the safe triangle", () => {
        render(() => <ActionWidgets />);
        hoverOpenParent();
        const el = {
            getBoundingClientRect: vi.fn(() => ({
                left: 100,
                right: 200,
                top: 0,
                bottom: 100,
                width: 100,
                height: 100,
            })),
        };
        h.flyout.setSubmenuEl(el);
        mouseLeave(triggerOf("Messengers"), 50, 50);
        fireEvent.mouseMove(document, { clientX: 400, clientY: 400 });
        expect(el.getBoundingClientRect).toHaveBeenCalled();
        expect(flyoutUp()).toBeNull();
    });

    it("closes from its own close button", () => {
        render(() => <ActionWidgets />);
        hoverOpenParent();
        h.flyout.onClose();
        expect(flyoutUp()).toBeNull();
    });

    it("closes on a press outside, not on one in its slot, the flyout or a popover menu", () => {
        render(() => <ActionWidgets />);
        hoverOpenParent();
        fireEvent.mouseDown(slotOf("Messengers"));
        fireEvent.mouseDown(flyoutUp()!);
        expect(flyoutUp()).not.toBeNull();
        const popover = document.createElement("div");
        popover.className = "popover-menu";
        document.body.appendChild(popover);
        fireEvent.mouseDown(popover);
        expect(flyoutUp()).not.toBeNull();
        fireEvent.mouseDown(slotOf("Terminal"));
        expect(flyoutUp()).toBeNull();
        popover.remove();
    });

    it("treats a text node's press by its parent element", () => {
        render(() => <ActionWidgets />);
        hoverOpenParent();
        const popover = document.createElement("div");
        popover.className = "popover-menu";
        popover.textContent = "label";
        document.body.appendChild(popover);
        fireEvent.mouseDown(popover.firstChild!);
        expect(flyoutUp()).not.toBeNull();
        popover.remove();
    });

    it("sees a press even when its target stops it from bubbling", () => {
        render(() => <ActionWidgets />);
        hoverOpenParent();
        const blocker = document.createElement("div");
        blocker.addEventListener("mousedown", (e) => e.stopPropagation());
        document.body.appendChild(blocker);
        fireEvent.mouseDown(blocker);
        expect(flyoutUp()).toBeNull();
        blocker.remove();
    });

    it("closes on Escape and on no other key", () => {
        render(() => <ActionWidgets />);
        hoverOpenParent();
        fireEvent.keyDown(document, { key: "Enter" });
        expect(flyoutUp()).not.toBeNull();
        fireEvent.keyDown(document, { key: "Escape" });
        expect(flyoutUp()).toBeNull();
    });

    it("stops listening for presses and keys once closed", () => {
        render(() => <ActionWidgets />);
        const remove = vi.spyOn(document, "removeEventListener");
        hoverOpenParent();
        fireEvent.keyDown(document, { key: "Escape" });
        const types = remove.mock.calls.map(([type]) => type);
        expect(types).toContain("mousedown");
        expect(types).toContain("keydown");
        remove.mockRestore();
    });

    it("closes when the group is unpinned or hidden while open", () => {
        render(() => <ActionWidgets />);
        hoverOpenParent();
        setBar({ "widget:pinned": ["agent", "term", "settings"] });
        expect(flyoutUp()).toBeNull();
        setBar();
        hoverOpenParent();
        expect(flyoutUp()).not.toBeNull();
        setBar({ "widget:hidden": ["defwidget@msgr"] });
        expect(flyoutUp()).toBeNull();
    });

    it("stays open when the config changes underneath it", () => {
        render(() => <ActionWidgets />);
        hoverOpenParent();
        setBar({ ...SETTINGS, "widget:pinned": ["agent", "term", "msgr", "settings", "discord"] });
        expect(flyoutUp()).not.toBeNull();
        setBar({ ...SETTINGS, "widget:pinned": ["agent", "term", "msgr", "settings"] });
        expect(flyoutUp()).not.toBeNull();
    });

    it("ignores a hover leave on the flyout while the item menu is open", () => {
        render(() => <ActionWidgets />);
        hoverOpenParent();
        h.flyout.onItemContextMenu({ x: 1, y: 2 }, "defwidget@discord");
        h.flyout.onSubmenuLeave({ clientX: 0, clientY: 0 });
        tick(1000);
        expect(flyoutUp()).not.toBeNull();
    });

    it("has no leftovers once the bar unmounts", () => {
        const { unmount } = render(() => <ActionWidgets />);
        hoverOpenParent();
        unmount();
        expect(flyoutUp()).toBeNull();
        tick(1000);
    });

    it("switches from one group to another without both open", () => {
        setBar(SETTINGS, {
            ...WIDGETS,
            "defwidget@docs": leaf("Docs", 7, { "display:pinned": true, children: ["notes"] }),
        });
        render(() => <ActionWidgets />);
        hoverOpenParent("Messengers");
        mouseEnter(triggerOf("Docs"));
        expect(flyoutUp()).toBeNull();
        tick(90);
        expect(flyoutUp()!.textContent).toBe("Docs");
        expect(screen.getAllByTestId("flyout")).toHaveLength(1);
    });

    it("lets a click on a second group close the first, and a click on More close both", () => {
        setBar(SETTINGS, {
            ...WIDGETS,
            "defwidget@extra": leaf("Extra", 8),
            "defwidget@docs": leaf("Docs", 7, { "display:pinned": true, children: ["notes"] }),
        });
        render(() => <ActionWidgets />);
        fireEvent.click(moreBtn()!);
        fireEvent.click(tipOf("Messengers"));
        expect(moreUp()).toBeNull();
        expect(flyoutUp()!.textContent).toBe("Messengers");
        fireEvent.click(tipOf("Docs"));
        expect(screen.getAllByTestId("flyout")).toHaveLength(1);
        expect(flyoutUp()!.textContent).toBe("Docs");
        tick(1000);
        expect(flyoutUp()!.textContent).toBe("Docs");
    });

    it("keeps a second group open when the first one's close fires late", () => {
        setBar(SETTINGS, {
            ...WIDGETS,
            "defwidget@docs": leaf("Docs", 7, { "display:pinned": true, children: ["notes"] }),
        });
        render(() => <ActionWidgets />);
        hoverOpenParent("Messengers");
        const first = h.flyout;
        hoverOpenParent("Docs");
        first.onClose();
        expect(flyoutUp()!.textContent).toBe("Docs");
    });

    it("keeps hover-close working for an open group after the config changes underneath it", () => {
        render(() => <ActionWidgets />);
        hoverOpenParent();
        setBar({ ...SETTINGS });
        expect(flyoutUp()).not.toBeNull();
        mouseLeave(triggerOf("Messengers"));
        tick(300);
        expect(flyoutUp()).toBeNull();
    });

    it("stops tracking the pointer when the bar unmounts mid-leave", () => {
        const { unmount } = render(() => <ActionWidgets />);
        hoverOpenParent();
        h.flyout.setSubmenuEl({
            getBoundingClientRect: () => ({ left: 100, right: 200, top: 0, bottom: 100, width: 100, height: 100 }),
        });
        mouseLeave(triggerOf("Messengers"), 50, 50);
        const remove = vi.spyOn(document, "removeEventListener");
        unmount();
        expect(remove.mock.calls.map(([type]) => type)).toContain("mousemove");
        remove.mockRestore();
    });

    it("tells a stale flyout callback nothing after the flyout has gone", () => {
        render(() => <ActionWidgets />);
        hoverOpenParent();
        const flyout = h.flyout;
        h.flyout.onClose();
        expect(() => {
            flyout.onSubmenuEnter();
            flyout.onSubmenuLeave({ clientX: 0, clientY: 0 });
            flyout.setSubmenuEl(null);
            flyout.onClose();
        }).not.toThrow();
    });
});

describe("the item menu", () => {
    const openItemMenu = (key: string, pos = { x: 12, y: 34 }) => {
        h.more.onItemContextMenu(pos, key);
    };
    const labels = () => [...screen.getByTestId("popover").querySelectorAll("button")].map((b) => b.textContent);

    it("opens at the click position and closes through its own close", () => {
        render(() => <ActionWidgets />);
        fireEvent.click(moreBtn()!);
        expect(screen.queryByTestId("popover")).toBeNull();
        openItemMenu("defwidget@notes");
        expect(screen.getByTestId("popover").getAttribute("data-x")).toBe("12");
        expect(screen.getByTestId("popover").getAttribute("data-y")).toBe("34");
        h.popover.onClose();
        expect(screen.queryByTestId("popover")).toBeNull();
    });

    it("offers a leaf the open actions, then pin, then hide", () => {
        render(() => <ActionWidgets />);
        fireEvent.click(moreBtn()!);
        openItemMenu("defwidget@notes");
        expect(labels()).toEqual(["Open in New Window", "Open in New Tab", "Pin to bar", "Hide from widget bar"]);
        expect(screen.getByTestId("popover").querySelectorAll("hr")).toHaveLength(1);
        expect(h.openActions).toHaveBeenLastCalledWith("notes", h.config().widgets);
        expect(h.popover.items[0]).toMatchObject({ label: "Open in New Window", icon: "win" });
    });

    it("offers Unpin for a leaf that is pinned", () => {
        render(() => <ActionWidgets />);
        fireEvent.click(moreBtn()!);
        openItemMenu("defwidget@term");
        expect(labels()).toEqual(["Open in New Window", "Open in New Tab", "Unpin from bar", "Hide from widget bar"]);
    });

    it("closes the dropdown before running an open action", () => {
        render(() => <ActionWidgets />);
        fireEvent.click(moreBtn()!);
        h.runAction.mockImplementation(() => expect(moreUp()).toBeNull());
        openItemMenu("defwidget@notes");
        fireEvent.click(screen.getByText("Open in New Tab"));
        expect(h.runAction).toHaveBeenCalledTimes(1);
        expect(moreUp()).toBeNull();
    });

    it("pins, unpins and hides by the item's short name", () => {
        render(() => <ActionWidgets />);
        fireEvent.click(moreBtn()!);
        openItemMenu("defwidget@notes");
        fireEvent.click(screen.getByText("Pin to bar"));
        expect(h.pinWidget).toHaveBeenCalledWith("notes", h.config().settings, h.config().widgets);
        fireEvent.click(screen.getByText("Hide from widget bar"));
        expect(h.hideFromWidgetBar).toHaveBeenCalledWith("defwidget@notes", h.config().settings, h.config().widgets);
        openItemMenu("defwidget@term");
        fireEvent.click(screen.getByText("Unpin from bar"));
        expect(h.unpinWidget).toHaveBeenCalledWith("term", h.config().settings, h.config().widgets);
    });

    it("offers a group only the group-wide actions", () => {
        render(() => <ActionWidgets />);
        fireEvent.click(moreBtn()!);
        openItemMenu("defwidget@msgr");
        expect(labels()).toEqual(["Unpin group from bar", "Hide group from widget bar"]);
        expect(h.openActions).not.toHaveBeenCalled();
        fireEvent.click(screen.getByText("Unpin group from bar"));
        expect(h.unpinWidget).toHaveBeenCalledWith("msgr", h.config().settings, h.config().widgets);
        fireEvent.click(screen.getByText("Hide group from widget bar"));
        expect(h.hideFromWidgetBar).toHaveBeenCalledWith("defwidget@msgr", h.config().settings, h.config().widgets);
    });

    it("offers an unpinned group Pin", () => {
        setBar({ "widget:pinned": ["agent"] });
        render(() => <ActionWidgets />);
        fireEvent.click(moreBtn()!);
        openItemMenu("defwidget@msgr");
        expect(labels()).toEqual(["Pin group to bar", "Hide group from widget bar"]);
        fireEvent.click(screen.getByText("Pin group to bar"));
        expect(h.pinWidget).toHaveBeenCalledWith("msgr", h.config().settings, h.config().widgets);
    });

    it("cannot hide Settings", () => {
        render(() => <ActionWidgets />);
        fireEvent.click(moreBtn()!);
        openItemMenu("defwidget@settings");
        expect(labels()).toEqual(["Open in New Window", "Open in New Tab", "Unpin from bar"]);
    });

    it("tells the dropdown and flyout that an item menu is open until it closes", () => {
        render(() => <ActionWidgets />);
        fireEvent.click(moreBtn()!);
        openItemMenu("defwidget@notes");
        expect(h.more.isItemMenuOpen()).toBe(true);
        h.popover.onClose();
        expect(h.more.isItemMenuOpen()).toBe(false);
    });

    it("keeps the dropdown open when the pointer leaves it under the open menu", () => {
        render(() => <ActionWidgets />);
        hoverOpenMore();
        openItemMenu("defwidget@notes");
        h.more.onSubmenuLeave({ clientX: 0, clientY: 0 });
        tick(1000);
        expect(moreUp()).not.toBeNull();
        h.popover.onClose();
        h.more.onSubmenuLeave({ clientX: 0, clientY: 0 });
        tick(300);
        expect(moreUp()).toBeNull();
    });
});

describe("a pinned widget's right-click menu", () => {
    const rightClick = (label: string) => {
        const ev = new MouseEvent("contextmenu", { bubbles: true, cancelable: true });
        const stop = vi.spyOn(ev, "stopPropagation");
        triggerOf(label).dispatchEvent(ev);
        return { ev, stop };
    };

    it("claims the event, cancels a drag in progress, and marks the slot", () => {
        render(() => <ActionWidgets />);
        const { ev, stop } = rightClick("Terminal");
        expect(ev.defaultPrevented).toBe(true);
        expect(stop).toHaveBeenCalled();
        expect(h.pointer.cancel).toHaveBeenCalledTimes(1);
        expect(slotOf("Terminal").classList.contains("context-active")).toBe(true);
        expect(slotOf("Agent").classList.contains("context-active")).toBe(false);
    });

    it("opens the open actions, a separator, Unpin and Hide for a leaf, at the event", () => {
        render(() => <ActionWidgets />);
        const { ev } = rightClick("Terminal");
        expect(h.openMenuAt).toHaveBeenCalledTimes(1);
        const [items, at] = h.openMenuAt.mock.calls[0] as [MenuItem[], MouseEvent];
        expect(at).toBe(ev);
        expect(items.map((i) => i.label ?? i.type)).toEqual([
            "Open in New Window",
            "Open in New Tab",
            "separator",
            "Unpin from bar",
            "Hide from widget bar",
        ]);
        expect(items[0]).toMatchObject({ icon: "win", click: h.runAction });
        expect(h.openActions).toHaveBeenLastCalledWith("term", h.config().widgets);
        items[3].click();
        expect(h.unpinWidget).toHaveBeenCalledWith("term", h.config().settings, h.config().widgets);
        items[4].click();
        expect(h.hideFromWidgetBar).toHaveBeenCalledWith("defwidget@term", h.config().settings, h.config().widgets);
    });

    it("offers Settings no Hide", () => {
        render(() => <ActionWidgets />);
        rightClick("Settings");
        const items = h.openMenuAt.mock.calls[0][0] as MenuItem[];
        expect(items.map((i) => i.label ?? i.type)).toEqual([
            "Open in New Window",
            "Open in New Tab",
            "separator",
            "Unpin from bar",
        ]);
    });

    it("offers a group only the group actions, and closes its flyout", () => {
        render(() => <ActionWidgets />);
        hoverOpenParent();
        const { ev } = rightClick("Messengers");
        expect(flyoutUp()).toBeNull();
        const [items, at] = h.openMenuAt.mock.calls[0] as [MenuItem[], MouseEvent];
        expect(at).toBe(ev);
        expect(items.map((i) => i.label)).toEqual(["Unpin group from bar", "Hide group from widget bar"]);
        expect(h.openActions).not.toHaveBeenCalled();
        items[0].click();
        expect(h.unpinWidget).toHaveBeenCalledWith("msgr", h.config().settings, h.config().widgets);
        items[1].click();
        expect(h.hideFromWidgetBar).toHaveBeenCalledWith("defwidget@msgr", h.config().settings, h.config().widgets);
    });

    it("clears the mark on the next press, once", () => {
        render(() => <ActionWidgets />);
        rightClick("Terminal");
        fireEvent.mouseDown(document.body);
        expect(slotOf("Terminal").classList.contains("context-active")).toBe(false);
        rightClick("Agent");
        expect(slotOf("Agent").classList.contains("context-active")).toBe(true);
        fireEvent.mouseDown(document.body);
        expect(slotOf("Agent").classList.contains("context-active")).toBe(false);
    });

    it("clears the mark on Escape and on no other key", () => {
        render(() => <ActionWidgets />);
        rightClick("Terminal");
        fireEvent.keyDown(document, { key: "a" });
        expect(slotOf("Terminal").classList.contains("context-active")).toBe(true);
        fireEvent.keyDown(document, { key: "Escape" });
        expect(slotOf("Terminal").classList.contains("context-active")).toBe(false);
    });

    it("moves the mark to a second widget and drops the first one's listeners", () => {
        render(() => <ActionWidgets />);
        const remove = vi.spyOn(document, "removeEventListener");
        rightClick("Terminal");
        rightClick("Agent");
        expect(slotOf("Terminal").classList.contains("context-active")).toBe(false);
        expect(slotOf("Agent").classList.contains("context-active")).toBe(true);
        const removed = remove.mock.calls.map(([type]) => type);
        expect(removed).toContain("mousedown");
        expect(removed).toContain("keydown");
        remove.mockRestore();
        fireEvent.keyDown(document, { key: "Escape" });
        expect(slotOf("Agent").classList.contains("context-active")).toBe(false);
    });
});
