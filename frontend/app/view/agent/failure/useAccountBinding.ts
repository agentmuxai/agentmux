// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Split out of agent-view.tsx (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §3.5 step 9).

import { ContextMenuModel } from "@/app/store/contextmenu";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import {
    boundAccountEmail,
    loadAccounts,
    subscribeAccountChanges,
    type Account,
} from "@/app/view/identity/identity-model";
import { createEffect, createMemo, createSignal, on, onCleanup, type Accessor } from "solid-js";
import { lastLinkedAccountId } from "../providers/provider-id-aliases";
import { accountPickerItems, planBind, type BindMode } from "./account-picker";
import { computeAccountBindCandidates } from "./bind-account-candidates";

/** Where the hook reads accounts and links from, and how it shows its picker; real ones by default. */
export interface AccountBindingDeps {
    loadAccounts: () => Account[];
    subscribeAccountChanges: (cb: (list: Account[]) => void) => () => void;
    listIdentities: (agentId: string) => Promise<Array<{ provider: string; account_id: string }>>;
    showPicker: (items: ReturnType<typeof accountPickerItems>, e: MouseEvent) => void;
}

const defaultDeps: AccountBindingDeps = {
    loadAccounts,
    subscribeAccountChanges,
    listIdentities: (agentId) => RpcApi.ListAgentIdentitiesCommand(TabRpcClient, { agent_id: agentId }),
    showPicker: (items, e) => ContextMenuModel.showContextMenu(items, e),
};

export interface AccountBinding {
    /** The bound account's login email, for the composer's sign-in chip. */
    authEmail: Accessor<string | undefined>;
    /** Accounts this agent could bind to: valid OAuth accounts for its provider, not the linked one. */
    bindCandidates: Accessor<Account[]>;
    /** The failure row's "adopt": binds a lone candidate at once, else opens the picker. */
    onBindAccount: (e?: MouseEvent) => void;
    /** The composer chip's "switch": always a named choice from the picker. */
    onSwitchAccount: (e: MouseEvent) => void;
    /** Re-read the linked account (the identity links changed). */
    refreshLinkedAccountId: () => Promise<void>;
}

// ---- Bind-account (SPEC_AGENT_LOGIN_FLOW_TIGHTENING_2026_09_04.md §2/§3) ----
export function useAccountBinding(opts: {
    agentDefinitionId: Accessor<string | undefined>;
    providerId: Accessor<string | undefined>;
    bindExistingAccount: (account: Account) => Promise<void>;
    deps?: Partial<AccountBindingDeps>;
}): AccountBinding {
    const deps = { ...defaultDeps, ...opts.deps };

    // Cross-pane/cross-tab live account cache — same subscription pattern
    // `AgentLaunchModal.tsx` already uses. Seeded synchronously (no network
    // round trip; the app-wide cache is already warm) then kept live.
    const [accountCache, setAccountCache] = createSignal<Account[]>(deps.loadAccounts());
    createEffect(() => {
        const unsub = deps.subscribeAccountChanges((list) => setAccountCache(list));
        onCleanup(unsub);
    });

    // This agent's own currently-linked account for its provider, if any —
    // excluded from bind candidates (nothing to adopt). Refreshed on mount
    // and whenever this agent's identity links change — the same event that
    // also drives the auto-unblock check, since both concerns become stale
    // for the identical reason.
    const [linkedAccountId, setLinkedAccountId] = createSignal<string | undefined>(undefined);
    const refreshLinkedAccountId = async () => {
        const agentDefinitionId = opts.agentDefinitionId();
        const providerId = opts.providerId();
        if (!agentDefinitionId || !providerId) {
            setLinkedAccountId(undefined);
            return;
        }
        try {
            const links = await deps.listIdentities(agentDefinitionId);
            setLinkedAccountId(lastLinkedAccountId(links, providerId));
        } catch {
            setLinkedAccountId(undefined);
        }
    };
    // Re-resolve whenever the agent id or provider resolves or changes — both
    // come from block meta, which may not have loaded on the first run, and a
    // one-shot mount-time call left the link (and the chip's email) unset.
    createEffect(
        on(
            () => [opts.agentDefinitionId(), opts.providerId()] as const,
            () => void refreshLinkedAccountId(),
        ),
    );

    // The bound account's login email for the composer's sign-in chip
    // (SPEC_ACCOUNT_EMAIL_IN_ARMORY_2026_09_23.md) — live across logins and
    // rebinds via accountCache + linkedAccountId.
    const authEmail = createMemo(() => boundAccountEmail(accountCache(), linkedAccountId()));

    const bindCandidates = createMemo(() => {
        const providerId = opts.providerId();
        if (!providerId) return [];
        return computeAccountBindCandidates(providerId, accountCache(), linkedAccountId());
    });

    // A flat picker of accounts to bind, anchored at the click — same
    // ContextMenuModel primitive the Armory's Bind-to-Agent menu uses
    // (SPEC_ARMORY_BIND_TO_AGENT_CONTEXT_MENU_2026_08_09.md), just a flat
    // list here (the trigger IS the button — no outer submenu to nest under).
    // What to do for a given candidate list is `planBind` (account-picker.ts,
    // unit-tested): the failure row's "adopt" binds a lone candidate at once; the
    // composer chip's "switch" never does — the agent works and a switch restarts
    // it, so the user picks a named destination
    // (SPEC_COMPOSER_ACCOUNT_SWITCH_AND_JEKT_HEIGHT_CAP_2026_09_26.md Part A).
    const runBind = (mode: BindMode, e: MouseEvent | undefined, labelPrefix = "") => {
        const plan = planBind(mode, bindCandidates());
        if (plan.kind === "none") return;
        if (plan.kind === "bind") {
            void opts.bindExistingAccount(plan.account);
            return;
        }
        // No event to anchor on (shouldn't happen — PaneRow's render call site
        // always passes one) → no-op rather than guessing a position.
        if (!e) return;
        deps.showPicker(
            accountPickerItems(plan.candidates, (acct) => void opts.bindExistingAccount(acct), labelPrefix),
            e,
        );
    };
    const onBindAccount = (e?: MouseEvent) => runBind("adopt", e);
    const onSwitchAccount = (e: MouseEvent) => runBind("switch", e, "Switch to ");

    return { authEmail, bindCandidates, onBindAccount, onSwitchAccount, refreshLinkedAccountId };
}
