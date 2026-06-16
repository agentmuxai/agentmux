// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// AccountsManager — the context-free Accounts management UI, rendered as the
// **Accounts** tab of the Trust Center (bundle-manager-modal). Phase 1 of
// SPEC_TRUST_CENTER_2026_06_15.md: surface the existing IdentityAccount model
// app-wide with no backend changes.
//
// It owns a block-free `IdentityViewModel` instance (the existing accounts
// view-model — account CRUD goes through the module-level RPC + cache, so the
// blockId/nodeModel it carries for the ViewModel interface are inert here) and
// composes the already-built `AccountsTab` + `AccountForm` so styling and the
// add/edit/delete lifecycle come for free. Identity-bundle and Memory
// management remain in their own Trust Center tabs.

import { onCleanup, Show, type JSX } from "solid-js";
import type { BlockNodeModel } from "@/app/block/blocktypes";
import { IdentityViewModel } from "@/app/view/identity/identity-model";
import { AccountsTab, AccountForm } from "@/app/view/identity/identity-view";
import { AccountsGallery } from "./AccountsGallery";
import "@/app/view/identity/identity-view.scss";

export function AccountsManager(): JSX.Element {
    // Block-free instance. `IdentityViewModel` only uses blockId/nodeModel to
    // satisfy the ViewModel interface; all account CRUD flows through the
    // module-level cache + `*IdentityAccountCommand` RPCs, so a synthetic id
    // and a stub nodeModel are safe. Created once per mount (SolidJS component
    // bodies run a single time).
    const model = new IdentityViewModel("trust-center:accounts", {} as BlockNodeModel);
    onCleanup(() => model.dispose());

    return (
        <div class="identity-view">
            <div class="identity-header">
                <span class="identity-header-title">Accounts</span>
                <button
                    class="identity-add-btn"
                    onClick={() => model.openAddForm()}
                    title="Add account"
                >
                    + Add
                </button>
            </div>

            <div class="identity-body">
                {/* Brand-tile landing: pick a service → OAuth / Key. */}
                <AccountsGallery model={model} />
                {/* Connected accounts (manage existing). Gated on having any —
                    the gallery above is the empty/landing state. */}
                <Show when={model.accountsAtom().length > 0}>
                    <div class="accounts-connected-heading">Connected accounts</div>
                    <AccountsTab model={model} />
                </Show>
            </div>

            <Show when={model.formOpenAtom()}>
                <AccountForm model={model} />
            </Show>
        </div>
    );
}

AccountsManager.displayName = "AccountsManager";
