// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it, vi } from "vitest";
import { update } from "./reducer";
import { mk, ready } from "./reducer-test-fixtures";
import type { AgentPaneCommand } from "./types";

describe("a command type the reducer has no handler for", () => {
    afterEach(() => vi.restoreAllMocks());

    // An older frontend can meet a command a newer srv sends.
    const unknown = (type: string) => ({ type, at: 1 }) as unknown as AgentPaneCommand;

    it("returns the state unchanged and warns once per type", () => {
        const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
        const state = ready();
        const first = update(state, unknown("FromTheFuture"));
        expect(first.state).toBe(state);
        expect(first.events).toEqual([]);
        update(state, unknown("FromTheFuture"));
        expect(warn).toHaveBeenCalledTimes(1);
        update(state, unknown("AnotherFromTheFuture"));
        expect(warn).toHaveBeenCalledTimes(2);
    });

    it("does not take a name from Object.prototype for a handler", () => {
        vi.spyOn(console, "warn").mockImplementation(() => {});
        const state = mk();
        expect(update(state, unknown("constructor")).state).toBe(state);
        expect(update(state, unknown("toString")).state).toBe(state);
    });
});
