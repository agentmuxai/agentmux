// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { switchNotice } from "./switch-notice";

describe("switchNotice", () => {
    it("says nothing for a clean switch", () => {
        expect(switchNotice({ status: "switched", failed: [] }, "b@y")).toBeUndefined();
    });
    it("says the agent is reaching a safe point first", () => {
        expect(switchNotice({ status: "preparing", failed: [] }, "b@y")).toEqual([
            "info",
            "Switching to b@y",
            "The agent is reaching a safe point first; the chip counts down. Click it to switch now.",
        ]);
    });
    it("keeps the queued, unchanged and problem notices", () => {
        expect(switchNotice({ status: "queued", failed: [] }, "b@y")?.[1]).toBe("Switching to b@y after this turn");
        expect(switchNotice({ status: "unchanged", failed: [] }, "b@y")?.[1]).toBe("Staying on b@y");
        expect(switchNotice({ status: "switched", failed: ["pane 1 didn't restart"] }, "b@y")).toEqual([
            "warning",
            "Switched to b@y, with a problem",
            "pane 1 didn't restart",
        ]);
    });
});
