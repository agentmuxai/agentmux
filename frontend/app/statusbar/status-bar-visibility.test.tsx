// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { statusBarShown } from "./status-bar-visibility";

describe("statusBarShown", () => {
    it("is on when the setting is absent or true", () => {
        expect(statusBarShown(undefined, "running")).toBe(true);
        expect(statusBarShown(true, "running")).toBe(true);
    });

    it("is hidden only by an explicit false", () => {
        expect(statusBarShown(false, "running")).toBe(false);
        expect(statusBarShown(false, "connecting")).toBe(false);
    });

    it("comes back while the backend is down, so the offline dot and Open Maintenance stay reachable", () => {
        expect(statusBarShown(false, "crashed")).toBe(true);
    });
});
