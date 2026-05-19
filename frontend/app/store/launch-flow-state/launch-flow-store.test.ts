// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";
import { createLaunchFlowStore } from "./launch-flow-store";
import type { LaunchFlowEvent } from "./types";

describe("createLaunchFlowStore", () => {
    it("starts in initial state", () => {
        const { state } = createLaunchFlowStore();
        expect(state.form.name).toBe("");
        expect(state.form.identityId).toBe("");
        expect(state.closed).toBe(false);
    });

    it("dispatch routes through the reducer + updates state", () => {
        const { state, dispatch } = createLaunchFlowStore();
        dispatch({ type: "NameChanged", name: "hello" });
        expect(state.form.name).toBe("hello");
    });

    it("eventSink receives emitted events", () => {
        const events: LaunchFlowEvent[] = [];
        const { dispatch } = createLaunchFlowStore({
            eventSink: (e) => events.push(e),
        });
        dispatch({ type: "IdentityChanged", identityId: "a" });
        expect(events).toEqual([{ type: "FetchBindings", identityId: "a" }]);
    });

    it("multiple dispatches preserve field-leaf reactivity (Store identity)", () => {
        // Updating only name shouldn't recreate the identities subtree.
        const { state, dispatch } = createLaunchFlowStore();
        const idsRefBefore = state.identities;
        dispatch({ type: "NameChanged", name: "x" });
        // Solid's reconcile preserves unchanged subtree identity.
        // Reading state.identities again after the name dispatch
        // should yield the same proxy.
        expect(state.identities).toBe(idsRefBefore);
    });

    it("no-op dispatches don't fire eventSink", () => {
        const sink = vi.fn();
        const { dispatch } = createLaunchFlowStore({ eventSink: sink });
        dispatch({ type: "NameChanged", name: "x" });
        dispatch({ type: "NameChanged", name: "x" }); // same → no-op
        expect(sink).not.toHaveBeenCalled();
    });
});
