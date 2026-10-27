// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Tests for the Connectors → Accounts row menu.

import { describe, expect, it, vi } from "vitest";

import { buildAccountRowMenu } from "./bind-to-agent-menu";
import { MENU_ICON } from "@/app/util/menu-icons";
import type { Account } from "./identity-model";

function mkAccount(over: Partial<Account> = {}): Account {
    return {
        id: "acct-1",
        name: "work-claude",
        provider: "claude" as Account["provider"],
        kind: "oauth",
        secret_ref: { backend: "oauth_config_dir", dir: "/tmp/acct-1" },
        context: {},
        assigned_agents: [],
        status: "valid",
        created_at: "0",
        updated_at: "0",
        ...over,
    };
}

describe("buildAccountRowMenu — Copy account ID and Delete account… (SPEC_ARMORY_ACCOUNTS_DELETE_AND_INLINE_DETAIL_2026_10_04 §3.1)", () => {
    it("has no Bind to Agent: an agent's account is changed from its composer", () => {
        const menu = buildAccountRowMenu(mkAccount(), vi.fn());
        expect(menu.some((m) => m.label === "Bind to Agent")).toBe(false);
        expect(menu.map((m) => m.label)).toEqual(["Copy account ID", undefined, "Delete account…"]);
    });

    it("Delete is the last item, after a separator, and opens the confirmation", () => {
        const onDelete = vi.fn();
        const menu = buildAccountRowMenu(mkAccount(), onDelete);
        const last = menu[menu.length - 1];
        expect(last.label).toBe("Delete account…");
        expect(menu[menu.length - 2].type).toBe("separator");
        last.click?.();
        expect(onDelete).toHaveBeenCalledTimes(1);
    });

    it("gives Copy account ID and Delete account their icons", () => {
        const menu = buildAccountRowMenu(mkAccount(), vi.fn());
        const icon = (label: string) => menu.find((m) => m.label === label)?.icon;
        expect(icon("Copy account ID")).toBe(MENU_ICON.copy);
        expect(icon("Delete account…")).toBe(MENU_ICON.delete);
    });

    it("is absent when the caller offers no delete", () => {
        const menu = buildAccountRowMenu(mkAccount());
        expect(menu.some((m) => m.label === "Delete account…")).toBe(false);
    });
});
