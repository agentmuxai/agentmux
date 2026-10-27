// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Split out of agent-view.tsx (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §3.5 step 9).

import { useAccountEpisodes, episodeReason } from "@/app/store/account-availability";
import { ContextMenuModel } from "@/app/store/contextmenu";
import { pushNotification } from "@/app/store/flash-notifications";
import { openModal } from "@/app/store/modalmodel";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import {
    accountLabel,
    boundAccountEmail,
    loadAccounts,
    subscribeAccountChanges,
    type Account,
} from "@/app/view/identity/identity-model";
import { notifyLinksChanged } from "@/app/view/identity/account-agents";
import { createEffect, createMemo, createSignal, on, onCleanup, type Accessor } from "solid-js";
import { resolveProviderAlias } from "../providers";
import { lastLinkedAccountId } from "../providers/provider-id-aliases";
import { accountPickerItems, planBind, type BindMode } from "./account-picker";
import { AnotherAccountLogin } from "./AnotherAccountLogin";
import { computeAccountBindCandidates } from "./bind-account-candidates";

/** Where the hook reads accounts and links from, and how it shows its picker; real ones by default. */
export interface AccountBindingDeps {
    loadAccounts: () => Account[];
    subscribeAccountChanges: (cb: (list: Account[]) => void) => () => void;
    listIdentities: (agentId: string) => Promise<Array<{ provider: string; account_id: string }>>;
    showPicker: (items: ContextMenuItem[], e: MouseEvent) => void;
    /** `agent.account.switch`: srv links the account and restarts the agent's panes, now or after its turn. */
    switchAccount: (agentId: string, accountId: string) => Promise<{ status: string; account_label: string; failed: string[] }>;
    /** What the signed-in-accounts menu says about an account's quota, if anything ("out of quota until …"). */
    quotaNote: (accountId: string) => string | undefined;
    notify: (type: "info" | "warning" | "error", title: string, message?: string) => void;
    /** Run a fresh sign-in that adds an account; `onSignedIn` gets the new account's id. */
    signInAnother: (onSignedIn: (accountId: string) => void) => void;
}

const defaultDeps: AccountBindingDeps = {
    loadAccounts,
    subscribeAccountChanges,
    listIdentities: (agentId) => RpcApi.ListAgentIdentitiesCommand(TabRpcClient, { agent_id: agentId }),
    showPicker: (items, e) => ContextMenuModel.showContextMenu(items, e),
    switchAccount: (agentId, accountId) =>
        RpcApi.AgentAccountSwitchCommand(TabRpcClient, { agent_id: agentId, account: accountId, when: "after_turn" }),
    quotaNote: () => undefined,
    signInAnother: (onSignedIn) => void openModal(AnotherAccountLogin, { onSignedIn }),
    notify: (type, title, message = "") =>
        pushNotification({
            icon: type === "info" ? "fa-check" : "fa-triangle-exclamation",
            title,
            message,
            timestamp: new Date().toISOString(),
            type,
            expiration: Date.now() + 8000,
        }),
};

export interface AccountBinding {
    /** The bound account's login email, for the composer's sign-in chip. */
    authEmail: Accessor<string | undefined>;
    /** Accounts this agent could bind to: valid OAuth accounts for its provider, not the linked one. */
    bindCandidates: Accessor<Account[]>;
    /** The failure row's "adopt": binds a lone candidate at once, else opens the picker. */
    onBindAccount: (e?: MouseEvent) => void;
    /** The composer chip's menu: every signed-in account of the provider, the current one checked, then
     *  "Log in" (when signed out) and "Sign in with another account…". */
    onSwitchAccount: (e: MouseEvent) => void;
    /** Whether the chip has a menu: another account, "Log in" or "Sign in with another account…". */
    canSwitchAccount: Accessor<boolean>;
    /** The account a queued switch is waiting to move to (after the current turn), as the chip names it. */
    pendingAccountLabel: Accessor<string | undefined>;
    /** Re-read the linked account (the identity links changed). */
    refreshLinkedAccountId: () => Promise<void>;
}

// ---- Bind-account (SPEC_AGENT_LOGIN_FLOW_TIGHTENING_2026_09_04.md §2/§3) ----
export function useAccountBinding(opts: {
    agentDefinitionId: Accessor<string | undefined>;
    providerId: Accessor<string | undefined>;
    bindExistingAccount: (account: Account) => Promise<void>;
    /** `agent:pending_account` of this pane: the account id a queued switch moves to. */
    pendingAccountId?: Accessor<string | undefined>;
    /** The agent is signed out: its chip menu offers "Log in" (`onLogin`) next to the accounts. */
    signedOut?: Accessor<boolean>;
    onLogin?: () => void;
    deps?: Partial<AccountBindingDeps>;
}): AccountBinding {
    const deps = { ...defaultDeps, ...opts.deps };
    // Quota notes come from srv's account episodes, the same source as the Swarm strip.
    const episodes = opts.deps?.quotaNote ? undefined : useAccountEpisodes();
    const quotaNote = (accountId: string): string | undefined => {
        if (!episodes) return deps.quotaNote(accountId);
        const ep = episodes().find((e) => e.account_id === accountId);
        return ep ? episodeReason(ep) : undefined;
    };

    // Cross-pane/cross-tab live account cache. Seeded synchronously (no network
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
    // ContextMenuModel primitive Connectors → Accounts' Bind-to-Agent menu uses
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

    // The composer chip (SPEC_ACCOUNT_SWITCH_FROM_THE_COMPOSER_NO_BIND_2026_10_26.md §3.2): every
    // signed-in account of the agent's provider, the current one checked, with its quota state.
    // srv does the switch (`agent.account.switch`): at once when the agent is idle, else when its
    // turn ends.
    const switchChoices = createMemo(() => {
        const providerId = opts.providerId();
        if (!providerId) return [];
        const provider = resolveProviderAlias(providerId);
        return accountCache()
            .filter((a) => resolveProviderAlias(a.provider) === provider && a.status === "valid")
            .sort((a, b) => b.updated_at.localeCompare(a.updated_at));
    });
    // Signing in a new account is the in-app Claude login; other providers have no such panel yet.
    const canSignInAnother = createMemo(() => resolveProviderAlias(opts.providerId() ?? "") === "claude");
    const canLogIn = () => !!opts.signedOut?.() && !!opts.onLogin;
    const canSwitchAccount = createMemo(
        () => switchChoices().some((a) => a.id !== linkedAccountId()) || canSignInAnother() || canLogIn(),
    );
    const pendingAccountLabel = createMemo(() => {
        const id = opts.pendingAccountId?.();
        const a = id ? accountCache().find((x) => x.id === id) : undefined;
        return a ? accountLabel(a, accountCache()) : undefined;
    });
    const switchTo = async (account: Account) => {
        // Picking the account it is on cancels a queued switch; with none queued there is nothing to do.
        if (account.id === linkedAccountId() && !opts.pendingAccountId?.()) return;
        await switchToId(account.id, accountLabel(account, accountCache()));
    };
    const switchToId = async (accountId: string, label: string) => {
        const agentId = opts.agentDefinitionId();
        if (!agentId) return;
        try {
            const r = await deps.switchAccount(agentId, accountId);
            // Connectors' account cards and open detail list agents from the links.
            notifyLinksChanged();
            if (r.status === "unchanged") {
                deps.notify("info", `Staying on ${label}`, "The queued switch was cancelled.");
            } else if (r.status === "queued") {
                deps.notify("info", `Switching to ${label} after this turn`, "The agent keeps working; it moves when the turn ends.");
            } else if (r.failed.length > 0) {
                deps.notify("warning", `Switched to ${label}, with a problem`, r.failed.join(" "));
            }
            await refreshLinkedAccountId();
        } catch (e: any) {
            deps.notify("error", `Couldn't switch to ${label}`, String(e?.message ?? e));
        }
    };
    // A sign-in that just added an account: the agent moves to it (now, or after its turn).
    const switchToNewAccount = (accountId: string) => {
        const all = deps.loadAccounts();
        const account = accountCache().find((a) => a.id === accountId) ?? all.find((a) => a.id === accountId);
        void switchToId(accountId, account ? accountLabel(account, all) : "the new account");
    };
    const onSwitchAccount = (e: MouseEvent) => {
        const current = linkedAccountId();
        const items: ContextMenuItem[] = switchChoices().map((a) => ({
            label: accountLabel(a, accountCache()),
            sublabel: quotaNote(a.id),
            type: "checkbox" as const,
            checked: a.id === current,
            click: () => void switchTo(a),
        }));
        const actions: ContextMenuItem[] = [];
        if (canLogIn()) actions.push({ label: "Log in", click: () => opts.onLogin?.() });
        if (canSignInAnother()) {
            actions.push({ label: "Sign in with another account\u2026", click: () => deps.signInAnother(switchToNewAccount) });
        }
        if (items.length > 0 && actions.length > 0) items.push({ type: "separator" });
        items.push(...actions);
        if (items.length === 0) return;
        deps.showPicker(items, e);
    };

    return {
        authEmail,
        bindCandidates,
        onBindAccount,
        onSwitchAccount,
        canSwitchAccount,
        pendingAccountLabel,
        refreshLinkedAccountId,
    };
}
