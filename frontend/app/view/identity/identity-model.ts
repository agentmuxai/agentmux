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
import { waveEventSubscribe } from "@/app/store/wps";
import { Logger } from "@/util/logger";
import { brandForProvider } from "@/app/view/accounts/provider-brand";

// ── Types ────────────────────────────────────────────────────────────────────

export type AccountProvider = "github" | "openai" | "aws" | "anthropic" | "google" | "slack" | "custom" | "agentmux";
export type AccountKind = "pat" | "role" | "api_key" | "env_ref" | "oauth";
export type AccountStatus = "valid" | "expired" | "invalid" | "unknown" | "checking";
export type IdentityTab = "accounts" | "assignments";

export interface SecretRef {
    backend: "env" | "secrets_manager" | "plaintext_dev" | "keychain" | "oauth_config_dir";
    env_var?: string;
    sm_path?: string;
    sm_json_path?: string;
    value?: string; // plaintext_dev only
    // keychain only — pointer into the OS secret store. The plaintext is
    // never carried here; resolved backend-side at spawn. See
    // specs/archive/SPEC_TRUST_CENTER_2026_06_15.md §7/§12.2.
    service?: string;
    account?: string;
    // oauth_config_dir only — the provider CLI's per-account config dir.
    // Tokens live in that dir (owned/refreshed by the CLI), never here.
    dir?: string;
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
    // Armory key flow — non-secret display hint (e.g. "••••••••3f9a")
    // and per-service validation metadata. Populated by account.key.verify.
    masked_tail?: string;
    openai_model_count?: number;
    anthropic_model_count?: number;
    slack_team?: string;
    slack_user?: string;
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
     * reads this field — use parseAgentAccounts(agent) on AgentDefinition.
     * Scheduled for removal after SPEC_AGENT_IDENTITY_RESTRUCTURE Step 4
     * is fully rolled out.
     */
    assigned_agents: string[];
    status: AccountStatus;
    created_at: string;
    updated_at: string;
}

/**
 * Per-provider account references stored on a AgentDefinition.
 * A null value means no account is assigned for that provider.
 */
export type AgentAccounts = Partial<Record<AccountProvider, string | null>>;

/** Parse the JSON-encoded accounts blob from a AgentDefinition. */
export function parseAgentAccounts(agent: AgentDefinition): AgentAccounts {
    if (!agent.accounts) return {};
    try {
        return JSON.parse(agent.accounts) as AgentAccounts;
    } catch {
        return {};
    }
}

/** Serialize AgentAccounts back to the JSON blob stored on AgentDefinition. */
export function serializeAgentAccounts(accounts: AgentAccounts): string {
    return JSON.stringify(accounts);
}

/**
 * Return the IDs of agents that reference this account (reverse index).
 * Used by the global Identity panel to show "assigned agents" without
 * reading the deprecated Account.assigned_agents field.
 */
export function agentsAssignedToAccount(accountId: string, agents: AgentDefinition[]): string[] {
    return agents
        .filter((a) => {
            const accs = parseAgentAccounts(a);
            return Object.values(accs).includes(accountId);
        })
        .map((a) => a.name);
}

/**
 * Layer-4 Armory truthfulness (SPEC_ACCOUNT_DELETE_DEAUTH_LAYERS_2_4
 * §4): the delete-time notice text, derived from the RPC response's
 * `affectedAgents` (the agent ids whose links were cascaded by the
 * delete). Returns null when no agent was using the account — the
 * notice must not fire for a linkless delete.
 *
 * Wording is deliberately phrased around *linked* agents ("were using
 * this account"), not process liveness — the backend captures the link
 * set, not which of those agents currently has a live CLI process. Any
 * that ARE running still hold working tokens until restarted; we
 * disclose that, we do not pretend to revoke (spec §3).
 */
export function deleteDisclosureNotice(affectedAgents: string[] | undefined): string | null {
    const n = affectedAgents?.length ?? 0;
    if (n === 0) return null;
    return `Account deleted. ${n} agent(s) were using it — any still running hold its tokens until restarted.`;
}

export const PROVIDER_LABELS: Record<AccountProvider, string> = {
    github: "GitHub",
    openai: "OpenAI",
    aws: "AWS",
    anthropic: "Anthropic",
    google: "Google",
    slack: "Slack",
    custom: "Custom",
    agentmux: "AgentMux",
};

export const PROVIDER_COLORS: Record<AccountProvider, string> = {
    github: "#e1effe",
    openai: "#d1fae5",
    aws: "#fef3c7",
    anthropic: "#ede9fe",
    google: "#e8f0fe",
    slack: "#f3e8fd",
    custom: "#f1f5f9",
    agentmux: "#ede9fe",
};

export const KIND_LABELS: Record<AccountKind, string> = {
    pat: "Personal Access Token",
    role: "IAM Role",
    api_key: "API Key",
    env_ref: "Environment Variable",
    oauth: "OAuth (browser login)",
};

// ── Storage (DB-backed via RPC, in-memory cache) ─────────────────────────────
//
// As of v0.33.30x (PR #479 + this PR) accounts live in the SQLite
// `db_accounts` table on the sidecar, not in localStorage. This
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
        case "keychain":
            return { backend: "keychain", service: s.service, account: s.account };
        case "oauth_config_dir":
            return { backend: "oauth_config_dir", dir: s.dir };
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
        case "keychain":
            return { backend: "keychain", service: s.service ?? "", account: s.account ?? "" };
        case "oauth_config_dir":
            return { backend: "oauth_config_dir", dir: s.dir ?? "" };
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

/** Pull the latest account list from the DB and update the cache.
 *
 * Latest-call-wins: two rapid `identityaccounts:changed` broadcasts (or a
 * broadcast racing a CRUD method's own refresh) launch overlapping RPCs,
 * and the backend handles requests on separate tasks — the request that
 * captured the OLDER account snapshot can resolve last. Without this
 * guard it would overwrite the newer result with no further broadcast to
 * correct it, leaving every consumer stale (codex P2 on #2474). A
 * superseded call skips the cache write and returns the live cache
 * (fresher than its own stale fetch) so awaiting callers still see the
 * newest data.
 */
let _refreshSeq = 0;
export async function refreshAccountCache(): Promise<Account[]> {
    const seq = ++_refreshSeq;
    try {
        const rows = await RpcApi.ListIdentityAccountsCommand(TabRpcClient, {});
        if (seq !== _refreshSeq) return _accountCache; // superseded by a newer refresh
        const mapped = rows.map(backendToAccount);
        setCache(mapped);
        return mapped;
    } catch (err) {
        Logger.warn("identity", `refreshAccountCache failed: ${(err as Error)?.message ?? err}`);
        return _accountCache;
    }
}

let _liveSyncInstalled = false;

/** Run once at app startup. Idempotent — repeated calls just refresh. */
export function primeAccountCache(): void {
    void refreshAccountCache();
    // Live sync: refresh the cache on every backend `identityaccounts:changed`
    // broadcast, so accounts created/edited/deleted by ANY path — the in-app
    // OAuth login persist, API-key verify, upsert/delete RPCs from another
    // tab, the spawn-time expiry probe — propagate to every cache consumer
    // (Armory Accounts tab, launch modal, pickers) without a reload.
    // Previously the cache was only refreshed by its own CRUD methods'
    // explicit calls, so backend-originated account creation (e.g. completing
    // an in-app OAuth login) left the Armory stale until reopen/reload, and
    // each consumer that wanted liveness had to hand-roll its own event
    // subscription (AgentLaunchModal did exactly that).
    //
    // App-lifetime subscription, deliberately never unsubscribed — the cache
    // itself is module-level app-lifetime state. Guarded so repeated
    // primeAccountCache() calls don't stack duplicate handlers.
    if (!_liveSyncInstalled) {
        _liveSyncInstalled = true;
        waveEventSubscribe({
            eventType: "identityaccounts:changed",
            handler: () => void refreshAccountCache(),
        });
    }
}

// ── ViewModel ────────────────────────────────────────────────────────────────

export class IdentityViewModel implements ViewModel {
    viewType = "identity";
    blockId: string;
    nodeModel: BlockNodeModel | null;

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

    // Preset provider/kind for a fresh Add form, set when the user clicks a
    // brand tile in the Accounts gallery (→ openAddFormFor). The AccountForm
    // reads this for its initial Provider/Kind when not editing.
    private _addPreset = createSignal<{ provider: AccountProvider; kind: AccountKind } | null>(null);
    addPresetAtom: Accessor<{ provider: AccountProvider; kind: AccountKind } | null> = this._addPreset[0];
    private setAddPreset: Setter<{ provider: AccountProvider; kind: AccountKind } | null> = this._addPreset[1];

    private _formError = createSignal<string | null>(null);
    formErrorAtom: Accessor<string | null> = this._formError[0];
    private setFormError: Setter<string | null> = this._formError[1];

    // Delete-time disclosure notice (layer 4, spec §4). Transient —
    // set from the delete RPC's `affectedAgents`, cleared by the user
    // (dismiss) or the next delete. No persistent state: the account
    // row is gone (correct); this is disclosure at delete time only.
    private _deleteNotice = createSignal<string | null>(null);
    deleteNoticeAtom: Accessor<string | null> = this._deleteNotice[0];
    private setDeleteNotice: Setter<string | null> = this._deleteNotice[1];

    dismissDeleteNotice = (): void => {
        this.setDeleteNotice(null);
    };

    /** Surface a transient notice in the Accounts tab's banner row (the
     *  same dismissible UI the delete disclosure uses). Added for the
     *  Bind-to-Agent menu's failure feedback (reagentx P1 on #2485) —
     *  a failed bind RPC must not vanish into logs only. */
    showNotice = (text: string): void => {
        this.setDeleteNotice(text);
    };

    /** Unsubscribe from the account cache; assigned in the constructor,
     *  invoked in dispose() so direct callers don't leak a listener. */
    private _unsubAccounts: (() => void) | null = null;

    constructor(blockId: string, nodeModel: BlockNodeModel | null) {
        this.blockId = blockId;
        this.nodeModel = nodeModel;
        // Initial paint from cache (may be empty on first launch).
        this.setAccounts(loadAccounts());
        // Stay in sync with module-level cache updates from any source
        // (other panes, RPC events, etc.). The unsubscribe is stashed and
        // invoked in dispose() so callers that mount this model directly
        // (e.g. the Armory Accounts tab) don't leak a listener per
        // open/close.
        this._unsubAccounts = subscribeAccountChanges((accounts) => {
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
    }

    // ── Derived helpers ──────────────────────────────────────────────────────

    // Group by *brand*, not raw provider, so a CLI-OAuth account (e.g.
    // provider "claude" — the Claude CLI's `~/.claude` login) surfaces under
    // its brand tile ("anthropic"). Display-only: the account's stored
    // `provider` is unchanged, so spawn-time env injection is unaffected.
    // See specs/archive/SPEC_TRUST_CENTER_CLI_AUTH_BINDING_2026_06_17.md.
    accountsByProvider = (): Map<AccountProvider, Account[]> => {
        const map = new Map<AccountProvider, Account[]>();
        const order: AccountProvider[] = ["github", "google", "aws", "openai", "anthropic", "slack", "custom", "agentmux"];
        for (const p of order) {
            const group = this.accountsAtom().filter((a) => brandForProvider(a.provider) === p);
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
            const result = await RpcApi.DeleteIdentityAccountCommand(TabRpcClient, { id });
            // Layer-4 disclosure (spec §4): agents that were using the
            // account may still hold its tokens in a live process.
            // null (no affected agents) also clears any stale notice
            // from a previous delete.
            this.setDeleteNotice(deleteDisclosureNotice(result?.affectedAgents));
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
        this.setAddPreset(null);
        this.setFormError(null);
        this.setFormOpen(true);
    };

    /** Open a fresh Add form pre-set to a provider + auth kind (brand tile). */
    openAddFormFor = (provider: AccountProvider, kind: AccountKind): void => {
        this.setEditingAccount(null);
        this.setAddPreset({ provider, kind });
        this.setFormError(null);
        this.setFormOpen(true);
    };

    openEditForm = (account: Account): void => {
        this.setSelectedAccount(null); // close detail modal before opening form
        this.setEditingAccount(account);
        this.setAddPreset(null);
        this.setFormError(null);
        this.setFormOpen(true);
    };

    cancelForm = (): void => {
        this.setEditingAccount(null);
        this.setAddPreset(null);
        this.setFormError(null);
        this.setFormOpen(false);
    };

    // ── View interface ───────────────────────────────────────────────────────

    giveFocus(): boolean {
        return false;
    }

    dispose(): void {
        // Drop the account-cache subscription so direct callers (Trust
        // Center Accounts tab) don't leak a listener per open/close.
        this._unsubAccounts?.();
        this._unsubAccounts = null;
    }
}

// Test-only hook for the (deliberately un-exported) SecretRef translators —
// the pattern identity-model.test.ts's header asks for. Not part of the
// public surface; do not import outside tests.
export const __internal__ = { secretRefFromBackend, secretRefToBackend };
