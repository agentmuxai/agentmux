// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Kanban as a pane of its own (#9670): its manifest, its title, and what it passes the board from the
// pane's meta (the same `kanban:*` keys the Swarm's Kanban section read).

import { cleanup, render } from "@solidjs/testing-library";
import { createRoot, createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/rpc-api", () => ({ RpcApi: {} }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

// The board stands in for itself: the pane's contract with it is `KanbanSectionProps`.
const board = vi.hoisted(() => ({ mounts: 0, last: null as null | Record<string, any> }));
vi.mock("./kanban-section", () => ({
    KanbanSection: (props: Record<string, any>) => {
        board.mounts++;
        board.last = props;
        return (
            <div
                data-testid="board"
                data-visible={String(props.visible)}
                data-open={String(props.openRequest)}
                data-lanes={String(props.lanes)}
                data-collapsed={props.collapsed.join(",")}
            />
        );
    },
}));

import type { PaneTabHostContext } from "@/app/block/pane-tab-registry";
import { kanbanPaneTab } from "./kanban";
import { create, kanbanTitleOf } from "./kanban-instance";

afterEach(() => {
    cleanup();
    board.mounts = 0;
    board.last = null;
});

function setup(initial: Record<string, unknown> = {}, visibility: "active" | "dormant" = "active") {
    const [meta, setMeta] = createSignal<Record<string, unknown>>(initial);
    const written: Record<string, unknown>[] = [];
    const ctx = {
        blockId: "k1",
        meta,
        setMeta: async (patch: Record<string, unknown>) => {
            written.push(patch);
            setMeta((prev) => ({ ...prev, ...patch }));
        },
        visibility: () => visibility,
    } as unknown as PaneTabHostContext;
    let instance!: ReturnType<typeof create>;
    createRoot(() => {
        instance = create(ctx);
    });
    const view = render(() => <>{instance.component({ ctx })}</>);
    return { ...view, instance, written };
}

describe("the Kanban pane", () => {
    it("is a native, full-bleed pane that takes part in pane zoom, with no sections of its own", () => {
        expect(kanbanPaneTab.view).toBe("kanban");
        expect(kanbanPaneTab.label).toBe("Kanban");
        expect(kanbanPaneTab.capabilities).toEqual({ paneZoom: {}, noPadding: true });
        expect(kanbanPaneTab.sections).toBeUndefined();
    });

    it("mounts the board at once and passes it the pane's meta", () => {
        const { getByTestId, container } = setup({ "kanban:open": "t1", "kanban:lanes": true, "kanban:collapsed": ["review", "merged"] });
        expect(board.mounts).toBe(1);
        expect(container.querySelector(".kanban-pane")).not.toBeNull();
        const el = getByTestId("board");
        expect(el.dataset.visible).toBe("true");
        expect(el.dataset.open).toBe("t1");
        expect(el.dataset.lanes).toBe("true");
        expect(el.dataset.collapsed).toBe("review,merged");
    });

    it("tells the board it is hidden when the pane is, so its polling pauses", () => {
        expect(setup({}, "dormant").getByTestId("board").dataset.visible).toBe("false");
    });

    it("writes the board's choices to the pane's meta and clears an open request once handled", () => {
        const { getByTestId, written } = setup({ "kanban:open": "t1" });
        board.last!.onOpenHandled();
        expect(written).toContainEqual({ "kanban:open": null });
        expect(getByTestId("board").dataset.open).toBe("null");
        board.last!.onLanes(false);
        expect(written).toContainEqual({ "kanban:lanes": false });
        board.last!.onCollapsed(["backlog"]);
        expect(written).toContainEqual({ "kanban:collapsed": ["backlog"] });
    });

    it("names the pane for how many tasks need the operator", () => {
        expect(kanbanTitleOf(0)).toBe("Kanban");
        expect(kanbanTitleOf(1)).toBe("Kanban · 1 needs you");
        expect(kanbanTitleOf(3)).toBe("Kanban · 3 need you");
        const { instance } = setup();
        expect(instance.liveTitle!().text).toBe("Kanban");
        board.last!.onNeedsCount(2);
        expect(instance.liveTitle!().text).toBe("Kanban · 2 need you");
    });
});
