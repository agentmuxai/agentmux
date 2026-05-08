// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Identity pane — first-class management of Identity bundles.
//
// An Identity bundle is a named credential set (e.g. "Work", "Personal")
// that contains one Account per provider via the v7
// `db_identity_bindings` junction. Bundles are reusable across many
// agent instances — pick one in the launch modal alongside a Memory.
//
// This module owns the standalone-pane ViewModel (`view: "identity"`).
// It is distinct from `identity-model.ts` which owns the Account-level
// CRUD that the agent pane's old Identity tab used. The new pane
// consumes Account types from that older module but does its own
// bundle-level state management.

import { BlockNodeModel } from "@/app/block/blocktypes";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { getWaveObjectAtom, makeORef } from "@/app/store/wos";
import { createEffect, createMemo, createSignal, onCleanup, type Accessor } from "solid-js";

import {
    type Account,
    loadAccounts,
    PROVIDER_LABELS,
    refreshAccountCache,
    subscribeAccountChanges,
} from "./identity-model";

/** Form-state shape for editing an Identity bundle. */
export interface IdentityBundleDraft {
    id?: string;
    name: string;
    description: string;
}

export function emptyBundleDraft(): IdentityBundleDraft {
    return { id: undefined, name: "", description: "" };
}

export function bundleDraftFrom(b: IdentityBundle): IdentityBundleDraft {
    return {
        id: b.id,
        name: b.name,
        description: b.description ?? "",
    };
}

/** Wire shape for `upsertidentitybundle`. */
export function bundleDraftToWire(d: IdentityBundleDraft): Partial<IdentityBundle> {
    return {
        id: d.id,
        name: d.name.trim(),
        description: d.description.trim(),
    };
}

export class IdentityPaneViewModel implements ViewModel {
    viewType = "identity";
    blockId: string;
    nodeModel: BlockNodeModel;

    viewIcon: Accessor<string> = () => "user";
    viewName: Accessor<string>;
    viewText: Accessor<string | HeaderElem[]> = () => "Identity";
    noPadding: Accessor<boolean> = () => false;

    get viewComponent(): ViewComponent {
        return null; // overridden by the barrel
    }

    blockAtom: Accessor<Block | undefined>;

    private _bundles = createSignal<IdentityBundle[]>([]);
    bundlesAtom: Accessor<IdentityBundle[]> = this._bundles[0];
    setBundles = this._bundles[1];

    private _selectedId = createSignal<string | null>(null);
    selectedIdAtom: Accessor<string | null> = this._selectedId[0];
    setSelectedId = this._selectedId[1];

    private _draft = createSignal<IdentityBundleDraft | null>(null);
    draftAtom: Accessor<IdentityBundleDraft | null> = this._draft[0];
    setDraft = this._draft[1];

    private _saving = createSignal<boolean>(false);
    savingAtom: Accessor<boolean> = this._saving[0];
    setSaving = this._saving[1];

    private _error = createSignal<string | null>(null);
    errorAtom: Accessor<string | null> = this._error[0];
    setError = this._error[1];

    /** All accounts (cached); used to populate per-provider binding pickers.
     *  Subscribed to identity-model's account-change broadcaster so that
     *  accounts created externally (e.g. via the agent-pane Identity tab)
     *  appear immediately. Reagent + codex P1 (#748). */
    private _accounts = createSignal<Account[]>([]);
    accountsAtom: Accessor<Account[]> = this._accounts[0];
    setAccounts = this._accounts[1];

    /** Bindings for the currently-selected Identity bundle. */
    private _bindings = createSignal<IdentityBinding[]>([]);
    bindingsAtom: Accessor<IdentityBinding[]> = this._bindings[0];
    setBindings = this._bindings[1];

    selectedBundleAtom: Accessor<IdentityBundle | null>;

    /** Unsubscribe handle for the account-change subscription. */
    private _unsubscribeAccounts: (() => void) | null = null;

    constructor(blockId: string, nodeModel: BlockNodeModel) {
        this.blockId = blockId;
        this.nodeModel = nodeModel;
        this.blockAtom = getWaveObjectAtom(makeORef("block", blockId));
        this.viewName = createMemo(() => {
            const block = this.blockAtom();
            return (block?.meta?.["frame:title"] as string) ?? "Identity";
        });

        this.selectedBundleAtom = createMemo(() => {
            const id = this.selectedIdAtom();
            if (!id) return null;
            return this.bundlesAtom().find((b) => b.id === id) ?? null;
        });

        // Refresh bindings whenever the selection changes. Use
        // createEffect (not createMemo) — the previous version used
        // createMemo whose return value is discarded; createMemo is
        // lazy and only re-runs when its return value is consumed,
        // so the effect fired exactly once at construction. Reagent
        // P1 (PR #748).
        createEffect(() => {
            const id = this.selectedIdAtom();
            void this.refreshBindings(id);
        });

        // Hydrate the account list from the shared cache, then subscribe
        // to changes so externally-created accounts (via the agent-pane
        // Identity tab) appear instantly without a remount.
        void (async () => {
            try {
                await refreshAccountCache();
            } catch {
                // Cache refresh errors are non-fatal — we still subscribe
                // and any later refresh by another consumer will populate.
            }
            this.setAccounts(loadAccounts());
        })();
        this._unsubscribeAccounts = subscribeAccountChanges((accounts) => {
            this.setAccounts(accounts);
        });
        onCleanup(() => {
            this._unsubscribeAccounts?.();
            this._unsubscribeAccounts = null;
        });

        void this.refreshBundles();
    }

    /** Re-fetch the bundle list. */
    async refreshBundles(): Promise<void> {
        try {
            const list = await RpcApi.ListIdentityBundlesCommand(TabRpcClient, {});
            this.setBundles(list);
            this.setError(null);
        } catch (e) {
            this.setError(`Failed to load Identity bundles: ${(e as Error).message ?? e}`);
        }
    }

    startNew(): void {
        this.setDraft(emptyBundleDraft());
        this.setSelectedId(null);
    }

    startEdit(bundle: IdentityBundle): void {
        if (bundle.is_blank) {
            this.setError("The blank Identity is system-managed and cannot be edited.");
            return;
        }
        this.setDraft(bundleDraftFrom(bundle));
        this.setSelectedId(bundle.id);
    }

    cancelDraft(): void {
        this.setDraft(null);
    }

    async saveDraft(): Promise<void> {
        const draft = this.draftAtom();
        if (!draft) return;
        if (!draft.name.trim()) {
            this.setError("Identity name is required.");
            return;
        }
        this.setSaving(true);
        this.setError(null);
        try {
            const saved = await RpcApi.UpsertIdentityBundleCommand(
                TabRpcClient,
                bundleDraftToWire(draft),
            );
            this.setDraft(null);
            this.setSelectedId(saved.id);
            await this.refreshBundles();
        } catch (e) {
            this.setError(`Save failed: ${(e as Error).message ?? e}`);
        } finally {
            this.setSaving(false);
        }
    }

    async deleteBundle(id: string): Promise<void> {
        const target = this.bundlesAtom().find((b) => b.id === id);
        if (target?.is_blank) {
            this.setError("The blank Identity is system-managed and cannot be deleted.");
            return;
        }
        this.setError(null);
        try {
            await RpcApi.DeleteIdentityBundleCommand(TabRpcClient, { id });
            if (this.selectedIdAtom() === id) this.setSelectedId(null);
            this.setDraft(null);
            await this.refreshBundles();
        } catch (e) {
            this.setError(`Delete failed: ${(e as Error).message ?? e}`);
        }
    }

    /** Set the account binding for `(identity, provider)`. account_id =
     *  empty string means unbind. */
    async setBinding(provider: string, account_id: string): Promise<void> {
        const id = this.selectedIdAtom();
        if (!id) return;
        this.setError(null);
        try {
            if (account_id === "") {
                await RpcApi.UnbindIdentityAccountCommand(TabRpcClient, {
                    identity_id: id,
                    provider,
                });
            } else {
                await RpcApi.BindIdentityAccountCommand(TabRpcClient, {
                    identity_id: id,
                    provider,
                    account_id,
                });
            }
            await this.refreshBindings(id);
        } catch (e) {
            this.setError(`Binding update failed: ${(e as Error).message ?? e}`);
        }
    }

    /** Re-fetch bindings for a specific identity_id. Pulls into the
     *  bindings signal so memos that read it react. */
    async refreshBindings(identity_id: string | null): Promise<void> {
        if (!identity_id) {
            this.setBindings([]);
            return;
        }
        try {
            const list = await RpcApi.ListIdentityBindingsCommand(TabRpcClient, {
                identity_id,
            });
            this.setBindings(list);
        } catch (e) {
            this.setError(`Failed to load bindings: ${(e as Error).message ?? e}`);
            this.setBindings([]);
        }
    }

    /** Account list grouped by provider, for the per-provider binding rows. */
    accountsByProvider = createMemo<Map<string, Account[]>>(() => {
        const m = new Map<string, Account[]>();
        for (const a of this.accountsAtom()) {
            if (!m.has(a.provider)) m.set(a.provider, []);
            m.get(a.provider)!.push(a);
        }
        return m;
    });

    /** Return the providers we should render rows for: any provider that
     *  has at least one account, plus any provider that's already bound
     *  (so an existing binding stays visible even if the underlying
     *  account was deleted). */
    providersForBindingRows = createMemo<string[]>(() => {
        const set = new Set<string>();
        for (const provider of this.accountsByProvider().keys()) set.add(provider);
        for (const b of this.bindingsAtom()) set.add(b.provider);
        return Array.from(set).sort();
    });

    providerLabel(provider: string): string {
        return (PROVIDER_LABELS as Record<string, string>)[provider] ?? provider;
    }

    dispose(): void {
        this._unsubscribeAccounts?.();
        this._unsubscribeAccounts = null;
    }
}
