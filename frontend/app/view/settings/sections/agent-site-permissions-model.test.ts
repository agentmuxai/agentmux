// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import { withKind } from "./agent-site-permissions-model";

describe("withKind", () => {
    it("adds a kind once and keeps srv's order", () => {
        expect(withKind(["microphone"], "camera", true)).toEqual(["camera", "microphone"]);
        expect(withKind(["camera"], "camera", true)).toEqual(["camera"]);
    });

    it("removes a kind, and an unknown one is dropped", () => {
        expect(withKind(["camera", "microphone"], "camera", false)).toEqual(["microphone"]);
        expect(withKind(["camera"], "screen", true)).toEqual(["camera"]);
    });
});
