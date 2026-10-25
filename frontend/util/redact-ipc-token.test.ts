// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { redactIpcToken } from "./redact-ipc-token";

describe("redactIpcToken", () => {
    it("replaces the token and keeps the other parameters", () => {
        expect(redactIpcToken("http://localhost:5173/?ipc_port=1234&ipc_token=abc-123&windowLabel=main")).toBe(
            "http://localhost:5173/?ipc_port=1234&ipc_token=<redacted>&windowLabel=main"
        );
    });
    it("handles a token first, last, in the fragment, and repeated", () => {
        expect(redactIpcToken("http://h/?ipc_token=abc")).toBe("http://h/?ipc_token=<redacted>");
        expect(redactIpcToken("http://h/?a=1&ipc_token=abc#frag")).toBe("http://h/?a=1&ipc_token=<redacted>#frag");
        expect(redactIpcToken("http://h/#x=1&ipc_token=abc")).toBe("http://h/#x=1&ipc_token=<redacted>");
        expect(redactIpcToken("http://h/?ipc_token=a&ipc_token=b")).toBe(
            "http://h/?ipc_token=<redacted>&ipc_token=<redacted>"
        );
    });
    it("leaves an address without a token, and a look-alike parameter, alone", () => {
        expect(redactIpcToken("http://h/?windowLabel=main")).toBe("http://h/?windowLabel=main");
        expect(redactIpcToken("http://h/?my_ipc_token=abc")).toBe("http://h/?my_ipc_token=abc");
        expect(redactIpcToken("")).toBe("");
    });
});
