// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import { makeHistoryAt, parseHistoryAt } from "./history-start-at";

describe("history start-at requests", () => {
    it("reads what makeHistoryAt wrote, byte and zone kept", () => {
        const at = makeHistoryAt({ at_ms: 5, at_byte: 99, zone: "agent:x:archive:7" });
        expect(parseHistoryAt(JSON.parse(JSON.stringify(at)))).toEqual(at);
    });

    it("ignores anything without a position", () => {
        expect(parseHistoryAt(null)).toBeNull();
        expect(parseHistoryAt("x")).toBeNull();
        expect(parseHistoryAt({ n: 3 })).toBeNull();
        expect(parseHistoryAt({ at_ms: "5" })).toBeNull();
    });
});
