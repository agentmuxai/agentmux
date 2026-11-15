// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { apiAuthFields, apiAuthPatch } from "./api-auth";

describe("an API node's auth fields", () => {
    it("a node with no auth shows none", () => {
        expect(apiAuthFields({})).toEqual({ vault: "", how: "bearer", header: "" });
        expect(apiAuthFields({ auth: null })).toEqual({ vault: "", how: "bearer", header: "" });
        expect(apiAuthFields({ auth: "api-key-x" })).toEqual({ vault: "", how: "bearer", header: "" });
    });

    it("reads a bearer entry and a named header", () => {
        expect(apiAuthFields({ auth: { vault: "api-key-github", as: "bearer" } })).toEqual({ vault: "api-key-github", how: "bearer", header: "" });
        expect(apiAuthFields({ auth: { vault: "api-key-acme", as: "header:X-Api-Key" } })).toEqual({ vault: "api-key-acme", how: "header", header: "X-Api-Key" });
    });

    it("writes the one object, and nothing when there is no entry", () => {
        expect(apiAuthPatch({ vault: " api-key-github ", how: "bearer", header: "ignored" })).toEqual({ auth: { vault: "api-key-github", as: "bearer" } });
        expect(apiAuthPatch({ vault: "api-key-acme", how: "header", header: " X-Api-Key " })).toEqual({ auth: { vault: "api-key-acme", as: "header:X-Api-Key" } });
        expect(apiAuthPatch({ vault: "  ", how: "header", header: "X" })).toEqual({ auth: undefined });
    });

    it("round-trips what it writes", () => {
        const f = { vault: "api-key-acme", how: "header" as const, header: "X-Api-Key" };
        expect(apiAuthFields({ ...apiAuthPatch(f) })).toEqual(f);
    });
});
