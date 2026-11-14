// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Agent node's "Reply fields" (`output_schema`): typed as JSON, kept on the node only while it is an object,
 * cleared when emptied, and the person's own text is not rewritten while they type. Real DroneViewModel and DroneView;
 * only RPC is mocked.
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

function mountSelectedAgent(data: Record<string, unknown> = {}) {
    model = new DroneViewModel({ blockId: "drone-test-block", meta: () => ({}) } as any);
    const node = model.addNode("agent", { x: 0, y: 0 });
    model.updateNodeData(node.id, data);
    model.setSelected(node.id);
    const result = render(() => <DroneView model={model!} />);
    const field = () => {
        const label = [...result.container.querySelectorAll(".drone-node-field-label")].find((l) => l.textContent === "Reply fields");
        return label?.parentElement?.querySelector("textarea") as HTMLTextAreaElement;
    };
    const dataOf = () => model!.draftAtom().graph.nodes.find((n) => n.id === node.id)!.data;
    const type = (text: string) => {
        const el = field();
        el.value = text;
        el.dispatchEvent(new Event("input", { bubbles: true }));
    };
    const note = () => result.container.querySelector(".drone-node-field-note")?.textContent ?? null;
    return { field, dataOf, type, note };
}

describe("Agent node reply fields", () => {
    it("is on the Agent node, empty at first", () => {
        const { field, dataOf } = mountSelectedAgent();
        expect(field()).toBeDefined();
        expect(field().value).toBe("");
        expect(dataOf()["output_schema"]).toBeUndefined();
    });

    it("keeps a typed object on the node", () => {
        const { type, dataOf, note } = mountSelectedAgent();
        type('{"verdict": {"enum": ["ok", "findings"]}, "cause": "string"}');
        expect(dataOf()["output_schema"]).toEqual({ verdict: { enum: ["ok", "findings"] }, cause: "string" });
        expect(note()).toBeNull();
    });

    it("says so while the text is not JSON yet, and keeps the last good object", () => {
        const { type, dataOf, note, field } = mountSelectedAgent();
        type('{"cause": "string"}');
        type('{"cause": "str');
        expect(note()).toBe("Not valid JSON yet");
        expect(dataOf()["output_schema"]).toEqual({ cause: "string" });
        // What they typed is not rewritten.
        expect(field().value).toBe('{"cause": "str');
    });

    it("refuses anything that is not an object", () => {
        const { type, dataOf, note } = mountSelectedAgent();
        type("[1, 2]");
        expect(note()).toBe("An object of field: type");
        type('"cause"');
        expect(note()).toBe("An object of field: type");
        expect(dataOf()["output_schema"]).toBeUndefined();
    });

    it("clears the field on the node when the text is emptied", () => {
        const { type, dataOf, note } = mountSelectedAgent({ output_schema: { cause: "string" } });
        type("   ");
        expect(dataOf()["output_schema"]).toBeNull();
        expect(note()).toBeNull();
    });

    it("shows a schema the node already has", () => {
        const { field } = mountSelectedAgent({ output_schema: { cause: "string" } });
        expect(field().value).toBe('{"cause":"string"}');
    });
});
