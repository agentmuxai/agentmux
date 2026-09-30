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

function mount(opts: { accounts: Account[]; links?: Array<{ provider: string; account_id: string }>; listFails?: boolean }) {
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
    };
    const root = createRoot((dispose) => {
        const [agentId, setAgentId] = createSignal<string | undefined>("agent-1");
        const b = useAccountBinding({
            agentDefinitionId: agentId,
            providerId: () => "claude",
            bindExistingAccount: bind,
            deps,
        });
        return { ...b, setAgentId, dispose };
    });
    return { ...root, bind, showPicker, push: (l: Account[]) => push?.(l) };
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
