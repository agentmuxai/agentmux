// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// "Fail over to this account": the operator's mark on an account (`identity.account.set_failover`,
// srv's `account_episodes::failover`). An agent that hits its usage limit is never moved onto a
// marked account by itself.

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { PROVIDERS, resolveProviderAlias } from "@/app/view/agent/providers/catalog";
import { refreshAccountCache, type Account } from "./identity-model";

/** What the row menu's "Fail over to this account" item calls, or nothing for an account no agent runs on. */
export function failoverToggle(account: Account): ((allow: boolean) => void) | undefined {
    if (!PROVIDERS[resolveProviderAlias(account.provider)]) return undefined;
    return (allow) => {
        void RpcApi.IdentityAccountSetFailoverCommand(TabRpcClient, { account_id: account.id, allow }).then(() => refreshAccountCache());
    };
}
