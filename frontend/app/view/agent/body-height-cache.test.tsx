// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import * as unloadModule from "./tool-result-unload";
import { forgetBodyHeights, rememberBodyHeights, takeBodyHeight, withBodyHeights } from "./body-height-cache";
import { ToolBlock } from "./components/ToolBlock";
import type { ToolNode } from "./types";

/** Counts every read of an open tool body's `offsetHeight`, and answers with `heightOf`. */
let reads = 0;
let heightOf = (_el: HTMLElement): number => 400;
const original = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "offsetHeight");
beforeEach(() => {
    reads = 0;
    heightOf = () => 400;
    Object.defineProperty(HTMLElement.prototype, "offsetHeight", {
        configurable: true,
        get() {
            const el = this as HTMLElement;
            if (!el.classList.contains("agent-tool-overlay")) return 0;
            reads++;
            return heightOf(el);
        },
    });
});
afterEach(() => {
    cleanup();
    if (original) Object.defineProperty(HTMLElement.prototype, "offsetHeight", original);
    forgetBodyHeights(["a", "b", "c", "tc-1"]);
});

function rows(ids: string[]): HTMLElement {
    const root = document.createElement("div");
    for (const id of ids) {
        const row = document.createElement("div");
        row.dataset.nodeId = id;
        const body = document.createElement("div");
        body.className = "agent-tool-overlay";
        body.dataset.h = String(ids.indexOf(id) + 1);
        row.append(body);
        root.append(row);
    }
    return root;
}

describe("rememberBodyHeights", () => {
    it("reads each wanted open body once, and only those", () => {
        heightOf = (el) => Number(el.dataset.h) * 100;
        const root = rows(["a", "b", "c"]);
        rememberBodyHeights(root, ["a", "c"]);
        expect(reads).toBe(2);
        expect(takeBodyHeight("a")).toBe(100);
        expect(takeBodyHeight("c")).toBe(300);
        expect(takeBodyHeight("b")).toBeUndefined();
    });

    it("gives a height once", () => {
        rememberBodyHeights(rows(["a"]), ["a"]);
        expect(takeBodyHeight("a")).toBe(400);
        expect(takeBodyHeight("a")).toBeUndefined();
    });

    it("reads nothing with no ids or no root", () => {
        rememberBodyHeights(rows(["a"]), []);
        rememberBodyHeights(null, ["a"]);
        expect(reads).toBe(0);
    });
});

describe("withBodyHeights", () => {
    const tool = (id: string): ToolNode =>
        ({
            type: "tool",
            id,
            tool: "Bash",
            params: { command: "ls" },
            status: "success",
            collapsed: true,
            summary: "Bash ls",
            result: { stdout: "x".repeat(4_000) },
            resultSource: { stream: "g:1", gen: "g1", line: 1 },
        }) as unknown as ToolNode;

    it("reads the heights of the results about to unload before it runs the unload, and forgets what was not taken", () => {
        const root = rows(["a", "b"]);
        let readsBefore = -1;
        const out = withBodyHeights(root, [tool("a"), tool("b")], new Set(["b"]), () => {
            readsBefore = reads;
            return "done";
        });
        expect(out).toBe("done");
        // "b" is kept (pinned or held), so only "a" is read.
        expect(readsBefore).toBe(1);
        expect(takeBodyHeight("a")).toBeUndefined(); // forgotten: the unload did not take it
    });

    it("plans nothing when no tool body is open", () => {
        const root = document.createElement("div");
        const plan = vi.spyOn(unloadModule, "planUnload");
        let ran = false;
        withBodyHeights(root, [tool("a")], new Set(), () => (ran = true));
        expect(ran).toBe(true);
        expect(plan).not.toHaveBeenCalled();
        plan.mockRestore();
    });

    it("forgets even when the unload throws", () => {
        expect(() =>
            withBodyHeights(rows(["a"]), [tool("a")], new Set(), () => {
                throw new Error("boom");
            })
        ).toThrow("boom");
        expect(takeBodyHeight("a")).toBeUndefined();
    });
});

// The unload used to read each row's body height as its branch was torn down, in the middle of the update, so
// each read came after the previous row's DOM change: a layout per open body.
describe("ToolBlock's height on unload", () => {
    const loaded: ToolNode = {
        type: "tool",
        id: "tc-1",
        tool: "Bash",
        params: { command: "ls" },
        status: "success",
        collapsed: false,
        summary: "Bash ls",
        result: { exitCode: 0 } as never,
    };
    const stub = { pill: null, bytes: 4_000, tokens: 1_000 };

    const mountPinned = (node: () => ToolNode) =>
        render(() => (
            <div data-node-id="tc-1">
                <ToolBlock node={node()} pinned={true} heldOpen={true} onTogglePin={() => {}} />
            </div>
        ));

    it("takes the height read beforehand and reads none of its own", () => {
        const [node, setNode] = createSignal<ToolNode>(loaded);
        const { container } = mountPinned(node);
        reads = 0;
        rememberBodyHeights(container, ["tc-1"]);
        expect(reads).toBe(1);
        setNode({ ...loaded, result: undefined, resultUnloaded: stub });
        expect(reads).toBe(1); // nothing read during the unload itself
        const placeholder = container.querySelector(".agent-tool-result-unloaded") as HTMLElement;
        expect(placeholder.style.minHeight).toBe("400px");
    });

    it("still reads its own height when the unload did not go through the batched pass", () => {
        const [node, setNode] = createSignal<ToolNode>(loaded);
        const { container } = mountPinned(node);
        reads = 0;
        setNode({ ...loaded, result: undefined, resultUnloaded: stub });
        expect(reads).toBe(1);
        const placeholder = container.querySelector(".agent-tool-result-unloaded") as HTMLElement;
        expect(placeholder.style.minHeight).toBe("400px");
    });

    it("a row that scrolled out and comes back with its result out is the same as before: a placeholder, no height to keep", () => {
        // The remembered height lives in the row's own component, which a scroll-out discards; a row mounted fresh
        // with its text already out never had one.
        const { container } = mountPinned(() => ({ ...loaded, result: undefined, resultUnloaded: stub }));
        const placeholder = container.querySelector(".agent-tool-result-unloaded") as HTMLElement;
        expect(placeholder).not.toBeNull();
        expect(placeholder.style.minHeight).toBe("");
        expect(reads).toBe(0);
    });

    it("reads no height when the row just goes away", () => {
        const { unmount } = mountPinned(() => loaded);
        reads = 0;
        unmount();
        expect(reads).toBe(0);
    });
});
