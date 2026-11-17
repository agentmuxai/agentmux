// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// @vitest-environment jsdom

/**
 * The portal check (docs/specs/SPEC_PANE_BACKGROUND_THEMES_2026_11_15.md §7.6, check 4): each way a pane opens a panel
 * outside its own element is opened here from a pane with a backdrop, and the panel's root must carry that pane's
 * scope. A way that is added later and mounts on `document.body` belongs in this file.
 *
 * jsdom has no layout, so placement is mocked; what is pinned is which element carries the scope, and that it is
 * gone when the panel is.
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@floating-ui/dom", () => ({
    autoUpdate: vi.fn((_ref: unknown, _el: unknown, update?: () => void) => {
        update?.();
        return vi.fn();
    }),
    computePosition: vi.fn(() => Promise.resolve({ x: 7, y: 9 })),
    flip: vi.fn(() => ({})),
    offset: vi.fn(() => ({})),
    shift: vi.fn(() => ({})),
}));
vi.mock("@/app/platform/pane-overlay", async (original) => ({
    ...(await original<typeof import("@/app/platform/pane-overlay")>()),
    useHiddenTabMarker: () => () => undefined,
    usePaneOverlay: vi.fn(),
    registerPaneOverlay: () => ({ release: () => {} }),
}));
vi.mock("@/app/motion/motion", () => ({ shouldReduceMotion: () => true, scaledMs: (ms: number) => ms }));
vi.mock("@/app/util/menu-position", () => ({
    assertMenuInPaintableArea: () => {},
    computeMenuPosition: vi.fn(async () => ({ style: { position: "fixed", left: "3px", top: "4px" }, maxHeight: 600 })),
}));
// The find bar's own parts are not the subject: the scope goes on the element the bar is mounted in.
vi.mock("@/app/element/search-parts", () => ({ SearchBar: () => <div class="search-container">find</div> }));

import { AnchoredPopover } from "@/app/element/anchored-popover";
import { Search, useSearch } from "@/app/element/search";
import { initTitleTooltips } from "@/app/element/title-tooltip";
import { resetTooltipTiming, SHOW_DELAY_MS } from "@/app/element/tooltip-timing";
import { openMenuAt } from "@/app/element/ui/open-menu-at";
import { closeJsContextMenu, showJsContextMenu } from "@/app/host/js-context-menu";
import { closeMenuAt } from "@/app/util/menu-at-open";
import { PeekOverlay } from "@/app/view/agent/components/PeekOverlay";
import { BUILTINS, deriveBuiltin } from "./builtins";
import { OVERLAY_BLOCK_ATTR } from "./overlay-scope";
import { registerPaneScope, releasePaneScope, scopeVars } from "./scope";

const vars = scopeVars(deriveBuiltin(BUILTINS[0].id, "dark")!);

/** A pane's frame with a button in it, and a button outside every pane. */
let inside: HTMLElement;
let outside: HTMLElement;

beforeEach(() => {
    const frame = document.body.appendChild(document.createElement("div"));
    frame.dataset.blockid = "pane-1";
    inside = frame.appendChild(document.createElement("button"));
    inside.textContent = "in the pane";
    outside = document.body.appendChild(document.createElement("button"));
    registerPaneScope("pane-1", vars);
});

afterEach(() => {
    closeMenuAt();
    closeJsContextMenu();
    cleanup();
    releasePaneScope("pane-1");
    vi.useRealTimers();
    vi.unstubAllGlobals();
    document.body.replaceChildren();
});

/** The element carries the pane's scope: its ink, its overlay surface, and the two markers. */
function expectScoped(el: Element | null | undefined, what: string): void {
    expect(el, what).toBeTruthy();
    const root = el as HTMLElement;
    expect(root.getAttribute(OVERLAY_BLOCK_ATTR), what).toBe("pane-1");
    expect(root.hasAttribute("data-backdrop"), what).toBe(true);
    expect(root.style.getPropertyValue("--ink-text"), what).toBe(vars["--ink-text"]);
    expect(root.style.getPropertyValue("--main-bg-color"), what).toContain("var(--surface-overlay)");
}

function expectUnscoped(el: Element | null | undefined, what: string): void {
    expect(el, what).toBeTruthy();
    const root = el as HTMLElement;
    expect(root.hasAttribute(OVERLAY_BLOCK_ATTR), what).toBe(false);
    expect(root.hasAttribute("data-backdrop"), what).toBe(false);
    expect(root.style.getPropertyValue("--ink-text"), what).toBe("");
}

const shellOf = (cls: string) => document.querySelector<HTMLElement>(`.${cls}`)?.parentElement;

describe("a popover (AnchoredPopover: status panels, the kit Menu, tooltips)", () => {
    it("takes the scope of the pane its anchor is in, from its first paint, and keeps it when it is placed", async () => {
        vi.useFakeTimers();
        render(() => (
            <AnchoredPopover anchor={inside} class="my-panel">
                content
            </AnchoredPopover>
        ));
        expectScoped(shellOf("my-panel"), "at mount");
        await vi.advanceTimersByTimeAsync(50);
        const shell = shellOf("my-panel")!;
        expect(shell.style.left).toBe("3px");
        expectScoped(shell, "after it is placed");
    });

    it("anchored to a rect or a point, takes the pane it is told", () => {
        render(() => (
            <AnchoredPopover anchor={new DOMRect(5, 5, 0, 0)} scope={inside} class="at-point">
                content
            </AnchoredPopover>
        ));
        expectScoped(shellOf("at-point"), "with scope");
        render(() => (
            <AnchoredPopover anchor={new DOMRect(5, 5, 0, 0)} class="no-scope">
                content
            </AnchoredPopover>
        ));
        expectUnscoped(shellOf("no-scope"), "a rect and no scope");
    });

    it("opened from outside every pane, or from a pane with no backdrop, is left as it was", () => {
        render(() => (
            <AnchoredPopover anchor={outside} class="from-chrome">
                content
            </AnchoredPopover>
        ));
        expectUnscoped(shellOf("from-chrome"), "from the window's chrome");
        releasePaneScope("pane-1");
        render(() => (
            <AnchoredPopover anchor={inside} class="plain-pane">
                content
            </AnchoredPopover>
        ));
        expectUnscoped(shellOf("plain-pane"), "from a pane with no backdrop");
    });

    it("follows the pane: a backdrop removed while the panel is open leaves the panel in the window's theme", () => {
        render(() => (
            <AnchoredPopover anchor={inside} class="my-panel">
                content
            </AnchoredPopover>
        ));
        expectScoped(shellOf("my-panel"), "open");
        registerPaneScope("pane-1", undefined);
        expectUnscoped(shellOf("my-panel"), "backdrop removed");
    });
});

describe("a right-click menu (openMenuAt, the kit Menu at the pointer)", () => {
    const shells = () => [...document.querySelectorAll<HTMLElement>('.anchored-popover--menu:not([aria-hidden="true"])')];
    const items: ContextMenuItem[] = [{ label: "Copy" }, { label: "Colour", submenu: [{ label: "Red" }] }];
    const at = (target: EventTarget | null) => ({ stopPropagation: () => {}, clientX: 10, clientY: 10, target });

    it("takes the scope of the pane that was clicked, and so does a submenu it opens", async () => {
        vi.useFakeTimers();
        openMenuAt(items, at(inside));
        expect(shells()).toHaveLength(1);
        expectScoped(shells()[0], "the menu");
        const row = [...document.querySelectorAll<HTMLElement>('[role^="menuitem"]')].find((r) => r.textContent?.includes("Colour"))!;
        fireEvent.mouseEnter(row);
        await vi.advanceTimersByTimeAsync(200);
        expect(shells()).toHaveLength(2);
        // The submenu hangs from a row of the menu, which is not in the pane: it finds the pane through the menu.
        expectScoped(shells()[1], "the submenu");
    });

    it("opened on the window's chrome is left as it was", () => {
        openMenuAt(items, at(outside));
        expectUnscoped(shells()[0], "a menu on the chrome");
        closeMenuAt();
        openMenuAt(items, { stopPropagation: () => {}, clientX: 10, clientY: 10 });
        expectUnscoped(shells()[0], "a menu with no target");
    });
});

describe("the JS context menu (showJsContextMenu)", () => {
    const overlay = () => document.getElementById("cef-context-menu-overlay");
    const menu: NativeContextMenuItem[] = [{ id: "copy", label: "Copy" }];

    it("takes the scope of the pane under the pointer, every level inside it, and drops it when it closes", () => {
        const fromPoint = vi.fn(() => inside);
        (document as any).elementFromPoint = fromPoint;
        try {
            showJsContextMenu(menu, { x: 40, y: 50 }, null);
            expect(fromPoint).toHaveBeenCalledWith(40, 50);
            const root = overlay();
            expectScoped(root, "the overlay");
            expect(root!.querySelector(".menu-item"), "its rows are inside the scoped overlay").toBeTruthy();
            closeJsContextMenu();
            expect(overlay()).toBeNull();
            expectUnscoped(root, "after it closed");
        } finally {
            delete (document as any).elementFromPoint;
        }
    });

    it("opened on the window's chrome is left as it was", () => {
        (document as any).elementFromPoint = () => outside;
        try {
            showJsContextMenu(menu, { x: 40, y: 50 }, null);
            expectUnscoped(overlay(), "the overlay");
        } finally {
            delete (document as any).elementFromPoint;
        }
    });
});

describe("a title tooltip (title-tooltip.ts)", () => {
    const shell = () => document.body.querySelector<HTMLElement>(".anchored-popover--tooltip");
    const over = (el: Element) => el.dispatchEvent(new Event("pointerover", { bubbles: true }));

    it("takes the scope of the pane its element is in, and a tooltip for the chrome after it does not keep it", async () => {
        vi.useFakeTimers();
        const stop = initTitleTooltips(document);
        resetTooltipTiming();
        try {
            inside.title = "Close the pane";
            outside.title = "Open settings";
            over(inside);
            await vi.advanceTimersByTimeAsync(SHOW_DELAY_MS + 20);
            expectScoped(shell(), "a tooltip in the pane");
            // The one shell is used again for the next element.
            over(outside);
            await vi.advanceTimersByTimeAsync(SHOW_DELAY_MS + 20);
            expect(shell()?.textContent).toBe("Open settings");
            expectUnscoped(shell(), "a tooltip on the chrome");
        } finally {
            stop();
        }
    });
});

describe("the find bar (Search)", () => {
    it("takes the scope of the pane it searches, on the element it is mounted in", () => {
        const search = useSearch({ anchorRef: { current: inside } });
        render(() => <Search {...search} />);
        expect(document.querySelector(".search-container")).toBeNull();
        search.isOpen._set(true);
        const bar = document.querySelector(".search-container");
        expectScoped(bar?.parentElement, "the bar's mount");
        // The bar itself is free to rewrite its whole style when it is placed.
        expect((bar as HTMLElement).hasAttribute("data-backdrop")).toBe(false);
    });

    it("over something that is in no pane is left as it was", () => {
        const search = useSearch({ anchorRef: { current: outside } });
        render(() => <Search {...search} />);
        search.isOpen._set(true);
        expectUnscoped(document.querySelector(".search-container")?.parentElement, "the bar's mount");
    });
});

describe("the agent pane's hover panel (PeekOverlay)", () => {
    it("takes the scope of the pane its row is in, on the element it is mounted in", () => {
        render(() => (
            <PeekOverlay show={true} rowEl={() => inside}>
                <span>peek content</span>
            </PeekOverlay>
        ));
        const panel = screen.getByText("peek content").closest(".agent-node-peek-overlay");
        expectScoped(panel?.parentElement, "the panel's mount");
    });

    it("for a row in no pane is left as it was", () => {
        render(() => (
            <PeekOverlay show={true} rowEl={() => outside}>
                <span>peek content</span>
            </PeekOverlay>
        ));
        expectUnscoped(screen.getByText("peek content").closest(".agent-node-peek-overlay")?.parentElement, "the panel's mount");
    });
});
