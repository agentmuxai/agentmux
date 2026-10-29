// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { shouldWriteThroughAccountLink } from "./identity-link-writethrough";

describe("shouldWriteThroughAccountLink", () => {
    it("links a new agent or a fork to the account it was launched with", () => {
        expect(shouldWriteThroughAccountLink({ accountId: "322f", continuing: false, linked: null })).toBe(true);
    });

    it("writes nothing without an account", () => {
        expect(shouldWriteThroughAccountLink({ accountId: "", continuing: false, linked: null })).toBe(false);
        expect(shouldWriteThroughAccountLink({ accountId: "", continuing: true, linked: "" })).toBe(false);
    });

    it("keeps the link a continued agent has, whatever account its last instance ran on", () => {
        expect(shouldWriteThroughAccountLink({ accountId: "322f", continuing: true, linked: "b860" })).toBe(false);
    });

    it("links a continued agent that has none for its provider (a link for another provider does not count)", () => {
        expect(shouldWriteThroughAccountLink({ accountId: "322f", continuing: true, linked: "" })).toBe(true);
    });

    it("leaves a continued agent alone when its links could not be read", () => {
        expect(shouldWriteThroughAccountLink({ accountId: "322f", continuing: true, linked: null })).toBe(false);
    });
});
