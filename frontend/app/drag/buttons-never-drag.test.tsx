// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A press that starts on a button never starts a drag of what the button sits on: window tabs, pane tabs and
 * document tabs (the pane header and the Kanban card are covered in TileLayout.core.test.tsx and
 * kanban-card.paths.test.tsx). Where the press began is what counts, so each case presses on the close button,
 * then asks the drag source whether it may start; the same source asked after a press on its body says yes.
 * Pragmatic-dnd's adapter is a stand-in that keeps each source's `canDrag`.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const hub = vi.hoisted(() => ({ drags: [] as { element: HTMLElement; canDrag: (a?: unknown) => boolean }[] }));

vi.mock("@atlaskit/pragmatic-drag-and-drop/element/adapter", () => ({
    draggable: (cfg: { element: HTMLElement; canDrag: () => boolean }) => {
        hub.drags.push(cfg);
        return () => {};
    },
    dropTargetForElements: () => () => {},
}));
vi.mock("@atlaskit/pragmatic-drag-and-drop/prevent-unhandled", () => ({ preventUnhandled: { start() {}, stop() {} } }));
vi.mock("@/layout/index", () => ({ getLayoutModelForTabById: () => undefined, moveBlocksToTab: () => {} }));
vi.mock("@/app/drag/CrossWindowDragMonitor", () => ({
    getCurrentDragPayload: () => null,
    setCurrentDragPayload: () => {},
}));
vi.mock("@/store/global", () => ({ getApi: () => ({}) }));
vi.mock("@/app/tab/tab", () => ({
    Tab: () => (
        <div class="tab">
            <span class="tab-name">name</span>
            <button class="close" aria-label="Close tab" />
        </div>
    ),
}));
vi.mock("@/app/view/browser/hibernation", () => ({ hoverTabPill() {}, leaveTabPill() {}, pressTabPill() {} }));
vi.mock("@/app/drag/tearoff-snapshot", () => ({ prewarmWindowTabSnapshot() {} }));
vi.mock("@/app/drag/wayland-drag-image", () => ({ hideDragImageOnNativeWayland() {} }));
vi.mock("@/app/motion/solid", () => ({ usePulseWhen() {} }));

import { DroppableTab } from "@/app/tab/droppable-tab";
import { PaneTabStrip } from "@/app/element/PaneTabStrip";

const press = (el: Element) => el.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true }));

beforeEach(() => {
    hub.drags = [];
});
afterEach(cleanup);

describe("window tabs", () => {
    it("are not dragged or torn off by a press that began on the close button", () => {
        const { container } = render(() => (
            <DroppableTab
                tabId="tab-b"
                workspaceId="ws-1"
                activeTabId="tab-a"
                isActive={false}
                isFirst={false}
                isBeforeActive={false}
                allTabCount={3}
                tabIndex={1}
                tabIds={["tab-a", "tab-b", "tab-c"]}
                onSelect={() => {}}
                onClose={() => {}}
            />
        ));
        const source = hub.drags.find((d) => d.element.contains(container.querySelector(".close")))!;
        expect(source).toBeTruthy();
        press(container.querySelector(".close")!);
        expect(source.canDrag()).toBe(false);
        press(container.querySelector(".tab-name")!);
        expect(source.canDrag()).toBe(true);
    });
});

function strip(docDrag?: { docType: string; blockId: string; onReorder: () => void }) {
    const tabs = [{ id: "a" }, { id: "b" }];
    return render(() => (
        <PaneTabStrip
            tabs={tabs}
            activeId="a"
            getId={(t: { id: string }) => t.id}
            getLabel={(t: { id: string }) => t.id}
            onActivate={() => {}}
            onClose={() => {}}
            onReorder={() => {}}
            docDrag={docDrag as never}
        />
    ));
}

describe("pane tabs", () => {
    it("are not dragged by a press that began on a tab's close button, and are by one on the tab", () => {
        const { container } = strip();
        const pill = container.querySelector<HTMLElement>(".pane-tab")!;
        const source = hub.drags.find((d) => d.element === pill)!;
        expect(source).toBeTruthy();
        press(pill.querySelector(".pane-tab-close")!);
        expect(source.canDrag()).toBe(false);
        press(pill.querySelector(".pane-tab-label") ?? pill);
        expect(source.canDrag()).toBe(true);
    });
});

describe("document tabs", () => {
    it("are not dragged by a press that began on a tab's close button, and are by one on the tab", () => {
        const { container } = strip({ docType: "editor", blockId: "block-1", onReorder: () => {} });
        const pill = container.querySelector<HTMLElement>(".pane-tab")!;
        const source = hub.drags.find((d) => d.element === pill)!;
        expect(source).toBeTruthy();
        press(pill.querySelector(".pane-tab-close")!);
        expect(source.canDrag()).toBe(false);
        press(pill.querySelector(".pane-tab-label") ?? pill);
        expect(source.canDrag()).toBe(true);
    });
});
