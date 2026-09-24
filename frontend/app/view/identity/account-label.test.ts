// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// SPEC_ACCOUNT_EMAIL_IN_ARMORY_2026_09_23.md §3: an Armory account is called
// by its login email when the provider recorded one, else by its name.

import { describe, expect, it } from "vitest";
import { accountLabel } from "./identity-model";

describe("accountLabel", () => {
    it("is the login email when one is recorded", () => {
        expect(accountLabel({ name: "claude-oauth", context: { email: "me@example.com" } })).toBe(
            "me@example.com"
        );
    });

    it("falls back to the account name", () => {
        expect(accountLabel({ name: "claude-oauth", context: {} })).toBe("claude-oauth");
        expect(accountLabel({ name: "claude-oauth", context: { email: "" } })).toBe("claude-oauth");
        expect(accountLabel({ name: "claude-oauth" } as never)).toBe("claude-oauth");
    });
});
