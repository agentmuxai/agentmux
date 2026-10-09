// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";

const hub = vi.hoisted(() => ({ recordTurn: vi.fn() }));
vi.mock("./token-usage", () => ({ recordTurn: (...args: unknown[]) => hub.recordTurn(...args) }));
vi.mock("./mps", () => ({ muxEventSubscribe: vi.fn() }));

import { recordAmbientSpend } from "./ambient-spend";

beforeEach(() => hub.recordTurn.mockReset());

describe("recordAmbientSpend", () => {
    it("records a purpose's spend under ambient:<purpose>", () => {
        const tokens = { input: 120, output: 9, cacheCreation: 0, cacheRead: 4000 };
        recordAmbientSpend({ purpose: "continuity_state", tokens });
        expect(hub.recordTurn).toHaveBeenCalledWith("ambient:continuity_state", tokens);
    });

    it("ignores a payload without a purpose or tokens", () => {
        for (const data of [undefined, {}, { purpose: "", tokens: { input: 1, output: 1 } }, { purpose: "narration" }]) {
            recordAmbientSpend(data);
        }
        expect(hub.recordTurn).not.toHaveBeenCalled();
    });
});
