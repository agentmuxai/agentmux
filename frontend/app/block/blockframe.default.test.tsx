// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What a pane's frame draws around its view and what it wires up: the frame's classes and colour
 * variables, the pieces inside it, the connection it keeps ensured, and the right-click menu on its
 * body (including the browser pane's synthetic one). The real frame is mounted over stand-in node
 * and view models. A refactor of the frame must leave the committed snapshots as they are;
 * regenerate with `vitest -u` only for a change meant to look different.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    listeners: new Map<string, (payload: any) => void>(),
    unsubs: [] as ReturnType<typeof vi.fn>[],
    ensure: vi.fn(async (..._a: unknown[]) => {}),
    menus: [] as { items: any[]; x: unknown; y: unknown }[],
}));

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: new Proxy(
        { ConnEnsureCommand: (...a: unknown[]) => h.ensure(...a) },
        { get: (t: any, k: string) => (k in t ? t[k] : async () => ({})) }
    ),
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/util/app-api", () => ({
    getApi: () =>
        new Proxy(
            {
                listen: async (name: string, cb: (payload: any) => void) => {
                    h.listeners.set(name, cb);
                    const unsub = vi.fn();
                    h.unsubs.push(unsub);
                    return unsub;
                },
            } as Record<string, unknown>,
            { get: (t, k: string) => (k in t ? t[k] : () => false) }
        ),
}));
vi.mock("@/app/element/ui", async (orig) => ({
    ...(await orig<Record<string, unknown>>()),
    openMenuAt: (items: any[], ev: { clientX?: number; clientY?: number }) => h.menus.push({ items, x: ev.clientX, y: ev.clientY }),
}));
vi.mock("@/app/modals/conntypeahead", () => ({
    ChangeConnectionBlockModal: (p: { blockId: string }) => <div data-testid="conn-modal" data-block={p.blockId} />,
}));
vi.mock("@/element/blockstats", async (orig) => ({
    ...(await orig<Record<string, unknown>>()),
    BlockStatsBadge: (p: { blockId: string; noHeader: boolean; connection?: string }) => (
        <div data-testid="stats" data-noheader={String(p.noHeader)} data-connection={p.connection ?? ""} />
    ),
}));
vi.mock("./titlebar", () => ({ TitleBar: (p: { blockId: string; title: string }) => <div data-testid="titlebar" data-title={p.title} /> }));
vi.mock("./pane-size-badge", () => ({ PaneSizeBadge: () => <div data-testid="size-badge" /> }));

import { MOS } from "@/app/store/global";
import { registerBlockComponentModel, unregisterBlockComponentModel } from "@/app/store/block-component-registry";
import type { MuxObjUpdate } from "@/types/rpc/MuxObjUpdate";
import * as util from "@/util/util";
import { registerPaneTab } from "@/app/store/pane-tab-registry";
import { resetThemeAnsi } from "@/app/backdrop/ansi";
import { closeBackdropPicker, openBackdropPicker } from "@/app/backdrop/picker-state";
import { getPaneScope } from "@/app/backdrop/scope";
import { BlockFrame, buildPaneColorSubmenu } from "./blockframe";

registerPaneTab({
    apiVersion: 1,
    view: "term",
    label: "Terminal",
    icon: "t",
    create: () => ({ component: () => null as any }),
    capabilities: { hueBorder: true, statsBadgeSetting: "term:showstatsbadge" },
});
registerPaneTab({
    apiVersion: 1,
    view: "banded",
    label: "Banded",
    icon: "b",
    create: () => ({ component: () => null as any }),
    capabilities: { statsBadgeIn: "strip" },
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
    leafs?: number;
    magnified?: boolean;
    ephemeral?: boolean;
    hoisted?: boolean;
    manageConnection?: boolean;
    dragging?: boolean;
    bodyItems?: any[];
    /** The `bg` the view model reports: a terminal's own theme background, not the block's meta. */
    bg?: string;
    /** Mount for this block id (a second frame for a block that already has one) instead of a fresh block. */
    blockId?: string;
};

function setup(over: Over = {}) {
    const blockId = over.blockId ?? `bf${++nextId}`;
    seedBlock(blockId, over.meta ?? { view: "term" });
    const [focused, setFocused] = createSignal(over.focused ?? true);
    const toggleMagnify = vi.fn();
    const onClose = vi.fn();
    const nodeModel = {
        blockId,
        isFocused: focused,
        numLeafs: () => over.leafs ?? 2,
        isMagnified: () => over.magnified ?? false,
        isEphemeral: () => over.ephemeral ?? false,
        blockNum: () => 3,
        toggleMagnify,
        onClose,
        isSplitterDragging: () => over.dragging ?? false,
        paneChromeHoisted: over.hoisted === undefined ? undefined : () => over.hoisted,
        // The header the frame builds (hidden here by the view's noHeader) reads these.
        isMinimized: () => false,
        canMinimize: () => true,
        isClosing: () => false,
        closingTabs: () => new Set<string>(),
        toggleMinimize: () => {},
        addEphemeralNodeToLayout: () => {},
        dragHandleRef: { current: null },
    } as any;
    const viewModel = {
        noHeader: () => true,
        manageConnection: over.manageConnection ? util.createSignalAtom(true) : undefined,
        blockBg: over.bg ? () => ({ bg: over.bg }) : undefined,
        getBodyContextMenuItems: (ctx?: unknown) => over.bodyItems?.map((i) => ({ ...i, ctx })) ?? [],
    } as any;
    const blockRef = { current: null as HTMLDivElement | null };
    const onClick = vi.fn();
    const onFocusCapture = vi.fn();
    const view = render(() => (
        <BlockFrame
            nodeModel={nodeModel}
            viewModel={viewModel}
            preview={over.preview ?? false}
            blockModel={{ onClick, onFocusCapture, blockRef } as any}
        >
            <div class="body-child">body</div>
        </BlockFrame>
    ));
    return { blockId, view, nodeModel, viewModel, blockRef, onClick, onFocusCapture, setFocused, toggleMagnify, onClose };
}

const markup = (el: Element | null): string => (el ? el.outerHTML : "(absent)").replace(/></g, ">\n<") + "\n";
const snap = (name: string) => `./__snapshots__/blockframe.default.${name}.html`;
const frame = (c: HTMLElement) => c.querySelector(".block-frame-default") as HTMLElement;
const flush = () => new Promise((r) => setTimeout(r, 0));

beforeEach(() => {
    (globalThis as any).ResizeObserver ??= class {
        observe() {}
        unobserve() {}
        disconnect() {}
    };
    h.listeners.clear();
    h.unsubs.length = 0;
    h.menus.length = 0;
    h.ensure.mockClear();
});
afterEach(() => cleanup());

describe("the frame's markup", () => {
    it("a focused terminal pane among others", async () => {
        const t = setup();
        await expect(markup(frame(t.view.container))).toMatchFileSnapshot(snap("term-focused"));
    });

    it("alone in its tab, magnified and ephemeral: the classes say so", async () => {
        const t = setup({ leafs: 1, magnified: true, ephemeral: true, focused: false });
        const el = frame(t.view.container);
        for (const c of ["pane-alone", "magnified", "ephemeral"]) expect(el.classList.contains(c)).toBe(true);
        expect(el.classList.contains("block-focused")).toBe(false);
        await expect(markup(el)).toMatchFileSnapshot(snap("alone-magnified"));
    });

    it("a pane with an agent's colour carries it as the border colour, and the identity colour beside it", async () => {
        const t = setup({ meta: { view: "term", "frame:activebordercolor": "#ff8800" } });
        const el = frame(t.view.container);
        expect(el.classList.contains("has-agent-color")).toBe(true);
        await expect(el.getAttribute("style")).toMatchFileSnapshot(snap("agent-color-style"));
    });

    it("a pane with a picked hue carries it as both", async () => {
        const t = setup({ meta: { view: "term", "frame:hue": 210 } });
        const el = frame(t.view.container);
        expect(el.classList.contains("has-agent-color")).toBe(true);
        await expect(el.getAttribute("style")).toMatchFileSnapshot(snap("hue-style"));
    });

    it("a drag preview shows the view's icon and none of the live pieces", async () => {
        const t = setup({ preview: true, manageConnection: true, meta: { view: "term", connection: "me@host" } });
        const el = frame(t.view.container);
        expect(el.classList.contains("block-focused")).toBe(true);
        expect(t.view.container.querySelector(".body-child")).toBeNull();
        expect(t.view.container.querySelector('[data-testid="titlebar"]')).toBeNull();
        expect(t.view.container.querySelector('[data-testid="stats"]')).toBeNull();
        expect(h.ensure).not.toHaveBeenCalled();
        await expect(markup(el)).toMatchFileSnapshot(snap("preview"));
    });

    it("hands the body to the children, draws the title bar and the stats badge", () => {
        const t = setup({ meta: { view: "term", connection: "me@host", "display:name": "Build box" } });
        const c = t.view.container;
        expect(c.querySelector(".block-frame-default-inner .body-child")).not.toBeNull();
        expect(c.querySelector('[data-testid="titlebar"]')).not.toBeNull();
        const stats = c.querySelector('[data-testid="stats"]') as HTMLElement;
        expect(stats.dataset.connection).toBe("me@host");
        expect(stats.dataset.noheader).toBe("true");
    });

    it("draws no stats badge for a pane that draws the readout itself, and keeps it for one that doesn't", () => {
        const banded = setup({ meta: { view: "banded" } }).view.container;
        expect(banded.querySelector('[data-testid="stats"]')).toBeNull();
        expect(banded.querySelector('[data-testid="titlebar"]')).not.toBeNull();
        cleanup();
        expect(setup({ meta: { view: "term" } }).view.container.querySelector('[data-testid="stats"]')).not.toBeNull();
    });

    it("keeps the splitter badge to a drag, and the block mask last", () => {
        const idle = setup();
        expect(idle.view.container.querySelector('[data-testid="size-badge"]')).toBeNull();
        expect(frame(idle.view.container).lastElementChild!.classList.contains("block-mask")).toBe(true);
        cleanup();
        const drag = setup({ dragging: true });
        expect(drag.view.container.querySelector('[data-testid="size-badge"]')).not.toBeNull();
    });

    it("hands its element to the block model and forwards click and focus", () => {
        const t = setup();
        expect(t.blockRef.current).toBe(frame(t.view.container));
        fireEvent.click(frame(t.view.container));
        expect(t.onClick).toHaveBeenCalledTimes(1);
        fireEvent.focusIn(frame(t.view.container));
        expect(t.onFocusCapture).toHaveBeenCalledTimes(1);
    });
});

describe("the backdrop", () => {
    const style = (c: HTMLElement) => frame(c).style;

    it("a pane with none carries no data-backdrop and none of the scope's properties, nor any terminal-palette override", () => {
        const t = setup();
        expect(frame(t.view.container).hasAttribute("data-backdrop")).toBe(false);
        expect(style(t.view.container).getPropertyValue("--term-green")).toBe("");
        expect(style(t.view.container).getPropertyValue("--surface")).toBe("");
        expect(style(t.view.container).getPropertyValue("--main-text-color")).toBe("");
    });

    it("a built-in in the pane's meta puts its surface, ink and the re-pointed tokens on the frame, named by its id", () => {
        const t = setup({ meta: { view: "agent", backdrop: { kind: "builtin", id: "teal", polarity: "follow", version: 1 } } });
        const el = frame(t.view.container);
        expect(el.getAttribute("data-backdrop")).toBe("builtin:teal");
        expect(el.style.getPropertyValue("--surface")).toMatch(/^#[0-9a-f]{6}$/);
        expect(el.style.getPropertyValue("--main-text-color")).toBe("var(--ink-text)");
        expect(el.style.getPropertyValue("--block-agent-color")).not.toBe("");
        // The transcript's terminal palette is set again on the pane when the theme gives one.
        resetThemeAnsi();
        document.documentElement.style.setProperty("--term-green", "#4e9a06");
        const withPalette = setup({ meta: { view: "term", backdrop: { kind: "builtin", id: "teal", polarity: "follow", version: 1 } } });
        expect(frame(withPalette.view.container).style.getPropertyValue("--term-green")).toMatch(/^#[0-9a-f]{6}$/);
        document.documentElement.style.removeProperty("--term-green");
        resetThemeAnsi();
    });

    it("a plain colour in the block's own bg meta is a legacy backdrop; a gradient is left untouched", () => {
        const plain = setup({ meta: { view: "agent", bg: "#336699" } });
        expect(frame(plain.view.container).getAttribute("data-backdrop")).toBe("legacy");
        cleanup();
        const gradient = setup({ meta: { view: "agent", bg: "linear-gradient(red, blue)" } });
        expect(frame(gradient.view.container).hasAttribute("data-backdrop")).toBe(false);
    });

    it("a terminal whose view model reports its theme's background, with no bg in the block's meta, gets no backdrop and no vars", () => {
        const t = setup({ bg: "#1e1e2e" });
        expect(frame(t.view.container).hasAttribute("data-backdrop")).toBe(false);
        expect(style(t.view.container).getPropertyValue("--surface")).toBe("");
        expect(getPaneScope(t.blockId)()).toBeUndefined();
    });

    it("a drag preview of a block reads its live scope, and its unmounting leaves that scope", () => {
        const meta = { view: "agent", backdrop: { kind: "builtin", id: "teal", polarity: "follow", version: 1 } };
        const live = setup({ meta });
        const scope = getPaneScope(live.blockId)();
        expect(scope?.["--surface"]).toMatch(/^#[0-9a-f]{6}$/);
        const preview = setup({ blockId: live.blockId, preview: true, meta });
        const previewFrame = [...preview.view.container.querySelectorAll<HTMLElement>(".block-frame-default")].pop()!;
        expect(previewFrame.style.getPropertyValue("--surface")).toBe(scope?.["--surface"]);
        expect(previewFrame.getAttribute("data-backdrop")).toBe("builtin:teal");
        preview.view.unmount();
        expect(getPaneScope(live.blockId)()).toEqual(scope);
        expect(frame(live.view.container).style.getPropertyValue("--surface")).toBe(scope?.["--surface"]);
    });

    it("draws the picker inside the frame while it is open for this pane, and not for another or in a drag preview", () => {
        const t = setup();
        expect(t.view.container.querySelector(".backdrop-picker")).toBeNull();
        openBackdropPicker(t.blockId);
        expect(frame(t.view.container).querySelector(".backdrop-picker")).not.toBeNull();
        closeBackdropPicker(t.blockId);
        expect(t.view.container.querySelector(".backdrop-picker")).toBeNull();
        cleanup();
        const other = setup();
        openBackdropPicker("some-other-block");
        expect(other.view.container.querySelector(".backdrop-picker")).toBeNull();
        cleanup();
        const preview = setup({ preview: true });
        openBackdropPicker(preview.blockId);
        expect(preview.view.container.querySelector(".backdrop-picker")).toBeNull();
    });

    it("an agent pane's menu has a Backdrop… entry, after Pane Color, that opens the picker; other panes have none yet", () => {
        const t = setup({ meta: { view: "agent" } });
        const items = buildPaneColorSubmenu({ oid: t.blockId, meta: { view: "agent" } } as unknown as Block);
        const labels = items.map((i) => i.label);
        expect(labels.indexOf("Backdrop…")).toBe(labels.indexOf("Pane Color") + 1);
        items.find((i) => i.label === "Backdrop…")!.click!();
        expect(frame(t.view.container).querySelector(".backdrop-picker")).not.toBeNull();
        for (const view of ["term", "files", "browser"]) {
            const other = buildPaneColorSubmenu({ oid: "x", meta: { view } } as unknown as Block);
            expect(other.map((i) => i.label), view).not.toContain("Backdrop…");
        }
    });

    it("a terminal, a files pane and the browser ignore a backdrop record and an old plain bg: no vars, no data-backdrop, nothing registered", () => {
        const record = { kind: "builtin", id: "teal", polarity: "follow", version: 1 };
        for (const view of ["term", "files", "browser", "sysinfo"]) {
            const t = setup({ meta: { view, backdrop: record, bg: "#336699" } });
            expect(frame(t.view.container).hasAttribute("data-backdrop"), view).toBe(false);
            expect(style(t.view.container).getPropertyValue("--surface"), view).toBe("");
            expect(style(t.view.container).getPropertyValue("--main-text-color"), view).toBe("");
            expect(getPaneScope(t.blockId)(), view).toBeUndefined();
            cleanup();
        }
    });

    it("a drag preview of a block that is unmounted leaves the live pane's open picker open, and Backdrop… still reopens it", () => {
        const meta = { view: "agent" };
        const live = setup({ meta });
        openBackdropPicker(live.blockId);
        expect(frame(live.view.container).querySelector(".backdrop-picker")).not.toBeNull();
        const preview = setup({ blockId: live.blockId, preview: true, meta });
        preview.view.unmount();
        expect(frame(live.view.container).querySelector(".backdrop-picker")).not.toBeNull();
        closeBackdropPicker(live.blockId);
        expect(frame(live.view.container).querySelector(".backdrop-picker")).toBeNull();
        buildPaneColorSubmenu({ oid: live.blockId, meta } as unknown as Block)
            .find((i) => i.label === "Backdrop…")!
            .click!();
        expect(frame(live.view.container).querySelector(".backdrop-picker")).not.toBeNull();
    });

    it("a terminal with a backdrop record already in its meta renders exactly as one without", () => {
        const record = { kind: "builtin", id: "teal", polarity: "follow", version: 1 };
        const bare = setup({ meta: { view: "term" } });
        const bareMarkup = frame(bare.view.container).outerHTML.replaceAll(bare.blockId, "ID");
        cleanup();
        const withRecord = setup({ meta: { view: "term", backdrop: record, bg: "#336699" } });
        expect(frame(withRecord.view.container).outerHTML.replaceAll(withRecord.blockId, "ID")).toBe(bareMarkup);
    });

    it("a record that can't be read is no backdrop", () => {
        const t = setup({ meta: { view: "agent", backdrop: { kind: "builtin", id: "not-a-colour", version: 1 } } });
        expect(frame(t.view.container).hasAttribute("data-backdrop")).toBe(false);
    });
});

describe("the connection", () => {
    it("is ensured once for a pane the view lets the frame manage, and not for others", async () => {
        const t = setup({ manageConnection: true, meta: { view: "term", connection: "me@host" } });
        await flush();
        expect(h.ensure).toHaveBeenCalledTimes(1);
        expect(h.ensure.mock.calls[0][1]).toEqual({ connname: "me@host", logblockid: t.blockId });
        expect(h.ensure.mock.calls[0][2]).toEqual({ timeout: 60000 });
        cleanup();
        h.ensure.mockClear();
        setup({ meta: { view: "term", connection: "me@host" } });
        setup({ manageConnection: true, meta: { view: "term" } });
        await flush();
        expect(h.ensure).not.toHaveBeenCalled();
    });

    it("lets the block's component model open the switch-connection modal, and takes that away on unmount", async () => {
        const bcm: { openSwitchConnection: null | (() => void) } = { openSwitchConnection: null };
        const t = (() => {
            // The model is registered before the frame mounts, like the view's own does.
            const id = `bf${nextId + 1}`;
            registerBlockComponentModel(id, bcm as any);
            return setup({ manageConnection: true, meta: { view: "term" } });
        })();
        expect(typeof bcm.openSwitchConnection).toBe("function");
        expect(t.view.container.querySelector('[data-testid="conn-modal"]')).toBeNull();
        bcm.openSwitchConnection!();
        await flush();
        expect(t.view.container.querySelector('[data-testid="conn-modal"]')?.getAttribute("data-block")).toBe(t.blockId);
        cleanup();
        expect(bcm.openSwitchConnection).toBeNull();
        unregisterBlockComponentModel(t.blockId);
    });
});

describe("the body's right-click menu", () => {
    it("opens the view's items, then the pane's own, at the click", () => {
        const t = setup({ bodyItems: [{ label: "View item" }] });
        fireEvent.contextMenu(frame(t.view.container), { clientX: 12, clientY: 34 });
        expect(h.menus).toHaveLength(1);
        expect([h.menus[0].x, h.menus[0].y]).toEqual([12, 34]);
        const labels = h.menus[0].items.map((i) => i.label ?? i.type);
        expect(labels[0]).toBe("View item");
        expect(labels.length).toBeGreaterThan(1);
        expect(labels).toMatchInlineSnapshot(`
          [
            "View item",
            "separator",
            "Copy",
            "separator",
            "Split Right",
            "Split Down",
            "separator",
            "Magnify Pane",
            "Minimize to Header",
            "Close Pane",
          ]
        `);
    });

    it("does nothing for a drag preview", () => {
        const t = setup({ preview: true });
        fireEvent.contextMenu(frame(t.view.container), { clientX: 1, clientY: 2 });
        expect(h.menus).toHaveLength(0);
    });

    it("listens for the browser pane's own menu, and answers only for its block", async () => {
        const t = setup({ meta: { view: "web" }, bodyItems: [{ label: "Browser item" }] });
        await flush();
        const listener = h.listeners.get("browser-pane-context-menu")!;
        expect(listener).toBeTypeOf("function");
        const placeholder = document.createElement("div");
        placeholder.className = "browser-placeholder";
        placeholder.getBoundingClientRect = () => ({ left: 100, top: 50, width: 0, height: 0, right: 0, bottom: 0, x: 100, y: 50, toJSON: () => ({}) });
        frame(t.view.container).appendChild(placeholder);

        listener({ block_id: "someone-else", x: 1, y: 2, link_url: "", selection_text: "", is_editable: false, can_go_back: false, can_go_forward: false });
        expect(h.menus).toHaveLength(0);

        listener({ block_id: t.blockId, x: 7, y: 9, link_url: "https://x.test/", selection_text: "", is_editable: true, can_go_back: true, can_go_forward: false });
        expect(h.menus).toHaveLength(1);
        expect([h.menus[0].x, h.menus[0].y]).toEqual([107, 59]);
        const first = h.menus[0].items[0];
        expect(first.label).toBe("Browser item");
        expect(first.ctx).toEqual({ x: 7, y: 9, linkUrl: "https://x.test/", selectionText: undefined, isEditable: true, canGoBack: true, canGoForward: false });
    });

    it("stops listening when the frame goes", async () => {
        setup();
        await flush();
        expect(h.unsubs).toHaveLength(1);
        cleanup();
        await flush();
        expect(h.unsubs[0]).toHaveBeenCalledTimes(1);
    });
});
