// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { deviceOrigin, replayedUserMessage } from "./translator";

describe("replayedUserMessage", () => {
    const record = { type: "user", message: { role: "user", content: "Run the tests again" } };
    const fromPhone = { ...record, amx: { via: "viewer", device: "Pixel 9", message_id: "m1" } };

    it("renders a stored user record on replay only: live, the pane already shows what it typed", () => {
        expect(replayedUserMessage(record, true)).toEqual([{ type: "user_message", message: "Run the tests again" }]);
        expect(replayedUserMessage(record, false)).toEqual([]);
        expect(replayedUserMessage({ type: "assistant" }, false)).toBeNull();
    });

    it("renders a message sent from a paired device live too, with the device", () => {
        const sent = [{ type: "user_message", message: "Run the tests again", fromDevice: "Pixel 9" }];
        expect(replayedUserMessage(fromPhone, false)).toEqual(sent);
        expect(replayedUserMessage(fromPhone, true)).toEqual(sent);
    });

    it("reads the device only from a viewer amx", () => {
        expect(deviceOrigin(fromPhone)).toBe("Pixel 9");
        expect(deviceOrigin(record)).toBeUndefined();
        expect(deviceOrigin({ amx: { via: "viewer", device: "" } })).toBeUndefined();
        expect(deviceOrigin({ amx: { via: "other", device: "Pixel 9" } })).toBeUndefined();
        expect(deviceOrigin(null)).toBeUndefined();
    });
});
