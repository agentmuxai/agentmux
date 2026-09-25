// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Right-click "Copy ..." menus on Drone nodes and run-panel rows.
 *
 * `.drone-node` is `user-select: none` and its collapsed summary is truncated,
 * so the full task / URL / expression / template can't be selected or read.
 * The run panel shows only the first 8 chars of a run id. Real DroneViewModel,
 * ContextMenuModel and `showCopyContextMenu` run here; only the native menu
 * bridge (`getApi().showContextMenu`), the clipboard and RPC are mocked.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
    showContextMenu: vi.fn(),
    writeText: vi.fn(() => Promise.resolve()),
}));

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        ListDronesCommand: () => Promise.resolve([]),
        ListDroneRunsCommand: () => Promise.resolve([]),
        ListBundlesCommand: () => Promise.resolve([]),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/util/clipboard", () => ({ writeText: mocks.writeText, readText: () => Promise.resolve("") }));
vi.mock("@/app/store/global", async (importOriginal) => ({
    ...(await importOriginal<Record<string, unknown>>()),
    getApi: () => ({ showContextMenu: mocks.showContextMenu }),
}));

import { ContextMenuModel } from "@/app/store/contextmenu";
import type { DroneRun } from "@/app/store/rpc-api";
import { DroneViewModel } from "./drone-model";
import type { FlowNode } from "./drone-types";
import { DroneView, nodeCopyEntries, runCopyEntries } from "./drone-view";

let model: DroneViewModel | null = null;

// jsdom has no ResizeObserver; the canvas only uses it to track its own size.
vi.stubGlobal(
    "ResizeObserver",
    class {
        observe() {}
        unobserve() {}
        disconnect() {}
    }
);

afterEach(() => {
    cleanup();
    model?.dispose();
    model = null;
    vi.clearAllMocks();
});

function mount() {
    model = new DroneViewModel({ blockId: "drone-test-block", meta: () => ({}) } as any);
    const escaped = { count: 0 };
    const result = render(() => (
        <div onContextMenu={() => escaped.count++}>
            <DroneView model={model!} />
        </div>
    ));
    return { ...result, escaped, model: model! };
}

function rightClick(el: Element | null): MouseEvent {
    expect(el).not.toBeNull();
    const e = new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 5, clientY: 6 });
    el!.dispatchEvent(e);
    return e;
}

type NativeItem = { label: string; id: string };
function labels(): string[] {
    expect(mocks.showContextMenu).toHaveBeenCalledOnce();
    return (mocks.showContextMenu.mock.calls[0][1] as NativeItem[]).map((i) => i.label);
}
function clickItem(label: string) {
    const item = (mocks.showContextMenu.mock.calls[0][1] as NativeItem[]).find((i) => i.label === label);
    expect(item, `menu item "${label}"`).toBeDefined();
    ContextMenuModel.handleContextMenuClick(item!.id);
}

const LONG_TASK = "Summarise every open PR in the repo and list the ones blocked on review for more than three days";

describe("Drone node context menu", () => {
    it("agent node: copies the FULL task (the summary is truncated) and the node id", () => {
        const { container, escaped, model: m } = mount();
        const node = m.addNode("agent", { x: 0, y: 0 });
        m.updateNodeData(node.id, { task: LONG_TASK });

        const el = container.querySelector(".drone-node");
        expect(container.querySelector(".drone-node-summary")!.textContent).not.toContain(LONG_TASK);
        const e = rightClick(el);
        expect(e.defaultPrevented).toBe(true);
        expect(escaped.count).toBe(0);
        expect(labels()).toEqual(["Copy task", "Copy node ID"]);
        clickItem("Copy task");
        expect(mocks.writeText).toHaveBeenLastCalledWith(LONG_TASK);
        clickItem("Copy node ID");
        expect(mocks.writeText).toHaveBeenLastCalledWith(node.id);
    });

    it("right-clicking the header works too, not just the node body", () => {
        const { container, escaped, model: m } = mount();
        m.updateNodeData(m.addNode("condition", { x: 0, y: 0 }).id, { expr: "{{var.count}} > 0" });
        rightClick(container.querySelector(".drone-node-header"));
        expect(escaped.count).toBe(0);
        clickItem("Copy expression");
        expect(mocks.writeText).toHaveBeenLastCalledWith("{{var.count}} > 0");
    });

    it("a node with nothing but its id still offers Copy node ID", () => {
        const { container, model: m } = mount();
        const node = m.addNode("agent", { x: 0, y: 0 }); // empty task
        rightClick(container.querySelector(".drone-node"));
        expect(labels()).toEqual(["Copy node ID"]);
        clickItem("Copy node ID");
        expect(mocks.writeText).toHaveBeenLastCalledWith(node.id);
    });

    it("the node-type palette chips (nothing to copy) keep the pane menu", () => {
        const { container, escaped } = mount();
        rightClick(container.querySelector(".drone-chip"));
        expect(mocks.showContextMenu).not.toHaveBeenCalled();
        expect(escaped.count).toBe(1);
    });
});

describe("nodeCopyEntries", () => {
    const node = (data: Record<string, unknown>): FlowNode =>
        ({ id: "n1", position: { x: 0, y: 0 }, data }) as FlowNode;

    it("offers the primary editable value per kind", () => {
        expect(nodeCopyEntries(node({ kind: "agent", task: "t" }))[0]).toEqual({ label: "Copy task", value: "t" });
        expect(nodeCopyEntries(node({ kind: "condition", expr: "x" }))[0]).toEqual({
            label: "Copy expression",
            value: "x",
        });
        expect(nodeCopyEntries(node({ kind: "response", template: "hi" }))[0]).toEqual({
            label: "Copy template",
            value: "hi",
        });
        expect(nodeCopyEntries(node({ kind: "api", url: "https://x", body: "{}" }))).toEqual([
            { label: "Copy URL", value: "https://x" },
            { label: "Copy request body", value: "{}" },
            { label: "Copy node ID", value: "n1" },
        ]);
    });

    it("variables: name=value lines", () => {
        const entries = nodeCopyEntries(
            node({
                kind: "variables",
                entries: [
                    { name: "a", value: "1" },
                    { name: "b", value: "2" },
                ],
            })
        );
        expect(entries[0]).toEqual({ label: "Copy variables", value: "a=1\nb=2" });
    });

    it("ignores non-string data instead of copying '[object Object]'", () => {
        const entries = nodeCopyEntries(node({ kind: "agent", task: { nested: true } }));
        expect(entries[0].value).toBe("");
    });
});

describe("Drone run-panel row context menu", () => {
    const RUN = {
        id: "3f2a9c1e-aaaa-bbbb-cccc-1234567890ab",
        drone_id: "d1",
        status: "failed",
        started_at: 1,
        ended_at: 2,
        block_states: {},
        output: "",
        error: "HTTP 500 from upstream",
    } as unknown as DroneRun;

    it("copies the full run id (display is truncated to 8 chars) and the error", () => {
        const { container, escaped, model: m } = mount();
        m.setRuns([RUN]);
        expect(container.querySelector(".drone-runpanel-id")!.textContent).toBe("3f2a9c1e");
        rightClick(container.querySelector(".drone-runpanel-row"));
        expect(escaped.count).toBe(0);
        expect(labels()).toEqual(["Copy run ID", "Copy error"]);
        clickItem("Copy run ID");
        expect(mocks.writeText).toHaveBeenLastCalledWith(RUN.id);
        clickItem("Copy error");
        expect(mocks.writeText).toHaveBeenLastCalledWith("HTTP 500 from upstream");
    });

    it("runCopyEntries: output offered only when present", () => {
        expect(runCopyEntries({ id: "r", output: "ok", error: "" }).filter((e) => e.value)).toEqual([
            { label: "Copy run ID", value: "r" },
            { label: "Copy output", value: "ok" },
        ]);
    });
});
