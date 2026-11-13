// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The paths of a pane's frame that blockframe.default.test.tsx leaves out: when the header is drawn,
 * the colours and the settings the frame reads, what a drag preview leaves out, the connection bar,
 * the switch-connection modal and the splitter badge. The real frame is mounted over stand-in node
 * and view models; the pieces it hands props to are probes that record them.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    modalProps: [] as {
        blockId: string;
        nodeModel: unknown;
        viewModel: unknown;
        blockRef: unknown;
        connBtnRef: unknown;
        changeConnModalOpen: () => boolean;
        setChangeConnModalOpen: (v: boolean) => void;
    }[],
    badgeProps: [] as { target: () => Element | undefined; blockId: string }[],
    conn: {} as Record<string, unknown>,
    theme: "dark" as string,
}));

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: new Proxy({} as Record<string, unknown>, { get: () => async () => ({}) }),
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/util/app-api", () => ({
    getApi: () =>
        new Proxy({ listen: async () => () => {} } as Record<string, unknown>, {
            get: (t, k: string) => (k in t ? t[k] : () => false),
        }),
}));
vi.mock("@/app/store/conn-status", async (orig) => ({
    ...(await orig<Record<string, unknown>>()),
    getConnStatusAtom: (conn: string) => () => h.conn[conn] ?? null,
}));
vi.mock("@/app/store/window-appearance", async (orig) => ({
    ...(await orig<Record<string, unknown>>()),
    currentWindowTheme: () => h.theme,
}));
vi.mock("@/app/modals/conntypeahead", () => ({
    ChangeConnectionBlockModal: (p: (typeof h.modalProps)[number]) => {
        h.modalProps.push(p);
        return <div data-testid="conn-modal" />;
    },
}));
vi.mock("@/element/blockstats", async (orig) => ({
    ...(await orig<Record<string, unknown>>()),
    BlockStatsBadge: (p: { noHeader: boolean; connection?: string }) => (
        <div data-testid="stats" data-noheader={String(p.noHeader)} data-connection={p.connection ?? ""} />
    ),
}));
vi.mock("./titlebar", () => ({
    TitleBar: (p: { title: string }) => <div data-testid="titlebar" data-title={p.title} />,
}));
vi.mock("./pane-size-badge", () => ({
    PaneSizeBadge: (p: (typeof h.badgeProps)[number]) => {
        h.badgeProps.push(p);
        return <div data-testid="size-badge" />;
    },
}));

import { registerBlockComponentModel, unregisterBlockComponentModel } from "@/app/store/block-component-registry";
import { setFullConfigAtom } from "@/app/store/config-signals";
import { MOS, useBlockAtom } from "@/app/store/global";
import { registerPaneTab } from "@/app/store/pane-tab-registry";
import type { MuxObjUpdate } from "@/types/rpc/MuxObjUpdate";
import * as util from "@/util/util";
import { BlockFrame, computeBlockActiveBorderColor, computeBlockIdentityColor } from "./blockframe";

registerPaneTab({
    apiVersion: 1,
    view: "term",
    label: "Terminal",
    icon: "t",
    create: () => ({ component: () => null }) as never,
    capabilities: { hueBorder: true, statsBadgeSetting: "term:showstatsbadge" },
});

let version = 0;
let nextId = 0;
function seedBlock(blockId: string, meta: Record<string, unknown>): void {
    MOS.updateMuxObject({
        updatetype: "update",
        otype: "block",
        oid: blockId,
        obj: { otype: "block", oid: blockId, version: ++version, meta } as unknown as Block,
    } as MuxObjUpdate);
}

type Over = {
    meta?: Record<string, unknown>;
    preview?: boolean;
    focused?: boolean;
    hoisted?: boolean;
    noHeader?: boolean | "absent";
    manageConnection?: boolean;
    dragging?: boolean;
    noViewModel?: boolean;
    viewIcon?: string;
    blockBg?: Record<string, unknown>;
    blockId?: string;
    /** The view's own end buttons (an agent pane's search and Messages). */
    endButtons?: number;
    /** Replaces parts of the node model (the minimize pair's merge target and spies). */
    node?: Record<string, unknown>;
};

function setup(over: Over = {}) {
    const blockId = over.blockId ?? `bp${++nextId}`;
    seedBlock(blockId, over.meta ?? { view: "term" });
    const [focused] = createSignal(over.focused ?? true);
    const nodeModel = {
        blockId,
        isFocused: focused,
        numLeafs: () => 2,
        isMagnified: () => false,
        isEphemeral: () => false,
        blockNum: () => 3,
        toggleMagnify: () => {},
        onClose: () => {},
        isSplitterDragging: () => over.dragging ?? false,
        paneChromeHoisted: over.hoisted === undefined ? undefined : () => over.hoisted,
        isMinimized: () => false,
        canMinimize: () => true,
        isClosing: () => false,
        closingTabs: () => new Set<string>(),
        toggleMinimize: () => {},
        addEphemeralNodeToLayout: () => {},
        dragHandleRef: { current: null },
        ...(over.node ?? {}),
    } as never;
    const viewModel = over.noViewModel
        ? null
        : ({
              ...(over.noHeader === "absent" ? {} : { noHeader: () => over.noHeader ?? true }),
              manageConnection: over.manageConnection ? util.createSignalAtom(true) : undefined,
              viewIcon: over.viewIcon ? util.createSignalAtom(over.viewIcon) : undefined,
              blockBg: over.blockBg ? util.createSignalAtom(over.blockBg) : undefined,
              endIconButtons: over.endButtons
                  ? util.createSignalAtom(
                        Array.from({ length: over.endButtons }, (_, i) => ({ elemtype: "iconbutton", icon: "search", title: `Aux ${i}`, click: () => {} }))
                    )
                  : undefined,
              getBodyContextMenuItems: () => [],
          } as never);
    const blockRef = { current: null as HTMLDivElement | null };
    const view = render(() => (
        <BlockFrame
            nodeModel={nodeModel}
            viewModel={viewModel}
            preview={over.preview ?? false}
            blockModel={{ onClick: () => {}, onFocusCapture: () => {}, blockRef } as never}
        >
            <div class="body-child">body</div>
        </BlockFrame>
    ));
    return { blockId, view, nodeModel, viewModel, blockRef };
}

const frame = (c: HTMLElement) => c.querySelector(".block-frame-default") as HTMLElement;
const q = (c: HTMLElement, sel: string) => c.querySelector(sel);
const flush = () => new Promise((r) => setTimeout(r, 0));
const style = (c: HTMLElement) => frame(c).style;

beforeEach(() => {
    (globalThis as { ResizeObserver?: unknown }).ResizeObserver ??= class {
        observe() {}
        unobserve() {}
        disconnect() {}
    };
    h.modalProps.length = 0;
    h.badgeProps.length = 0;
    h.conn = {};
    h.theme = "dark";
});
afterEach(() => {
    cleanup();
    setFullConfigAtom(null as never);
});

describe("the header", () => {
    it("is drawn when neither the host nor the view hides it, with the block's own title", () => {
        const t = setup({ noHeader: false, meta: { view: "term", "frame:title": "Build box" } });
        const header = q(t.view.container, ".block-frame-default-header");
        expect(header).not.toBeNull();
        expect(header!.textContent).toContain("Build box");
    });

    it("puts the pane's content in one scroll range and the window controls outside it", () => {
        const t = setup({ noHeader: false, meta: { view: "term", "frame:title": "Build box" } });
        const header = q(t.view.container, ".block-frame-default-header") as HTMLElement;
        const range = q(header, ".block-frame-header-scroll") as HTMLElement;
        expect(range).not.toBeNull();
        expect(q(range, ".block-frame-header-content")).not.toBeNull();
        // The view's end buttons scroll with the tabs (#29325); the window controls stay pinned after the range,
        // and the grip is pinned with them, just left of minimize.
        expect(q(range, ".pane-grip")).toBeNull();
        const pinned = q(header, ".block-frame-end-icons") as HTMLElement;
        expect(pinned).not.toBeNull();
        expect(range.contains(pinned)).toBe(false);
        expect(q(pinned, ".block-frame-minimize")).not.toBeNull();
        expect(pinned.firstElementChild?.classList.contains("pane-grip")).toBe(true);
        expect(pinned.children[1]?.classList.contains("block-frame-minimize-pair")).toBe(true);
    });

    it("keeps the grip right of the view's own buttons and their separator, just left of minimize", () => {
        const t = setup({ noHeader: false, meta: { view: "term" }, endButtons: 2 });
        const header = q(t.view.container, ".block-frame-default-header") as HTMLElement;
        const grip = q(header, ".pane-grip") as HTMLElement;
        const separator = q(header, ".block-frame-header-aux .block-frame-btn-separator") as HTMLElement;
        const minimize = q(header, ".block-frame-minimize-pair") as HTMLElement;
        expect(separator).not.toBeNull();
        const follows = (a: Element, b: Element) => !!(a.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING);
        expect(follows(separator, grip)).toBe(true);
        expect(follows(grip, minimize)).toBe(true);
        expect(grip.nextElementSibling).toBe(minimize);
    });

    it("draws no grip in a drag preview", () => {
        const t = setup({ noHeader: false, preview: true });
        expect(q(t.view.container, ".pane-grip")).toBeNull();
    });

    it("is drawn when the view says nothing about it", () => {
        const t = setup({ noHeader: "absent" });
        expect(q(t.view.container, ".block-frame-default-header")).not.toBeNull();
    });

    it("is left to the host's pane chrome when that is hoisted, whatever the view says", () => {
        const t = setup({ noHeader: false, hoisted: true });
        expect(q(t.view.container, ".block-frame-default-header")).toBeNull();
    });

    it("stays when the host's chrome is not hoisted", () => {
        const t = setup({ noHeader: false, hoisted: false });
        expect(q(t.view.container, ".block-frame-default-header")).not.toBeNull();
    });

    it("is hidden when the view hides it and the host does not", () => {
        const t = setup({ noHeader: true, hoisted: false });
        expect(q(t.view.container, ".block-frame-default-header")).toBeNull();
    });

    it("tells the stats badge whether the header is there", () => {
        const withHeader = setup({ noHeader: false });
        expect(q(withHeader.view.container, '[data-testid="stats"]')!.getAttribute("data-noheader")).toBe("false");
    });
});

describe("the stats badge", () => {
    it("follows the terminal setting for a terminal, and is dropped when it is off", () => {
        setFullConfigAtom({ settings: { "term:showstatsbadge": false } } as never);
        const off = setup();
        expect(q(off.view.container, '[data-testid="stats"]')).toBeNull();
        cleanup();
        setFullConfigAtom({ settings: { "term:showstatsbadge": true } } as never);
        expect(q(setup().view.container, '[data-testid="stats"]')).not.toBeNull();
    });

    it("stays for a view with no such setting, whatever the terminal's is", () => {
        setFullConfigAtom({ settings: { "term:showstatsbadge": false } } as never);
        const t = setup({ meta: { view: "web" } });
        expect(q(t.view.container, '[data-testid="stats"]')).not.toBeNull();
    });

    it("measures a remote shell only for a terminal", () => {
        const term = setup({ meta: { view: "term", connection: "me@host" } });
        expect(q(term.view.container, '[data-testid="stats"]')!.getAttribute("data-connection")).toBe("me@host");
        cleanup();
        const other = setup({ meta: { view: "web", connection: "me@host" } });
        expect(q(other.view.container, '[data-testid="stats"]')!.getAttribute("data-connection")).toBe("");
    });
});

describe("the colours", () => {
    const hue = { view: "term", "frame:hue": 210 };

    it("a drag preview carries no agent colour and no identity colour", () => {
        const t = setup({ preview: true, meta: hue });
        const el = frame(t.view.container);
        expect(el.classList.contains("has-agent-color")).toBe(false);
        expect(style(t.view.container).getPropertyValue("--block-agent-color")).toBe("transparent");
        expect(style(t.view.container).getPropertyValue("--block-identity-color")).toBe("");
    });

    it("a view without the hue border takes none, however the block is coloured", () => {
        const t = setup({ meta: { view: "web", "frame:hue": 210, "frame:activebordercolor": "#ff8800" } });
        expect(frame(t.view.container).classList.contains("has-agent-color")).toBe(false);
        expect(style(t.view.container).getPropertyValue("--block-agent-color")).toBe("transparent");
    });

    it("a colour that would not show as a ring is not taken", () => {
        const t = setup({ meta: { view: "term", "frame:activebordercolor": "transparent" } });
        expect(frame(t.view.container).classList.contains("has-agent-color")).toBe(false);
        expect(style(t.view.container).getPropertyValue("--block-agent-color")).toBe("transparent");
    });

    it("the border colour and the identity colour follow the theme", () => {
        const meta = hue as never;
        const dark = computeBlockActiveBorderColor(meta, false);
        const light = computeBlockActiveBorderColor(meta, true);
        expect(light).not.toBe(dark);
        const darkPane = setup({ meta: hue });
        expect(style(darkPane.view.container).getPropertyValue("--block-agent-color")).toBe(dark);
        expect(style(darkPane.view.container).getPropertyValue("--block-identity-color")).toBe(
            computeBlockIdentityColor(meta, false)
        );
        cleanup();
        h.theme = "light";
        const lightPane = setup({ meta: hue });
        expect(style(lightPane.view.container).getPropertyValue("--block-agent-color")).toBe(light);
        expect(computeBlockIdentityColor(meta, true)).not.toBe(computeBlockIdentityColor(meta, false));
        expect(style(lightPane.view.container).getPropertyValue("--block-identity-color")).toBe(
            computeBlockIdentityColor(meta, true)
        );
    });
});

describe("a drag preview", () => {
    it("is lit as focused whether or not the pane is, and is not interactive", () => {
        const t = setup({ preview: true, focused: false });
        const el = frame(t.view.container);
        expect(el.classList.contains("block-focused")).toBe(true);
        expect((el as HTMLElement & { inert?: boolean }).inert).toBe(true);
        cleanup();
        const live = setup({ focused: false });
        expect(frame(live.view.container).classList.contains("block-focused")).toBe(false);
        expect(!!(frame(live.view.container) as HTMLElement & { inert?: boolean }).inert).toBe(false);
    });

    it("shows the view's own icon", () => {
        const t = setup({ preview: true, viewIcon: "rocket" });
        expect(q(t.view.container, ".block-frame-preview .block-frame-view-icon i")!.className).toContain("rocket");
    });

    it("takes no background", () => {
        const bg = { bg: "#123456", "bg:opacity": 0.5 };
        const t = setup({ preview: true, blockBg: bg });
        expect(q(t.view.container, ".block-frame-default-inner")!.getAttribute("style")).toBeNull();
    });

    it("draws no connection bar, even over a lost connection", () => {
        h.conn["me@host"] = { connection: "me@host", status: "error", hasconnected: true, error: "x" };
        const t = setup({ preview: true, meta: { view: "term", connection: "me@host" } });
        expect(q(t.view.container, ".connstatus-overlay")).toBeNull();
    });

    it("draws no splitter badge, even during a drag", () => {
        const t = setup({ preview: true, dragging: true });
        expect(q(t.view.container, '[data-testid="size-badge"]')).toBeNull();
    });

    it("never shows the switch-connection modal, even with its flag set", async () => {
        const t = setup({ preview: true, manageConnection: true });
        const flag = useBlockAtom(t.blockId, "changeConn", () =>
            util.createSignalAtom(false)
        ) as util.SignalAtom<boolean>;
        flag._set(true);
        await flush();
        expect(q(t.view.container, '[data-testid="conn-modal"]')).toBeNull();
    });
});

describe("the body's background", () => {
    it("comes from the view's own background", () => {
        const t = setup({ blockBg: { bg: "#123456", "bg:opacity": 0.5 } });
        const inner = q(t.view.container, ".block-frame-default-inner") as HTMLElement;
        expect(inner.style.background).toContain("rgb(18, 52, 86)");
        expect(inner.style.opacity).toBe("0.5");
    });

    it("is none when the view has none", () => {
        const t = setup();
        expect(q(t.view.container, ".block-frame-default-inner")!.getAttribute("style")).toBeNull();
    });
});

describe("the title bar", () => {
    it("carries the pane's own title", () => {
        const t = setup({ meta: { view: "term", "pane-title": "Pinned title" } });
        expect(q(t.view.container, '[data-testid="titlebar"]')!.getAttribute("data-title")).toBe("Pinned title");
    });

    it("has no title of its own when the pane is not given one", () => {
        const t = setup({ meta: { view: "term", "display:name": "Build box" } });
        expect(q(t.view.container, '[data-testid="titlebar"]')!.getAttribute("data-title")).toBe("");
    });
});

describe("the connection bar", () => {
    const lost = { connection: "me@host", status: "disconnected", hasconnected: true, error: null };

    it("shows over a pane whose connection is gone", () => {
        h.conn["me@host"] = lost;
        const t = setup({ meta: { view: "term", connection: "me@host" } });
        expect(q(t.view.container, ".connstatus-overlay")!.textContent).toContain('Disconnected from "me@host"');
    });

    it("is not drawn for a pane the view has no model for", () => {
        h.conn["me@host"] = lost;
        const t = setup({ noViewModel: true, meta: { view: "term", connection: "me@host" } });
        expect(q(t.view.container, ".connstatus-overlay")).toBeNull();
    });

    it("is not drawn for a connection that is up", () => {
        h.conn["me@host"] = { ...lost, status: "connected" };
        const t = setup({ meta: { view: "term", connection: "me@host" } });
        expect(q(t.view.container, ".connstatus-overlay")).toBeNull();
    });
});

describe("the switch-connection modal", () => {
    function open(t: ReturnType<typeof setup>) {
        const flag = useBlockAtom(t.blockId, "changeConn", () =>
            util.createSignalAtom(false)
        ) as util.SignalAtom<boolean>;
        flag._set(true);
        return flag;
    }

    it("is given the frame's own parts, and closes through the flag", async () => {
        const t = setup({ manageConnection: true });
        expect(q(t.view.container, '[data-testid="conn-modal"]')).toBeNull();
        const flag = open(t);
        await flush();
        expect(q(t.view.container, '[data-testid="conn-modal"]')).not.toBeNull();
        const p = h.modalProps.at(-1);
        expect(p.blockId).toBe(t.blockId);
        expect(p.nodeModel).toBe(t.nodeModel);
        expect(p.viewModel).toBe(t.viewModel);
        expect(p.blockRef).toBe(t.blockRef);
        expect(p.changeConnModalOpen()).toBe(true);
        p.setChangeConnModalOpen(false);
        await flush();
        expect(flag()).toBe(false);
        expect(q(t.view.container, '[data-testid="conn-modal"]')).toBeNull();
    });

    it("anchors on the one holder the block keeps for its connection button", async () => {
        const t = setup({ manageConnection: true });
        open(t);
        await flush();
        const holder = h.modalProps.at(-1).connBtnRef;
        const kept = useBlockAtom(t.blockId, "connBtnRef", () => () => {
            throw new Error("the frame should have made it");
        })();
        expect(kept).toBe(holder);
    });

    it("is not drawn for a pane with no view model", async () => {
        const t = setup({ noViewModel: true });
        open(t);
        await flush();
        expect(q(t.view.container, '[data-testid="conn-modal"]')).toBeNull();
    });
});

describe("the splitter badge", () => {
    it("measures the frame, for its own block, while a splitter is dragged", () => {
        const t = setup({ dragging: true });
        expect(q(t.view.container, '[data-testid="size-badge"]')).not.toBeNull();
        const p = h.badgeProps.at(-1);
        expect(p.blockId).toBe(t.blockId);
        expect(p.target()).toBe(frame(t.view.container));
    });
});

describe("the block's component model", () => {
    it("is left as the view had it for a pane the frame does not manage", () => {
        const bcm: { openSwitchConnection: null | (() => void) } = { openSwitchConnection: null };
        registerBlockComponentModel("bp-fixed", bcm as never);
        setup({ blockId: "bp-fixed" });
        expect(bcm.openSwitchConnection).toBeNull();
        unregisterBlockComponentModel("bp-fixed");
    });
});

describe("Maximize on a lone pane", () => {
    const maximizeOf = (node: Record<string, unknown>) => {
        const t = setup({ noHeader: false, node });
        return q(t.view.container, ".block-frame-magnify");
    };

    it("is drawn when the tab has more than one pane", () => {
        expect(maximizeOf({ numLeafs: () => 2 })).not.toBeNull();
    });

    it("is left out when the tab has one pane", () => {
        expect(maximizeOf({ numLeafs: () => 1 })).toBeNull();
    });

    it("keeps Restore for a magnified pane, whatever the count", () => {
        expect(maximizeOf({ numLeafs: () => 1, isMagnified: () => true })).not.toBeNull();
    });
});

describe("the two stacked minimize buttons", () => {
    const header = (over: Over = {}) => {
        const t = setup({ noHeader: false, ...over });
        return q(t.view.container, ".block-frame-default-header") as HTMLElement;
    };
    const pairOf = (hd: HTMLElement) => q(hd, ".block-frame-minimize-pair") as HTMLElement;

    it("draws Merge into next pane above Minimize to header, in that order, in a labelled group", () => {
        const pair = pairOf(header({ node: { canMergeIntoNeighbour: () => true, mergeIntoNeighbour: () => true } }));
        expect(pair.getAttribute("role")).toBe("group");
        expect(pair.getAttribute("aria-label")).toBe("Minimize");
        const buttons = [...pair.querySelectorAll("button")];
        expect(buttons.map((b) => b.getAttribute("title"))).toEqual(["Merge into next pane", "Minimize to header"]);
        expect(buttons.map((b) => b.getAttribute("aria-label"))).toEqual(["Merge into next pane", "Minimize to header"]);
        expect(buttons[0].querySelector("svg.pane-merge-glyph")).not.toBeNull();
        expect(buttons[1].querySelector("svg.pane-minimize-glyph")).not.toBeNull();
        expect(buttons[1].classList.contains("block-frame-minimize")).toBe(true);
    });

    it("Minimize calls toggleMinimize with chip, and Shift changes nothing", () => {
        const toggleMinimize = vi.fn();
        const hd = header({ node: { toggleMinimize, canMergeIntoNeighbour: () => true, mergeIntoNeighbour: () => true } });
        const minimize = q(hd, ".block-frame-minimize") as HTMLElement;
        fireEvent.click(minimize);
        fireEvent.click(minimize, { shiftKey: true });
        expect(toggleMinimize.mock.calls).toEqual([[{ chip: true }], [{ chip: true }]]);
    });

    it("Merge calls mergeIntoNeighbour and never toggles a minimize", () => {
        const toggleMinimize = vi.fn();
        const mergeIntoNeighbour = vi.fn(() => true);
        const hd = header({ node: { toggleMinimize, mergeIntoNeighbour, canMergeIntoNeighbour: () => true } });
        const merge = q(hd, ".block-frame-merge") as HTMLElement;
        fireEvent.click(merge);
        fireEvent.click(merge, { shiftKey: true });
        expect(mergeIntoNeighbour).toHaveBeenCalledTimes(2);
        expect(toggleMinimize).not.toHaveBeenCalled();
    });

    it("with no neighbour Merge is aria-disabled with the reason, stays focusable, and does nothing", () => {
        const mergeIntoNeighbour = vi.fn(() => false);
        const toggleMinimize = vi.fn();
        const hd = header({ node: { toggleMinimize, mergeIntoNeighbour, canMergeIntoNeighbour: () => false } });
        const merge = q(hd, ".block-frame-merge") as HTMLButtonElement;
        expect(merge.getAttribute("aria-disabled")).toBe("true");
        expect(merge.disabled).toBe(false);
        expect(merge.getAttribute("title")).toBe("No other pane to merge into");
        fireEvent.click(merge);
        expect(mergeIntoNeighbour).not.toHaveBeenCalled();
        expect(toggleMinimize).not.toHaveBeenCalled();
        // Minimize still works beside it.
        fireEvent.click(q(hd, ".block-frame-minimize") as HTMLElement);
        expect(toggleMinimize).toHaveBeenCalledWith({ chip: true });
    });

    it("a minimized pane's header shows only Restore, and Restore does not ask for a chip", () => {
        const toggleMinimize = vi.fn();
        const hd = header({ node: { isMinimized: () => true, toggleMinimize } });
        expect(pairOf(hd)).toBeNull();
        const restore = q(hd, ".block-frame-minimize") as HTMLElement;
        expect(restore.getAttribute("title")).toBe("Restore");
        fireEvent.click(restore);
        expect(toggleMinimize).toHaveBeenCalledWith();
    });

    it("shows neither for a pane that cannot minimize (the last expanded pane)", () => {
        const hd = header({ node: { canMinimize: () => false } });
        expect(pairOf(hd)).toBeNull();
        expect(q(hd, ".block-frame-minimize")).toBeNull();
    });
});
