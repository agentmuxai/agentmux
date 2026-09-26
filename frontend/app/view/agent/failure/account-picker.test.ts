// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";
import type { Account } from "@/app/view/identity/identity-model";
import { accountPickerItems, planBind } from "./account-picker";

const acct = (id: string, email?: string): Account =>
    ({ id, name: "claude-oauth", context: email ? { email } : undefined }) as unknown as Account;

describe("planBind", () => {
    it("no candidates: nothing to do, in either mode", () => {
        expect(planBind("adopt", [])).toEqual({ kind: "none" });
        expect(planBind("switch", [])).toEqual({ kind: "none" });
    });

    it("adopt binds a lone candidate at once (the agent is already broken)", () => {
        const a = acct("a1");
        expect(planBind("adopt", [a])).toEqual({ kind: "bind", account: a });
    });

    it("switch never binds a lone candidate: the user picks a named destination", () => {
        const a = acct("a1");
        expect(planBind("switch", [a])).toEqual({ kind: "pick", candidates: [a] });
    });

    it("two or more candidates always open the picker", () => {
        const list = [acct("a1"), acct("a2")];
        expect(planBind("adopt", list)).toEqual({ kind: "pick", candidates: list });
        expect(planBind("switch", list)).toEqual({ kind: "pick", candidates: list });
    });
});

describe("accountPickerItems", () => {
    it("labels each row with the email, falling back to the name, and binds that account", () => {
        const bind = vi.fn();
        const a1 = acct("a1", "one@example.com");
        const a2 = acct("a2");
        const items = accountPickerItems([a1, a2], bind);
        expect(items.map((i) => i.label)).toEqual(["one@example.com", "claude-oauth"]);
        items[1].click!();
        expect(bind).toHaveBeenCalledWith(a2);
    });

    it("prefixes the verb for the switch menu", () => {
        const items = accountPickerItems([acct("a1", "one@example.com")], () => {}, "Switch to ");
        expect(items[0].label).toBe("Switch to one@example.com");
    });
});
