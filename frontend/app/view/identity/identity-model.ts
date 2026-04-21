// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// NOTE: Identity is NOT a standalone pane. It is embedded inside the agent
// pane as a tab in the floating settings panel (AgentCardSettingsPanel →
// AgentIdentityPanel). The standalone identity widget was removed in
// v0.33.197. Do not re-register this as a block view — account management
// lives inside the agent pane's settings overlay.

import { BlockNodeModel } from "@/app/block/blocktypes";
import { createSignal, type Accessor, type Setter } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { Logger } from "@/util/logger";

// ── Types ────────────────────────────────────────────────────────────────────

export type AccountProvider = "github" | "aws" | "anthropic" | "custom";
export type AccountKind = "pat" | "role" | "api_key" | "env_ref";
export type AccountStatus = "valid" | "expired" | "invalid" | "unknown" | "checking";
export type IdentityTab = "accounts" | "assignments";

export interface SecretRef {
    backend: "env" | "secrets_manager" | "plaintext_dev";
    env_var?: string;
    sm_path?: string;
    sm_json_path?: string;
    value?: string; // plaintext_dev only
}

export interface AccountContext {
    github_username?: string;
    github_scopes?: string[];
    aws_profile?: string;
    aws_role_arn?: string;
    aws_region?: string;
    anthropic_model?: string;
    endpoint?: string;
    description?: string;
}

export interface Account {
    id: string;
    name: string;
    provider: AccountProvider;
    kind: AccountKind;
    display_name?: string;
    secret_ref: SecretRef;
    context: AccountContext;
    /**
     * @deprecated Derive from the agent-side reverse index instead.
     * Kept for backwards compatibility. Do not write new code that
     * reads this field — use parseAgentAccounts(agent) on ForgeAgent.
     * Scheduled for removal after SPEC_AGENT_IDENTITY_RESTRUCTURE Step 4
     * is fully rolled out.
     */
    assigned_agents: string[];
    status: AccountStatus;
    created_at: string;
    updated_at: string;
}

/**
 * Per-provider account references stored on a ForgeAgent.
 * A null value means no account is assigned for that provider.
 */
export type AgentAccounts = Partial<Record<AccountProvider, string | null>>;

/** Parse the JSON-encoded accounts blob from a ForgeAgent. */
export function parseAgentAccounts(agent: ForgeAgent): AgentAccounts {
    if (!agent.accounts) return {};
    try {
        return JSON.parse(agent.accounts) as AgentAccounts;
    } catch {
        return {};
    }
}

/** Serialize AgentAccounts back to the JSON blob stored on ForgeAgent. */
export function serializeAgentAccounts(accounts: AgentAccounts): string {
    return JSON.stringify(accounts);
}

/**
 * Return the IDs of agents that reference this account (reverse index).
 * Used by the global Identity panel to show "assigned agents" without
 * reading the deprecated Account.assigned_agents field.
 */
export function agentsAssignedToAccount(accountId: string, agents: ForgeAgent[]): string[] {
    return agents
        .filter((a) => {
            const accs = parseAgentAccounts(a);
            return Object.values(accs).includes(accountId);
        })
        .map((a) => a.name);
}

export const PROVIDER_LABELS: Record<AccountProvider, string> = {
    github: "GitHub",
    aws: "AWS",
    anthropic: "Anthropic",
    custom: "Custom",
};

export const PROVIDER_COLORS: Record<AccountProvider, string> = {
    github: "#e1effe",
    aws: "#fef3c7",
    anthropic: "#ede9fe",
    custom: "#f1f5f9",
};

export const KIND_LABELS: Record<AccountKind, string> = {
    pat: "Personal Access Token",
    role: "IAM Role",
    api_key: "API Key",
    env_ref: "Environment Variable",
};

// ── Storage (DB-backed via RPC, in-memory cache) ─────────────────────────────
//
// As of v0.33.30x (PR #479 + this PR) accounts live in the SQLite
// `db_identity_accounts` table on the sidecar, not in localStorage. This
// module keeps an in-memory cache so synchronous callers (e.g. agent
// startup payload assembly) can stay synchronous.
//
// Lifecycle:
// 1. `primeAccountCache()` runs at app startup and populates the cache
//    from the DB via `ListIdentityAccountsCommand`.
// 2. CRUD methods on `IdentityViewModel` go through RPCs and refresh the
//    cache afterwards.
// 3. `loadAccounts()` is a synchronous getter that returns whatever's
//    currently in the cache. First call after process start returns
//    `[]` if priming hasn't completed yet — callers that need
//    correctness over latency can `await refreshAccountCache()` first.

let _accountCache: Account[] = [];
const _cacheChangeListeners: Array<(accounts: Account[]) => void> = [];

/** Returns the current cache snapshot (synchronous). */
export function loadAccounts(): Account[] {
    return _accountCache;
}

/** Subscribe to cache updates. Returns an unsubscribe function. */
export function subscribeAccountChanges(fn: (accounts: Account[]) => void): () => void {
    _cacheChangeListeners.push(fn);
    return () => {
        const idx = _cacheChangeListeners.indexOf(fn);
        if (idx >= 0) _cacheChangeListeners.splice(idx, 1);
    };
}

function setCache(accounts: Account[]): void {
    _accountCache = accounts;
    for (const fn of _cacheChangeListeners) {
        try {
            fn(_accountCache);
        } catch {
            // listener errors must not break other listeners
        }
    }
}

/**
 * Translate the backend's `SecretRef` (discriminated union, `plaintext_dev`
 * field name) to the frontend's loose-shape `SecretRef` (`value` field for
 * the dev plaintext case). Reagent caught a real bug here in PR #480
 * review: a naked `as unknown as SecretRef` cast hid the field-name
 * mismatch, so editing a plaintext_dev account loaded from DB rendered
 * with an empty secret (read `value`, got `undefined`) and a save would
 * have round-tripped the wrong key back.
 */
function secretRefFromBackend(s: IdentityAccount["secret_ref"]): SecretRef {
    switch (s.backend) {
        case "env":
            return { backend: "env", env_var: s.env_var };
        case "secrets_manager":
            return {
                backend: "secrets_manager",
                sm_path: s.sm_path,
                sm_json_path: s.sm_json_path,
            };
        case "plaintext_dev":
            return { backend: "plaintext_dev", value: s.plaintext_dev };
    }
}

/** Reverse of `secretRefFromBackend`. Defaults plaintext_dev to "" if the
 * caller hasn't filled it in (the form uses controlled inputs and won't
 * leave it undefined in practice, but the cast through the loose local
 * type means TS can't enforce that for us). */
function secretRefToBackend(s: SecretRef): IdentityAccount["secret_ref"] {
    switch (s.backend) {
        case "env":
            return { backend: "env", env_var: s.env_var ?? "" };
        case "secrets_manager":
            return {
                backend: "secrets_manager",
                sm_path: s.sm_path ?? "",
                sm_json_path: s.sm_json_path,
            };
        case "plaintext_dev":
            return { backend: "plaintext_dev", plaintext_dev: s.value ?? "" };
    }
}

/**
 * Convert a backend `IdentityAccount` (snake_case, JSON context blob) to
 * the frontend `Account` shape. Most fields map 1:1; `secret_ref` goes
 * through `secretRefFromBackend` to bridge the field-name mismatch
 * (`plaintext_dev` ↔ `value`). `assigned_agents` is synthesized empty —
 * it's deprecated and consumers should use the agent-side reverse index
 * via `agentsAssignedToAccount`.
 */
function backendToAccount(a: IdentityAccount): Account {
    const ts = (n: number) => new Date(n).toISOString();
    return {
        id: a.id,
        name: a.name,
        provider: a.provider as AccountProvider,
        kind: a.kind as AccountKind,
        display_name: a.display_name,
        secret_ref: secretRefFromBackend(a.secret_ref),
        context: (a.context as AccountContext) ?? {},
        assigned_agents: [],
        status: (a.status as AccountStatus) ?? "unknown",
        created_at: ts(a.created_at),
        updated_at: ts(a.updated_at),
    };
}

/** Convert frontend `Account` to backend payload. */
function accountToBackend(a: Account): Partial<IdentityAccount> {
    const t = (s: string) => {
        const n = Date.parse(s);
        return Number.isFinite(n) ? n : 0;
    };
    return {
        id: a.id,
        name: a.name,
        provider: a.provider,
        kind: a.kind,
        display_name: a.display_name,
        secret_ref: secretRefToBackend(a.secret_ref),
        context: a.context as Record<string, unknown>,
        status: a.status,
        created_at: t(a.created_at),
        updated_at: t(a.updated_at),
    };
}

/** Pull the latest account list from the DB and update the cache. */
export async function refreshAccountCache(): Promise<Account[]> {
    try {
        const rows = await RpcApi.ListIdentityAccountsCommand(TabRpcClient, {});
        const mapped = rows.map(backendToAccount);
        setCache(mapped);
        return mapped;
    } catch (err) {
        Logger.warn("identity", `refreshAccountCache failed: ${(err as Error)?.message ?? err}`);
        return _accountCache;
    }
}

/** Run once at app startup. Idempotent — repeated calls just refresh. */
export function primeAccountCache(): void {
    void refreshAccountCache();
}

// ── ViewModel ────────────────────────────────────────────────────────────────

export class IdentityViewModel implements ViewModel {
    viewType = "identity";
    blockId: string;
    nodeModel: BlockNodeModel;

    viewIcon: Accessor<string> = () => "id-card";
    viewName: Accessor<string> = () => "Identity";
    viewText: Accessor<string | HeaderElem[]> = () => [];
    noPadding: Accessor<boolean> = () => false;

    get viewComponent(): ViewComponent {
        return null; // set by barrel to avoid circular import
    }

    // Tab state
    private _tab = createSignal<IdentityTab>("accounts");
    tabAtom: Accessor<IdentityTab> = this._tab[0];
    setTab: Setter<IdentityTab> = this._tab[1];

    // Accounts list
    private _accounts = createSignal<Account[]>([]);
    accountsAtom: Accessor<Account[]> = this._accounts[0];
    private setAccounts: Setter<Account[]> = this._accounts[1];

    // Selected account for detail panel
    private _selectedAccount = createSignal<Account | null>(null);
    selectedAccountAtom: Accessor<Account | null> = this._selectedAccount[0];
    setSelectedAccount: Setter<Account | null> = this._selectedAccount[1];

    // Add/edit form state
    private _formOpen = createSignal<boolean>(false);
    formOpenAtom: Accessor<boolean> = this._formOpen[0];
    private setFormOpen: Setter<boolean> = this._formOpen[1];

    private _editingAccount = createSignal<Account | null>(null);
    editingAccountAtom: Accessor<Account | null> = this._editingAccount[0];
    private setEditingAccount: Setter<Account | null> = this._editingAccount[1];

    private _formError = createSignal<string | null>(null);
    formErrorAtom: Accessor<string | null> = this._formError[0];
    private setFormError: Setter<string | null> = this._formError[1];

    constructor(blockId: string, nodeModel: BlockNodeModel) {
        this.blockId = blockId;
        this.nodeModel = nodeModel;
        // Initial paint from cache (may be empty on first launch).
        this.setAccounts(loadAccounts());
        // Stay in sync with module-level cache updates from any source
        // (other panes, RPC events, etc.).
        const unsub = subscribeAccountChanges((accounts) => {
            this.setAccounts(accounts);
            // Keep the selected account fresh if it was edited externally.
            const sel = this.selectedAccountAtom();
            if (sel) {
                const updated = accounts.find((a) => a.id === sel.id);
                if (updated && updated !== sel) this.setSelectedAccount(updated);
                if (!updated) this.setSelectedAccount(null);
            }
        });
        // Force a refresh in case the cache hasn't been primed yet.
        void refreshAccountCache();
        // No explicit dispose — subscription leaks per ViewModel are
        // bounded by the number of identity panes ever opened in a
        // session (small). If this becomes a memory concern, wire
        // `unsub` to ViewModel teardown (no such hook exists today).
        void unsub;
    }

    // ── Derived helpers ──────────────────────────────────────────────────────

    accountsByProvider = (): Map<AccountProvider, Account[]> => {
        const map = new Map<AccountProvider, Account[]>();
        const order: AccountProvider[] = ["github", "aws", "anthropic", "custom"];
        for (const p of order) {
            const group = this.accountsAtom().filter((a) => a.provider === p);
            if (group.length > 0) map.set(p, group);
        }
        return map;
    };

    // ── CRUD ────────────────────────────────────────────────────────────────

    createAccount = async (data: Omit<Account, "id" | "status" | "created_at" | "updated_at">): Promise<void> => {
        const now = new Date().toISOString();
        // Build a frontend-shape Account, then ship to backend. Backend mints
        // the canonical id + timestamps; we accept whatever it returns.
        const draft: Account = {
            ...data,
            id: "", // backend mints
            status: "unknown",
            created_at: now,
            updated_at: now,
        };
        try {
            const saved = await RpcApi.UpsertIdentityAccountCommand(
                TabRpcClient,
                accountToBackend(draft),
            );
            await refreshAccountCache();
            this.setFormOpen(false);
            this.setEditingAccount(null);
            this.setFormError(null);
            // Find the round-tripped row in the freshly-refreshed cache
            // (server timestamps will differ from the draft).
            const fresh = this.accountsAtom().find((a) => a.id === saved.id);
            this.setSelectedAccount(fresh ?? backendToAccount(saved));
        } catch (err) {
            this.setFormError((err as Error)?.message ?? String(err));
        }
    };

    updateAccount = async (id: string, data: Partial<Omit<Account, "id" | "created_at">>): Promise<void> => {
        const existing = this.accountsAtom().find((a) => a.id === id);
        if (!existing) {
            this.setFormError(`account ${id} not found`);
            return;
        }
        const merged: Account = { ...existing, ...data, updated_at: new Date().toISOString() };
        try {
            await RpcApi.UpsertIdentityAccountCommand(TabRpcClient, accountToBackend(merged));
            await refreshAccountCache();
            this.setFormOpen(false);
            this.setEditingAccount(null);
            this.setFormError(null);
            const fresh = this.accountsAtom().find((a) => a.id === id);
            this.setSelectedAccount(fresh ?? null);
        } catch (err) {
            this.setFormError((err as Error)?.message ?? String(err));
        }
    };

    deleteAccount = async (id: string): Promise<void> => {
        try {
            await RpcApi.DeleteIdentityAccountCommand(TabRpcClient, { id });
            await refreshAccountCache();
            if (this.selectedAccountAtom()?.id === id) {
                this.setSelectedAccount(null);
            }
        } catch (err) {
            // Surface delete failures via formError so the user sees them
            // even when the form isn't open.
            this.setFormError((err as Error)?.message ?? String(err));
        }
    };

    // ── Form controls ────────────────────────────────────────────────────────

    openAddForm = (): void => {
        this.setEditingAccount(null);
        this.setFormError(null);
        this.setFormOpen(true);
    };

    openEditForm = (account: Account): void => {
        this.setEditingAccount(account);
        this.setFormError(null);
        this.setFormOpen(true);
    };

    cancelForm = (): void => {
        this.setEditingAccount(null);
        this.setFormError(null);
        this.setFormOpen(false);
    };

    // ── View interface ───────────────────────────────────────────────────────

    giveFocus(): boolean {
        return false;
    }

    dispose(): void {
        // nothing to clean up — no backend subscriptions
    }
}
