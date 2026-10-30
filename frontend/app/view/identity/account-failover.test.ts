// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";

const rpc = vi.hoisted(() => ({ IdentityAccountSetFailoverCommand: vi.fn() }));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: rpc }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
const refresh = vi.hoisted(() => vi.fn());
vi.mock("./identity-model", () => ({ refreshAccountCache: refresh }));

import { failoverToggle } from "./account-failover";
import type { Account } from "./identity-model";

const account = (provider: string) => ({ id: "acct-1", provider }) as unknown as Account;

describe("failoverToggle", () => {
    beforeEach(() => {
        rpc.IdentityAccountSetFailoverCommand.mockReset().mockResolvedValue({});
        refresh.mockReset();
    });

    it("asks srv for the mark and re-reads the accounts", async () => {
        failoverToggle(account("claude"))!(false);
        expect(rpc.IdentityAccountSetFailoverCommand).toHaveBeenCalledWith({}, { account_id: "acct-1", allow: false });
        await vi.waitFor(() => expect(refresh).toHaveBeenCalledTimes(1));
    });

    it("is offered only for an account of a provider agents run on", () => {
        expect(failoverToggle(account("claude"))).toBeTypeOf("function");
        expect(failoverToggle(account("github"))).toBeUndefined();
    });
});
