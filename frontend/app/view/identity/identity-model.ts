// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { BlockNodeModel } from "@/app/block/blocktypes";
import { createSignal, type Accessor, type Setter } from "solid-js";

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

// ── Storage ──────────────────────────────────────────────────────────────────

const STORAGE_KEY = "agentmux:identity:accounts";

export function loadAccounts(): Account[] {
    try {
        const raw = localStorage.getItem(STORAGE_KEY);
        if (!raw) return [];
        return JSON.parse(raw) as Account[];
    } catch {
        return [];
    }
}

function saveAccounts(accounts: Account[]): void {
    try {
        localStorage.setItem(STORAGE_KEY, JSON.stringify(accounts));
    } catch {
        // silently ignore storage errors
    }
}

function generateId(): string {
    return `acct-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 7)}`;
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
        this.setAccounts(loadAccounts());
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

    createAccount = (data: Omit<Account, "id" | "status" | "created_at" | "updated_at">): void => {
        const now = new Date().toISOString();
        const account: Account = {
            ...data,
            id: generateId(),
            status: "unknown",
            created_at: now,
            updated_at: now,
        };
        const updated = [...this.accountsAtom(), account];
        this.setAccounts(updated);
        saveAccounts(updated);
        this.setFormOpen(false);
        this.setEditingAccount(null);
        this.setFormError(null);
        this.setSelectedAccount(account);
    };

    updateAccount = (id: string, data: Partial<Omit<Account, "id" | "created_at">>): void => {
        const updated = this.accountsAtom().map((a) =>
            a.id === id ? { ...a, ...data, updated_at: new Date().toISOString() } : a
        );
        this.setAccounts(updated);
        saveAccounts(updated);
        this.setFormOpen(false);
        this.setEditingAccount(null);
        this.setFormError(null);
        const refreshed = updated.find((a) => a.id === id) ?? null;
        this.setSelectedAccount(refreshed);
    };

    deleteAccount = (id: string): void => {
        const updated = this.accountsAtom().filter((a) => a.id !== id);
        this.setAccounts(updated);
        saveAccounts(updated);
        if (this.selectedAccountAtom()?.id === id) {
            this.setSelectedAccount(null);
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
