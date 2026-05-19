// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";
import { __resetDispatchLog, getRecentDispatches } from "../command-source";
import { createLaunchFlowStore } from "./launch-flow-store";
import type { LaunchFlowEvent } from "./types";

beforeEach(() => {
    __resetDispatchLog();
});

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

    describe("recordDispatch audit ring (§6.8)", () => {
        it("appends every dispatch to the global ring", () => {
            const { dispatch } = createLaunchFlowStore();
            dispatch({ type: "NameChanged", name: "alpha" });
            dispatch({ type: "RuntimeChanged", runtime: "container" });
            const records = getRecentDispatches();
            expect(records).toHaveLength(2);
            expect(records[0].slice).toBe("launch-flow-state");
            expect(records[0].key).toBeNull();
            expect((records[0].command as { type: string }).type).toBe("NameChanged");
            expect((records[1].command as { type: string }).type).toBe("RuntimeChanged");
        });

        it("captures emitted events alongside the command", () => {
            const { dispatch } = createLaunchFlowStore();
            dispatch({ type: "IdentityChanged", identityId: "ident-a" });
            const records = getRecentDispatches();
            expect(records[0].events).toEqual([
                { type: "FetchBindings", identityId: "ident-a" },
            ]);
        });

        it("tags source — defaults to 'user', honors explicit override", () => {
            const { dispatch } = createLaunchFlowStore();
            dispatch({ type: "NameChanged", name: "alpha" });
            dispatch({ type: "IdentitiesLoading" }, "system");
            const records = getRecentDispatches();
            expect(records[0].source).toBe("user");
            expect(records[1].source).toBe("system");
        });

        it("records no-op dispatches too (audit-completeness)", () => {
            const { dispatch } = createLaunchFlowStore();
            dispatch({ type: "NameChanged", name: "x" });
            dispatch({ type: "NameChanged", name: "x" }); // same → no-op
            // Audit ring records all dispatches, including no-ops, so
            // diag-panel users can see attempted-but-rejected transitions.
            expect(getRecentDispatches()).toHaveLength(2);
        });
    });
});
