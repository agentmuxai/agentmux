// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A right-click on a document tab shows the tab's own menu and nothing of the pane's: it ends at "Close to the Right"
 * (the body's Word Wrap and the pane's actions have their own menus). A right-click on the strip with no tab under it
 * is the pane menu as before.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@atlaskit/pragmatic-drag-and-drop/element/adapter", () => ({
    draggable: () => () => {},
    dropTargetForElements: () => () => {},
    monitorForElements: () => () => {},
}));

import { resolveContextMenuRegion } from "@/app/block/context-menu-region";
import { DocTabStrip } from "./DocTabStrip";
import { DocTabsController, type DocTabsSpec } from "./doc-tabs-controller";

type P = { path: string };
const spec: DocTabsSpec<P> = {
    keyOf: (p) => p.path,
    titleOf: (p) => p.path.slice(1),
    serialize: (p) => p.path,
    deserialize: (st) => (typeof st === "string" ? { path: st } : null),
};

function mount(paths: string[], docType: string, menuItems?: () => ContextMenuItem[]) {
    const ctl = new DocTabsController(spec, { meta: () => ({}), setMeta: () => {} }, paths.map((path) => ({ path })));
    const view = render(() => (
        <DocTabStrip ctl={ctl} docType={docType} blockId="b1" show={() => true} menuItems={menuItems} pathOf={(t) => t.key} />
    ));
    const row = view.container.querySelector(".doc-tab-strip") as HTMLElement;
    return { ctl, view, row };
}

/** The region the pane's body menu resolves for a right-click on `target`, after the click has been noted as it is in the page. */
function rightClick(target: Element) {
    fireEvent.contextMenu(target);
    return resolveContextMenuRegion(target);
}

const labels = (items: ContextMenuItem[]) => items.map((i) => i.label ?? "-");

afterEach(() => cleanup());

describe.each(["editor", "media"])("the document tab menu of a %s pane", (docType) => {
    it("ends at Close to the Right and takes none of the pane's menu", () => {
        const { view } = mount(["/a.ts", "/b.ts", "/c.ts"], docType);
        const tab = view.container.querySelector("[data-tab-id]") as HTMLElement;
        const region = rightClick(tab)!;
        const items = region.items!();
        expect(labels(items).at(-1)).toBe("Close to the Right");
        expect(items.at(-1)?.type).not.toBe("separator");
        // Everything the pane adds below the tab's items is dropped: Word Wrap and the like (viewItems), Copy and Paste,
        // Split, Magnify and Minimize to Header, Close Pane and Inspect Element.
        const omit = typeof region.omit === "function" ? region.omit() : region.omit;
        expect(new Set(omit)).toEqual(new Set(["viewItems", "clipboard", "split", "magnify", "close", "inspect"]));
    });

    it("keeps the pane's own tab items above the strip's", () => {
        const own = [{ label: "Rename..." }, { label: "Reveal" }];
        const { view } = mount(["/a.ts", "/b.ts"], docType, () => own);
        const region = rightClick(view.container.querySelector("[data-tab-id]") as HTMLElement)!;
        expect(labels(region.items!()).slice(0, 2)).toEqual(["Rename...", "Reveal"]);
        expect(labels(region.items!()).at(-1)).toBe("Close to the Right");
    });
});

describe("a right-click on the strip with no tab under it", () => {
    it("adds no tab items and drops nothing of the pane menu", () => {
        const { row } = mount(["/a.ts", "/b.ts"], "editor");
        const region = rightClick(row)!;
        expect(region.items!()).toEqual([]);
        const omit = typeof region.omit === "function" ? region.omit() : region.omit;
        expect(omit ?? []).toEqual([]);
    });

    it("follows the last click: a tab, then the empty strip", () => {
        const { view, row } = mount(["/a.ts", "/b.ts"], "editor");
        const tab = view.container.querySelector("[data-tab-id]") as HTMLElement;
        const onTab = rightClick(tab)!;
        expect((typeof onTab.omit === "function" ? onTab.omit() : onTab.omit)?.length).toBeGreaterThan(0);
        const onRow = rightClick(row)!;
        expect(typeof onRow.omit === "function" ? onRow.omit() : onRow.omit).toEqual([]);
    });
});
