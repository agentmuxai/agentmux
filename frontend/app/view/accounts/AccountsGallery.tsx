// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AccountsGallery — the brand-tile landing for the Armory Accounts tab.
 * Renders a grid of service logos, each with a badge showing how many accounts
 * are connected for that brand. Clicking a tile opens a small chooser of the
 * brand's auth modes (OAuth / Key); picking one opens the Add-account form
 * preset to that provider + kind. docs/specs/archive/SPEC_TRUST_CENTER_2026_06_15.md §3.
 */

import { createSignal, For, Show, type JSX } from "solid-js";
import { ProviderLogo } from "@/element/ProviderLogo";
import type { AccountKind, AccountProvider, IdentityViewModel } from "@/app/view/identity/identity-model";
import { SERVICE_CATALOG, modeLabel, type AuthMode, type ServiceTile } from "./accounts-catalog";
import "./accounts-gallery.scss";

export function AccountsGallery(props: {
    model: IdentityViewModel;
    /** True when an AgentMux Cloud session is connected — drives the agentmux
     *  tile's "1 connected" badge (projected, not a stored IdentityAccount). */
    agentMuxConnected?: () => boolean;
    /** Open the dedicated AgentMux connect panel instead of the OAuth/Key
     *  chooser. Required for the agentmux tile to do anything. */
    onAgentMux?: () => void;
    /** Open ClaudeLoginPanel (the in-app CLI login session, spec
     *  SPEC_INAPP_CLAUDE_OAUTH_LOGIN_2026_08_03.md §3.3 surface 3) instead
     *  of the generic Add-account form's OAuth path — Anthropic isn't in
     *  oauth-catalog.ts's service-OAuth scaffold (no device/PKCE endpoint
     *  an app-driven client could hit), so its "oauth" mode needs its own
     *  panel the same way the agentmux tile needs its own connect flow.
     *  Unlike agentmux, this fires from the chooser's OAuth button (the
     *  anthropic tile still offers Key too), not at tile-click time. */
    onClaudeConnect?: () => void;
}): JSX.Element {
    const model = props.model;
    const [chooser, setChooser] = createSignal<ServiceTile | null>(null);

    const countFor = (id: AccountProvider): number => {
        if (id === "agentmux") return props.agentMuxConnected?.() ? 1 : 0;
        return model.accountsByProvider().get(id)?.length ?? 0;
    };

    const pick = (tile: ServiceTile, mode: AuthMode) => {
        setChooser(null);
        if (tile.id === "anthropic" && mode === "oauth") {
            props.onClaudeConnect?.();
            return;
        }
        const kind: AccountKind = mode === "oauth" ? "oauth" : tile.keyKind;
        model.openAddFormFor(tile.id, kind);
    };

    const openTile = (tile: ServiceTile) => {
        // AgentMux Cloud is a singleton session, not a per-credential account —
        // open its dedicated connect panel rather than the OAuth/Key chooser.
        if (tile.id === "agentmux") {
            props.onAgentMux?.();
            return;
        }
        setChooser(tile);
    };

    return (
        <div class="accounts-gallery">
            <div class="accounts-gallery-grid">
                <For each={SERVICE_CATALOG}>
                    {(tile) => {
                        const n = () => countFor(tile.id);
                        return (
                            <button
                                type="button"
                                class="account-tile"
                                classList={{ "account-tile--connected": n() > 0 }}
                                onClick={() => openTile(tile)}
                                aria-label={`${tile.displayName} — ${n()} connected, add account`}
                            >
                                <Show when={n() > 0}>
                                    <span class="account-tile-count" title={`${n()} connected`}>{n()}</span>
                                </Show>
                                <span class="account-tile-logo">
                                    <ProviderLogo provider={tile.id} size={32} />
                                </span>
                                <span class="account-tile-name">{tile.displayName}</span>
                                <span class="account-tile-status">
                                    {n() > 0 ? `${n()} connected` : (tile.blurb ?? "Connect")}
                                </span>
                            </button>
                        );
                    }}
                </For>
            </div>

            {/* Tile-click chooser: OAuth and/or Key, per the brand's authModes. */}
            <Show when={chooser()}>
                {(tile) => (
                    <div
                        class="accounts-chooser-overlay"
                        onClick={(e) => e.target === e.currentTarget && setChooser(null)}
                    >
                        <div class="accounts-chooser" role="dialog" aria-label={`Connect ${tile().displayName}`}>
                            <div class="accounts-chooser-header">
                                <span class="account-tile-logo">
                                    <ProviderLogo provider={tile().id} size={20} />
                                </span>
                                <span class="accounts-chooser-title">Connect {tile().displayName}</span>
                                <button
                                    type="button"
                                    class="accounts-chooser-close"
                                    onClick={() => setChooser(null)}
                                    aria-label="Close"
                                >
                                    ✕
                                </button>
                            </div>
                            <div class="accounts-chooser-modes">
                                <For each={tile().authModes}>
                                    {(mode) => {
                                        const m = modeLabel(mode);
                                        return (
                                            <button
                                                type="button"
                                                class="accounts-chooser-mode"
                                                onClick={() => pick(tile(), mode)}
                                            >
                                                <span class="accounts-chooser-mode-title">{m.title}</span>
                                                <span class="accounts-chooser-mode-sub">{m.sub}</span>
                                            </button>
                                        );
                                    }}
                                </For>
                            </div>
                        </div>
                    </div>
                )}
            </Show>
        </div>
    );
}

AccountsGallery.displayName = "AccountsGallery";
