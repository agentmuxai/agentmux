// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// An Anthropic API key in a Claude agent's chip menu
// (docs/specs/SPEC_AGENT_PANE_API_KEY_ACCOUNT_SWITCH_2026_11_16.md).

import type { Account } from "@/app/view/identity/identity-model";
import { createRoot } from "solid-js";
import { describe, expect, it, vi } from "vitest";
import { KEYS_NOT_FOR_CONTAINERS } from "./key-accounts";
import { useAccountBinding, type AccountBindingDeps } from "./useAccountBinding";

vi.mock("@/app/store/pane-requests", () => ({ openPaneModal: vi.fn() }));

const plan = (id: string, email: string, updated = "2026-09-01"): Account =>
    ({ id, name: id, provider: "claude", kind: "oauth", secret_ref: { backend: "oauth_config_dir" }, context: { email }, status: "valid", updated_at: updated }) as unknown as Account;

const key = (id: string, displayName: string, status = "valid", updated = "2026-09-01"): Account =>
    ({ id, name: `${id}-name`, display_name: displayName, provider: "anthropic", kind: "api_key", secret_ref: { backend: "keychain" }, context: {}, status, updated_at: updated }) as unknown as Account;

const flush = () => new Promise((r) => setTimeout(r, 0));

function mount(o: { accounts: Account[]; links?: Array<{ provider: string; account_id: string }>; provider?: string; container?: boolean; pending?: string }) {
    const showPicker = vi.fn();
    const switchAccount = vi.fn(async () => ({ status: "switched", account_label: "", failed: [] as string[] }));
    const notify = vi.fn();
    const deps: AccountBindingDeps = {
        loadAccounts: () => o.accounts,
        subscribeAccountChanges: () => () => {},
        listIdentities: async () => o.links ?? [],
        showPicker,
        switchAccount,
        quotaNote: () => undefined,
        notify,
        prepNow: async () => ({}),
        signInAnother: vi.fn(),
    };
    const root = createRoot((dispose) => ({
        ...useAccountBinding({
            blockId: "block-1",
            agentDefinitionId: () => "agent-1",
            providerId: () => o.provider ?? "claude",
            bindExistingAccount: async () => {},
            pendingAccountId: () => o.pending,
            isContainer: () => !!o.container,
            deps,
        }),
        dispose,
    }));
    return { ...root, showPicker, switchAccount, notify };
}

const items = (m: ReturnType<typeof mount>) => {
    m.onSwitchAccount(new MouseEvent("click"));
    return m.showPicker.mock.calls[0][0] as ContextMenuItem[];
};

describe("an API key account in the chip menu", () => {
    it("is listed under its display name with the API mark, and a plan carries none", async () => {
        const m = mount({ accounts: [plan("p", "a@x", "2026-09-02"), key("k", "Work key")], links: [{ provider: "claude", account_id: "p" }] });
        await flush();
        const rows = items(m).filter((i) => i.type === "checkbox");
        expect(rows.map((i) => [i.label, i.sublabel, i.checked])).toEqual([
            ["a@x", undefined, true],
            ["Work key", "API", false],
        ]);
        m.dispose();
    });

    it("falls back to the account's name when it has no display name", async () => {
        const m = mount({ accounts: [key("k", "")] });
        await flush();
        expect(items(m).find((i) => i.sublabel === "API")?.label).toBe("k-name");
        m.dispose();
    });

    it("is offered unchecked (status unknown) but not once it is known invalid", async () => {
        const m = mount({ accounts: [key("k1", "Unchecked", "unknown"), key("k2", "Rejected", "invalid"), key("k3", "Expired", "expired")] });
        await flush();
        expect(items(m).filter((i) => i.sublabel === "API").map((i) => i.label)).toEqual(["Unchecked"]);
        m.dispose();
    });

    it("is ticked when the agent is on it, and the chip then shows its name and the mark", async () => {
        const m = mount({ accounts: [plan("p", "a@x"), key("k", "Work key")], links: [{ provider: "anthropic", account_id: "k" }] });
        await flush();
        expect(m.authEmail()).toBe("Work key · API");
        const rows = items(m).filter((i) => i.type === "checkbox");
        expect(rows.map((i) => [i.label, i.checked])).toEqual([["a@x", false], ["Work key", true]]);
        m.dispose();
    });

    it("names the key and the mark for a switch waiting on the turn", async () => {
        const m = mount({ accounts: [plan("p", "a@x"), key("k", "Work key")], links: [{ provider: "claude", account_id: "p" }], pending: "k" });
        await flush();
        expect(m.pendingAccountLabel()).toBe("Work key · API");
        m.dispose();
    });

    it("moves the agent with the same switch a plan uses, and tells which key it is", async () => {
        const m = mount({ accounts: [plan("p", "a@x"), key("k", "Work key")], links: [{ provider: "claude", account_id: "p" }] });
        await flush();
        items(m).find((i) => i.label === "Work key")!.click!();
        await flush();
        expect(m.switchAccount).toHaveBeenCalledWith("agent-1", "k");
        m.dispose();
    });

    it("is not offered to a container agent, and the menu says why", async () => {
        const m = mount({ accounts: [plan("p", "a@x"), key("k", "Work key")], links: [{ provider: "claude", account_id: "p" }], container: true });
        await flush();
        const menu = items(m);
        expect(menu.some((i) => i.sublabel === "API")).toBe(false);
        const why = menu.find((i) => i.label === KEYS_NOT_FOR_CONTAINERS);
        expect(why?.enabled).toBe(false);
        m.dispose();
    });

    it("is not offered to another provider's agent, and a container with no keys has no note", async () => {
        const codex = mount({ accounts: [key("k", "Work key")], provider: "codex" });
        await flush();
        codex.onSwitchAccount(new MouseEvent("click"));
        expect(codex.showPicker.mock.calls.length === 0 || !(codex.showPicker.mock.calls[0][0] as ContextMenuItem[]).some((i) => i.sublabel === "API")).toBe(true);
        codex.dispose();

        const plain = mount({ accounts: [plan("p", "a@x")], links: [{ provider: "claude", account_id: "p" }], container: true });
        await flush();
        expect(items(plain).some((i) => i.label === KEYS_NOT_FOR_CONTAINERS)).toBe(false);
        plain.dispose();
    });
});
