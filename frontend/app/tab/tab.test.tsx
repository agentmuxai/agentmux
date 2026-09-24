// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tests for the window tab strip's <Tab> — specifically the close-button
 * click-containment contract from
 * docs/specs/SPEC_TAB_CLOSE_BUTTON_SELECT_FLASH_2026_08_25.md §§2-3: a click
 * on the "✕" must fire onClose ONLY, never bubble to the tab's own
 * onClick={onSelect}. The bubbled select is what raced SetActiveTab against
 * CloseTab on the backend and produced the select/deselect flash.
 */

import { cleanup, render } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

// Block metas the activity-flash color lookup reads (MOS.getObjectValue).
const blockMetas: Record<string, Record<string, unknown>> = {};
vi.mock("@/app/store/global", () => ({
    atoms: {},
    recordTEvent: vi.fn(),
    refocusNode: vi.fn(),
    MOS: {
        makeORef: (otype: string, oid: string) => `${otype}:${oid}`,
        getObjectValue: (oref: string) => ({ meta: blockMetas[oref.replace(/^block:/, "")] }),
    },
}));
// The real resolver lives in blockframe.tsx (a large module graph); the
// test only needs "a pane with a color yields it".
vi.mock("@/app/block/blockframe", () => ({
    computeBlockActiveBorderColor: (meta: Record<string, unknown> | undefined) =>
        (meta?.["frame:activebordercolor"] as string | undefined) ?? undefined,
}));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: {} }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/services", () => ({
    ObjectService: {
        UpdateTabName: vi.fn(() => Promise.resolve()),
        UpdateObjectMeta: vi.fn(() => Promise.resolve()),
    },
}));
vi.mock("@/app/store/mos", () => ({
    makeORef: (otype: string, oid: string) => `${otype}:${oid}`,
    useMuxObjectValue: () => [
        () => ({ otype: "tab", oid: "tab-1", version: 1, name: "Tab One", meta: {}, blockids: ["blk-1"] }),
        () => false,
    ],
}));
vi.mock("@/app/tab/tab-measure", () => ({ measureTabWidth: () => 100 }));

import { emitActivityFlash } from "@/app/notification/activity-flash";
import { Tab } from "./tab";

afterEach(() => cleanup());

function renderTab(
    overrides: { onSelect?: () => void; onClose?: (e: MouseEvent | null) => void; active?: boolean } = {}
) {
    const onSelect = overrides.onSelect ?? vi.fn();
    const onClose = overrides.onClose ?? vi.fn();
    const utils = render(() => (
        <Tab
            id="tab-1"
            active={overrides.active ?? false}
            isFirst={false}
            isBeforeActive={false}
            isDragging={false}
            tabWidth={0}
            isNew={false}
            onSelect={onSelect}
            onClose={onClose}
            onDragStart={vi.fn()}
            onLoaded={vi.fn()}
        />
    ));
    return { ...utils, onSelect, onClose };
}

describe("Tab close button", () => {
    it("fires onClose and never onSelect when the close button of a background tab is clicked", async () => {
        const { container, onSelect, onClose } = renderTab();
        const closeButton = container.querySelector<HTMLButtonElement>("[title='Close Tab']");
        expect(closeButton).not.toBeNull();

        await userEvent.click(closeButton!);

        expect(onClose).toHaveBeenCalledTimes(1);
        expect(onSelect).not.toHaveBeenCalled();
    });

    it("still fires onSelect for a click on the tab body itself", async () => {
        const { container, onSelect, onClose } = renderTab();
        const tabEl = container.querySelector<HTMLDivElement>(".tab");
        expect(tabEl).not.toBeNull();

        await userEvent.click(tabEl!.querySelector(".name")!);

        expect(onSelect).toHaveBeenCalledTimes(1);
        expect(onClose).not.toHaveBeenCalled();
    });
});

// SPEC_AGENT_ACTIVITY_TAB_FLASH_2026_09_23.md — a tab clicks on every tone
// from one of ITS panes, active or not, in that pane's color.
describe("Tab activity flash", () => {
    function stubAnimate() {
        const animate = vi.fn((..._args: unknown[]) => ({ cancel: vi.fn() }) as unknown as Animation);
        const original = HTMLElement.prototype.animate;
        HTMLElement.prototype.animate = animate as unknown as typeof HTMLElement.prototype.animate;
        return { animate, restore: () => (HTMLElement.prototype.animate = original) };
    }

    it("clicks the tab's overlay for a block in this tab, with that pane's color", () => {
        blockMetas["blk-1"] = { "frame:activebordercolor": "#f59e0b" };
        const { animate, restore } = stubAnimate();
        const { container } = renderTab();
        emitActivityFlash({ blockId: "blk-1" });
        const inner = container.querySelector<HTMLDivElement>(".tab-inner")!;
        expect(animate).toHaveBeenCalledTimes(1);
        expect(animate.mock.instances[0]).toBe(inner);
        expect(animate.mock.calls[0][1]).toMatchObject({ pseudoElement: "::before" });
        expect(inner.style.getPropertyValue("--activity-flash-base")).toBe("#f59e0b");
        delete blockMetas["blk-1"];
        restore();
    });

    it("the active tab clicks too", () => {
        const { animate, restore } = stubAnimate();
        renderTab({ active: true });
        emitActivityFlash({ blockId: "blk-1" });
        expect(animate).toHaveBeenCalledTimes(1);
        restore();
    });

    it("an uncolored pane leaves the base unset (the stylesheet falls back to accent)", () => {
        const { animate, restore } = stubAnimate();
        const { container } = renderTab();
        emitActivityFlash({ blockId: "blk-1" });
        expect(animate).toHaveBeenCalledTimes(1);
        const inner = container.querySelector<HTMLDivElement>(".tab-inner")!;
        expect(inner.style.getPropertyValue("--activity-flash-base")).toBe("");
        restore();
    });

    it("ignores a block that lives in another tab", () => {
        const { animate, restore } = stubAnimate();
        renderTab();
        emitActivityFlash({ blockId: "blk-elsewhere" });
        expect(animate).not.toHaveBeenCalled();
        restore();
    });

    it("unsubscribes on unmount", () => {
        const { animate, restore } = stubAnimate();
        const { unmount } = renderTab();
        unmount();
        emitActivityFlash({ blockId: "blk-1" });
        expect(animate).not.toHaveBeenCalled();
        restore();
    });
});
