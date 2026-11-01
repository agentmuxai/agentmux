// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { deviceOrigin, isInjected, replayedUserMessage } from "./translator";

describe("replayedUserMessage", () => {
    const record = { type: "user", message: { role: "user", content: "Run the tests again" } };
    const fromPhone = { ...record, amx: { via: "viewer", device: "Pixel 9", message_id: "m1" } };

    it("renders a stored user record on replay only: live, the pane already shows what it typed", () => {
        // A record with no verified origin is untyped: shown in the container, never as the human.
        expect(replayedUserMessage(record, true)).toEqual([{ type: "user_message", message: "Run the tests again", untyped: true }]);
        expect(replayedUserMessage(record, false)).toEqual([]);
        const signed = { ...record, amx: { v: 1, verified: true, origin: { kind: "user" } } };
        expect(replayedUserMessage(signed, true)).toEqual([{ type: "user_message", message: "Run the tests again", origin: { kind: "user" } }]);
        expect(replayedUserMessage({ type: "assistant" }, false)).toBeNull();
    });

    it("renders a message sent from a paired device live too, with the device", () => {
        const sent = [{ type: "user_message", message: "Run the tests again", fromDevice: "Pixel 9", untyped: true }];
        expect(replayedUserMessage(fromPhone, false)).toEqual(sent);
        expect(replayedUserMessage(fromPhone, true)).toEqual(sent);
    });

    it("renders a message no pane typed (a jekt, a broadcast) live, and on replay", () => {
        const jekt = { ...record, amx: { via: "inject" } };
        expect(isInjected(jekt)).toBe(true);
        expect(isInjected(record)).toBe(false);
        expect(isInjected(fromPhone)).toBe(false);
        expect(isInjected(null)).toBe(false);
        const live = replayedUserMessage(jekt, false);
        expect(live).toEqual([{ type: "user_message", message: "Run the tests again", timestamp: expect.any(Number), untyped: true }]);
        expect(replayedUserMessage(jekt, true)).toEqual([{ type: "user_message", message: "Run the tests again", untyped: true }]);
    });

    it("reads the device only from a viewer amx", () => {
        expect(deviceOrigin(fromPhone)).toBe("Pixel 9");
        expect(deviceOrigin(record)).toBeUndefined();
        expect(deviceOrigin({ amx: { via: "viewer", device: "" } })).toBeUndefined();
        expect(deviceOrigin({ amx: { via: "other", device: "Pixel 9" } })).toBeUndefined();
        expect(deviceOrigin(null)).toBeUndefined();
    });
});
