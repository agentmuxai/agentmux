// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createRoot, createSignal } from "solid-js";
import { describe, expect, it } from "vitest";
import type { DocumentNode } from "../types";
import { createAgentViewState } from "./state";

const md = (id: string, content = id): DocumentNode => ({
    type: "markdown",
    id,
    content,
    timestamp: 0,
});

/** Helper: run a body inside createRoot and dispose cleanly. */
function withRoot<T>(body: (dispose: () => void) => T): T {
    return createRoot((dispose) => body(dispose));
}

describe("AgentViewState", () => {
    describe("nodeIndex", () => {
        it("builds an id → index map from the document", () => {
            withRoot((dispose) => {
                const doc = createSignal<DocumentNode[]>([md("a"), md("b"), md("c")]);
                const state = createAgentViewState(doc);
                const idx = state.nodeIndex();
                expect(idx.get("a")).toBe(0);
                expect(idx.get("b")).toBe(1);
                expect(idx.get("c")).toBe(2);
                expect(idx.size).toBe(3);
                dispose();
            });
        });

        it("re-indexes when the document changes", () => {
            withRoot((dispose) => {
                const doc = createSignal<DocumentNode[]>([md("a"), md("b")]);
                const state = createAgentViewState(doc);
                expect(state.indexOf("a")).toBe(0);

                doc[1]([md("z"), md("a"), md("b")]); // prepend "z"
                expect(state.indexOf("z")).toBe(0);
                expect(state.indexOf("a")).toBe(1);
                expect(state.indexOf("b")).toBe(2);
                dispose();
            });
        });

        it("returns -1 for unknown ids via indexOf", () => {
            withRoot((dispose) => {
                const doc = createSignal<DocumentNode[]>([md("a")]);
                const state = createAgentViewState(doc);
                expect(state.indexOf("missing")).toBe(-1);
                dispose();
            });
        });
    });

    describe("stickToBottom", () => {
        it("starts true (initial mount expects auto-scroll)", () => {
            withRoot((dispose) => {
                const doc = createSignal<DocumentNode[]>([]);
                const state = createAgentViewState(doc);
                expect(state.stickToBottom()).toBe(true);
                dispose();
            });
        });

        it("disengageStickToBottom flips it off", () => {
            withRoot((dispose) => {
                const doc = createSignal<DocumentNode[]>([]);
                const state = createAgentViewState(doc);
                state.disengageStickToBottom();
                expect(state.stickToBottom()).toBe(false);
                dispose();
            });
        });

        it("engageStickToBottom flips it on AND clears any head anchor (atomic)", () => {
            withRoot((dispose) => {
                const doc = createSignal<DocumentNode[]>([]);
                const state = createAgentViewState(doc);
                state.captureHeadAnchor({ nodeId: "n5", offsetPx: 50 });
                expect(state.stickToBottom()).toBe(false);
                expect(state.headAnchor()).not.toBeNull();

                state.engageStickToBottom();
                // Both atomic: stick on, anchor cleared. Otherwise a later
                // remount would restore to the stale anchor (codex P2).
                expect(state.stickToBottom()).toBe(true);
                expect(state.headAnchor()).toBeNull();
                dispose();
            });
        });
    });

    describe("captureHeadAnchor", () => {
        it("stores the anchor and flips stickToBottom off (atomic)", () => {
            withRoot((dispose) => {
                const doc = createSignal<DocumentNode[]>([]);
                const state = createAgentViewState(doc);
                expect(state.stickToBottom()).toBe(true); // pre-condition

                state.captureHeadAnchor({ nodeId: "n5", offsetPx: 50 });

                expect(state.headAnchor()).toEqual({ nodeId: "n5", offsetPx: 50 });
                expect(state.stickToBottom()).toBe(false);
                dispose();
            });
        });

        it("clearHeadAnchor drops the anchor without touching stickToBottom", () => {
            withRoot((dispose) => {
                const doc = createSignal<DocumentNode[]>([]);
                const state = createAgentViewState(doc);
                state.captureHeadAnchor({ nodeId: "n5", offsetPx: 50 });
                state.clearHeadAnchor();
                expect(state.headAnchor()).toBeNull();
                // stickToBottom stays false — caller decides when to re-engage stick.
                expect(state.stickToBottom()).toBe(false);
                dispose();
            });
        });
    });

    describe("streamingNodeId", () => {
        it("starts null and can be set/cleared", () => {
            withRoot((dispose) => {
                const doc = createSignal<DocumentNode[]>([]);
                const state = createAgentViewState(doc);
                expect(state.streamingNodeId()).toBeNull();

                state.setStreamingNodeId("msg_1");
                expect(state.streamingNodeId()).toBe("msg_1");

                state.setStreamingNodeId(null);
                expect(state.streamingNodeId()).toBeNull();
                dispose();
            });
        });
    });

    describe("nodes pass-through", () => {
        it("exposes the document signal directly", () => {
            withRoot((dispose) => {
                const initial = [md("a"), md("b")];
                const doc = createSignal<DocumentNode[]>(initial);
                const state = createAgentViewState(doc);
                expect(state.nodes()).toBe(initial);
                dispose();
            });
        });
    });
});
