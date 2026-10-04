// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The Armory's delete flow: one confirmation reached from the row menu and
// the detail panel, and a result the user always sees
// (SPEC_ARMORY_ACCOUNTS_DELETE_AND_INLINE_DETAIL_2026_10_04.md §3.1, §3.3, §3.5).

import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/mps", () => ({ muxEventSubscribe: vi.fn(() => () => {}) }));

const deleteMock = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        ListIdentityAccountsCommand: async () => [],
        DeleteIdentityAccountCommand: (_c: unknown, data: { id: string }) => deleteMock(data),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import type { BlockNodeModel } from "@/app/block/blocktypes";
import { deleteResultNotice, IdentityViewModel, type Account } from "./identity-model";

function mkAccount(over: Partial<Account> = {}): Account {
    return {
        id: "acct-1",
        name: "work-claude",
        provider: "claude" as Account["provider"],
        kind: "oauth",
        secret_ref: { backend: "oauth_config_dir", dir: "/tmp/acct-1" },
        context: { email: "me@example.com" },
        assigned_agents: [],
        status: "valid",
        created_at: "0",
        updated_at: "0",
        ...over,
    } as Account;
}

function modelWith(accounts: Account[]): IdentityViewModel {
    const model = new IdentityViewModel("test", {} as BlockNodeModel);
    (model as unknown as { setAccounts: (a: Account[]) => void }).setAccounts(accounts);
    return model;
}

describe("Armory delete flow", () => {
    beforeEach(() => {
        deleteMock.mockReset();
    });

    it("a stale menu on an already-deleted account opens no confirmation", () => {
        const model = modelWith([]);
        model.requestDelete(mkAccount());
        expect(model.pendingDeleteAtom()).toBeNull();
        expect(model.deleteNoticeAtom()).toBe("This account was already removed.");
    });

    it("confirming deletes the pending account and reports a cleanup failure", async () => {
        const account = mkAccount();
        const model = modelWith([account]);
        deleteMock.mockResolvedValue({
            deleted: true,
            affectedAgents: [],
            cleanup: { outcome: "failed", detail: "file in use", path: "/tmp/acct-1" },
        });

        model.requestDelete(account);
        expect(model.pendingDeleteAtom()?.id).toBe("acct-1");
        await model.confirmDelete();

        expect(deleteMock).toHaveBeenCalledWith({ id: "acct-1" });
        expect(model.pendingDeleteAtom()).toBeNull();
        expect(model.deleteNoticeAtom()).toContain("couldn't all be removed (file in use)");
    });

    it("an RPC error reaches the notice row, not the closed form", async () => {
        const account = mkAccount();
        const model = modelWith([account]);
        deleteMock.mockRejectedValue(new Error("store locked"));

        model.requestDelete(account);
        await model.confirmDelete();

        expect(model.pendingDeleteAtom()).toBeNull();
        expect(model.deleteNoticeAtom()).toBe("Couldn't delete me@example.com: store locked");
    });

    it("cancelling deletes nothing", () => {
        const account = mkAccount();
        const model = modelWith([account]);
        model.requestDelete(account);
        model.cancelDelete();
        expect(model.pendingDeleteAtom()).toBeNull();
        expect(deleteMock).not.toHaveBeenCalled();
    });
});

describe("deleteResultNotice", () => {
    it("says nothing when the row disappearing is feedback enough", () => {
        expect(deleteResultNotice({ deleted: true, affectedAgents: [], cleanup: { outcome: "removed" } })).toBeNull();
        expect(deleteResultNotice({ deleted: true, cleanup: { outcome: "absent" } })).toBeNull();
        expect(deleteResultNotice({ deleted: true, cleanup: null })).toBeNull();
    });

    it("reports an account another window already deleted", () => {
        expect(deleteResultNotice({ deleted: false })).toBe("This account was already removed.");
    });

    it("reports a login left outside AgentMux's folder", () => {
        const n = deleteResultNotice({ deleted: true, cleanup: { outcome: "skipped", path: "C:/Users/me/.claude" } });
        expect(n).toBe("Account deleted. Its login files are outside AgentMux's folder and were left in place: C:/Users/me/.claude.");
    });

    it("combines a cleanup failure with the affected-agent disclosure", () => {
        const n = deleteResultNotice({
            deleted: true,
            affectedAgents: ["a", "b"],
            cleanup: { outcome: "failed", detail: "denied" },
        });
        expect(n).toContain("couldn't all be removed (denied)");
        expect(n).toContain("2 agent(s) were using it");
        expect(n?.match(/Account deleted/g)?.length).toBe(1);
    });
});
