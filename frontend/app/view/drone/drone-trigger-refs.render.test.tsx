// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A node that reads `{{trigger.…}}` says, in words, which fields are typed and which are outside text, judged by the
 * jobs that run the drone. Real DroneViewModel and DroneView; only RPC is mocked.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        ListDronesCommand: () => Promise.resolve([]),
        ListDroneRunsCommand: () => Promise.resolve([]),
        ListBundlesCommand: () => Promise.resolve([]),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { DroneViewModel } from "./drone-model";
import { DroneView } from "./drone-view";

let model: DroneViewModel | null = null;

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
});

const field = (path: string, clean: boolean) => ({ path, kind: "text", clean, note: "" });

function mountMessage(body: string) {
    model = new DroneViewModel({ blockId: "drone-test-block", meta: () => ({}) } as any);
    const node = model.addNode("message", { x: 0, y: 0 });
    model.updateNodeData(node.id, { body });
    model.setSelected(node.id);
    model.setPayloadSchema({
        by_hand: [field("*", false)],
        jobs: [{ job_id: "j", job_name: "tasks", trigger: "poll", fields: [field("item.num", true), field("item.title", false), field("*", false)] }],
    });
    const result = render(() => <DroneView model={model!} />);
    return () => [...result.container.querySelectorAll(".drone-trigger-ref")].map((e) => e.textContent?.replace(/\s+/g, " ").trim());
}

describe("trigger references on a node", () => {
    it("says which fields are outside text and which are typed", () => {
        const refs = mountMessage("Task {{trigger.item.num}}: {{trigger.item.title}}");
        expect(refs()).toEqual(["trigger.item.num: typed", "trigger.item.title: outside text"]);
    });

    it("says nothing for a node that reads no trigger field", () => {
        expect(mountMessage("Nothing here {{var.a}}")()).toEqual([]);
    });
});
