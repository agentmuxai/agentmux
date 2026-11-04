// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Which section is open in Connectors → Accounts. Adding an account, signing in to AgentMux and
// an account's details all open in the same caret-topped section under their tile or card
// (CardGrid's detail), never in a dialog over the pane.

import { createSignal, type Accessor } from "solid-js";
import type { Account, AccountProvider, IdentityViewModel } from "@/app/view/identity/identity-model";

/** The connected grid's key for AgentMux Cloud, which is no stored account. */
export const AGENTMUX_KEY = "agentmux-cloud";

export interface OpenSection {
    /** The tile whose add section is open, if any. */
    addTile: Accessor<AccountProvider | null>;
    setAddTile: (tile: AccountProvider | null) => void;
    /** The connected grid's open key: AgentMux's card, else the selected account. */
    connectedKey: () => string | null;
    toggleAccount: (account: Account) => void;
    toggleMux: () => void;
    closeConnected: () => void;
}

export function useOpenSection(model: IdentityViewModel): OpenSection {
    const [addTile, setAddTile] = createSignal<AccountProvider | null>(null);
    const [muxOpen, setMuxOpen] = createSignal(false);
    return {
        addTile,
        setAddTile,
        connectedKey: () => (muxOpen() ? AGENTMUX_KEY : (model.selectedAccountAtom()?.id ?? null)),
        toggleAccount: (account) => {
            setMuxOpen(false);
            model.setSelectedAccount(model.selectedAccountAtom()?.id === account.id ? null : account);
        },
        toggleMux: () => {
            model.setSelectedAccount(null);
            setMuxOpen(!muxOpen());
        },
        closeConnected: () => {
            setMuxOpen(false);
            model.setSelectedAccount(null);
        },
    };
}
