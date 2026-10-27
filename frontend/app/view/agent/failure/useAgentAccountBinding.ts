// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { getBlockMetaKeyAtom } from "@/app/store/global";
import type { Accessor } from "solid-js";
import type { Account } from "@/app/view/identity/identity-model";
import { useAccountBinding } from "./useAccountBinding";

/**
 * `useAccountBinding` for an agent pane: the agent it acts for, and the switch queued for it
 * (`agent:pending_account`), are read from the pane's block meta.
 */
export function useAgentAccountBinding(opts: {
    blockId: string;
    providerId: Accessor<string | undefined>;
    bindExistingAccount: (account: Account) => Promise<void>;
    /** The agent is signed out, and the action its chip menu's "Log in" runs. */
    signedOut?: Accessor<boolean>;
    onLogin?: () => void;
}) {
    const meta = (key: "agentId" | "agent:pending_account") => () =>
        getBlockMetaKeyAtom(opts.blockId, key)() as string | undefined;
    return useAccountBinding({
        agentDefinitionId: meta("agentId"),
        providerId: opts.providerId,
        bindExistingAccount: opts.bindExistingAccount,
        pendingAccountId: meta("agent:pending_account"),
        signedOut: opts.signedOut,
        onLogin: opts.onLogin,
    });
}
