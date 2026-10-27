// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { Account } from "@/app/view/identity/identity-model";
import { createRoot, createSignal } from "solid-js";
import { describe, expect, it, vi } from "vitest";
import { useAccountBinding, type AccountBindingDeps } from "./useAccountBinding";

const oauth = (id: string, email: string, updated = "2026-09-01"): Account =>
    ({
        id,
        name: id,
        provider: "claude",
        kind: "oauth",
        secret_ref: { backend: "oauth_config_dir" },
        context: { email },
        status: "valid",
        updated_at: updated,
    }) as unknown as Account;

const flush = () => new Promise((r) => setTimeout(r, 0));

function mount(opts: {
    accounts: Account[];
    links?: Array<{ provider: string; account_id: string }>;
    listFails?: boolean;
    pending?: string;
    switchResult?: { status: string; account_label: string; failed: string[] };
    quota?: Record<string, string>;
}) {
    const switchAccount = vi.fn(async () => opts.switchResult ?? { status: "switched", account_label: "", failed: [] });
    const notify = vi.fn();
    const bind = vi.fn<(account: Account) => Promise<void>>(async () => {});
    const showPicker = vi.fn();
    let push: ((list: Account[]) => void) | undefined;
    const deps: AccountBindingDeps = {
        loadAccounts: () => opts.accounts,
        subscribeAccountChanges: (cb) => {
            push = cb;
            return () => {};
        },
        listIdentities: async () => {
            if (opts.listFails) throw new Error("down");
            return opts.links ?? [];
        },
        showPicker,
        switchAccount,
        quotaNote: (id) => opts.quota?.[id],
        notify,
    };
    const root = createRoot((dispose) => {
        const [agentId, setAgentId] = createSignal<string | undefined>("agent-1");
        const b = useAccountBinding({
            agentDefinitionId: agentId,
            providerId: () => "claude",
            bindExistingAccount: bind,
            pendingAccountId: () => opts.pending,
            deps,
        });
        return { ...b, setAgentId, dispose };
    });
    return { ...root, bind, showPicker, switchAccount, notify, push: (l: Account[]) => push?.(l) };
}

describe("useAccountBinding", () => {
    it("offers the other valid OAuth accounts, never the one already linked", async () => {
        const m = mount({
            accounts: [oauth("acc-a", "a@x"), oauth("acc-b", "b@x")],
            links: [{ provider: "claude", account_id: "acc-a" }],
        });
        await flush();
        expect(m.bindCandidates().map((a) => a.id)).toEqual(["acc-b"]);
        expect(m.authEmail()).toBe("a@x");
        m.dispose();
    });

    it("reads a failed identity lookup as no linked account", async () => {
        const m = mount({ accounts: [oauth("acc-a", "a@x")], listFails: true });
        await flush();
        expect(m.authEmail()).toBeUndefined();
        expect(m.bindCandidates().map((a) => a.id)).toEqual(["acc-a"]);
        m.dispose();
    });

    it("adopt with a single candidate binds it at once", async () => {
        const m = mount({ accounts: [oauth("acc-a", "a@x")] });
        await flush();
        m.onBindAccount();
        expect(m.bind).toHaveBeenCalledWith(expect.objectContaining({ id: "acc-a" }));
        expect(m.showPicker).not.toHaveBeenCalled();
        m.dispose();
    });

    it("switch never binds directly: it opens the picker at the click", async () => {
        const m = mount({ accounts: [oauth("acc-a", "a@x")] });
        await flush();
        const ev = new MouseEvent("click");
        m.onSwitchAccount(ev);
        expect(m.bind).not.toHaveBeenCalled();
        expect(m.showPicker).toHaveBeenCalledTimes(1);
        expect(m.showPicker.mock.calls[0][1]).toBe(ev);
        m.dispose();
    });

    it("the switch menu lists every signed-in account of the provider, the current one checked, with quota notes", async () => {
        const m = mount({
            accounts: [oauth("acc-a", "a@x", "2026-09-02"), oauth("acc-b", "b@x"), { ...oauth("acc-c", "c@x"), status: "expired" } as Account],
            links: [{ provider: "claude", account_id: "acc-a" }],
            quota: { "acc-b": "out of quota until Sun 7:00 PM" },
        });
        await flush();
        expect(m.canSwitchAccount()).toBe(true);
        m.onSwitchAccount(new MouseEvent("click"));
        const items = m.showPicker.mock.calls[0][0] as ContextMenuItem[];
        expect(items.map((i) => [i.label, i.checked, i.sublabel])).toEqual([
            ["a@x", true, undefined],
            ["b@x", false, "out of quota until Sun 7:00 PM"],
        ]);
        m.dispose();
    });

    it("a lone linked account has nothing to switch to", async () => {
        const m = mount({ accounts: [oauth("acc-a", "a@x")], links: [{ provider: "claude", account_id: "acc-a" }] });
        await flush();
        expect(m.canSwitchAccount()).toBe(false);
        m.dispose();
    });

    it("picking an account asks srv to switch and says so when it waits for the turn", async () => {
        const m = mount({
            accounts: [oauth("acc-a", "a@x"), oauth("acc-b", "b@x")],
            links: [{ provider: "claude", account_id: "acc-a" }],
            switchResult: { status: "queued", account_label: "b@x", failed: [] },
        });
        await flush();
        m.onSwitchAccount(new MouseEvent("click"));
        const items = m.showPicker.mock.calls[0][0] as ContextMenuItem[];
        items.find((i) => i.label === "b@x")!.click!();
        await flush();
        expect(m.switchAccount).toHaveBeenCalledWith("agent-1", "acc-b");
        expect(m.notify).toHaveBeenCalledWith("info", "Switching to b@x after this turn", expect.any(String));
        // The account it is on is not a switch.
        items.find((i) => i.label === "a@x")!.click!();
        await flush();
        expect(m.switchAccount).toHaveBeenCalledTimes(1);
        m.dispose();
    });

    it("picking the account it is on, while a switch waits, cancels the switch", async () => {
        const m = mount({
            accounts: [oauth("acc-a", "a@x"), oauth("acc-b", "b@x")],
            links: [{ provider: "claude", account_id: "acc-a" }],
            pending: "acc-b",
            switchResult: { status: "unchanged", account_label: "a@x", failed: [] },
        });
        await flush();
        m.onSwitchAccount(new MouseEvent("click"));
        const items = m.showPicker.mock.calls[0][0] as ContextMenuItem[];
        items.find((i) => i.label === "a@x")!.click!();
        await flush();
        expect(m.switchAccount).toHaveBeenCalledWith("agent-1", "acc-a");
        expect(m.notify).toHaveBeenCalledWith("info", "Staying on a@x", expect.any(String));
        m.dispose();
    });

    it("names same-email accounts with their numbers, in the chip and the menu", async () => {
        const numbered = (id: string, seq: number): Account => ({ ...oauth(id, "me@x"), context: { email: "me@x", seq } }) as Account;
        const m = mount({
            accounts: [numbered("acc-1", 1), numbered("acc-2", 2)],
            links: [{ provider: "claude", account_id: "acc-2" }],
            pending: "acc-1",
        });
        await flush();
        expect(m.authEmail()).toBe("me@x \u00b72");
        expect(m.pendingAccountLabel()).toBe("me@x \u00b71");
        m.onSwitchAccount(new MouseEvent("click"));
        const items = m.showPicker.mock.calls[0][0] as ContextMenuItem[];
        expect(items.map((i) => i.label).sort()).toEqual(["me@x \u00b71", "me@x \u00b72"]);
        m.dispose();
    });

    it("follows live account changes", async () => {
        const m = mount({ accounts: [] });
        await flush();
        expect(m.bindCandidates()).toEqual([]);
        m.push([oauth("acc-new", "n@x")]);
        expect(m.bindCandidates().map((a) => a.id)).toEqual(["acc-new"]);
        m.dispose();
    });

    it("re-resolves the linked account when the agent id changes", async () => {
        const m = mount({ accounts: [oauth("acc-a", "a@x")], links: [{ provider: "claude", account_id: "acc-a" }] });
        await flush();
        expect(m.authEmail()).toBe("a@x");
        m.setAgentId(undefined);
        await flush();
        expect(m.authEmail()).toBeUndefined();
        m.dispose();
    });
});
