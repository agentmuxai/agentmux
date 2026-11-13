// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AccountsGallery — the brand-tile landing for Connectors → Accounts.
 * Renders a grid of service logos, each with a badge showing how many accounts
 * are connected for that brand. Clicking a tile opens the add section under it (the
 * same caret-topped section a connected account's details use): its auth modes
 * (OAuth / Key), then the sign-in or the form, in place. docs/specs/archive/SPEC_TRUST_CENTER_2026_06_15.md §3.
 */

import type { JSX } from "solid-js";
import { Card, CardGrid } from "@/app/element/ui";
import { ProviderLogo } from "@/element/ProviderLogo";
import type { AccountProvider, IdentityViewModel } from "@/app/view/identity/identity-model";
import { SERVICE_CATALOG, type ServiceTile } from "./accounts-catalog";
import "./accounts-gallery.scss";
import { tip } from "@/app/element/tip";

export function AccountsGallery(props: {
    model: IdentityViewModel;
    /** True when an AgentMux Cloud session is connected — drives the agentmux
     *  tile's "1 connected" badge (projected, not a stored IdentityAccount). */
    agentMuxConnected?: () => boolean;
    /** The tile whose add section is open, if any. */
    selected: () => AccountProvider | null;
    /** A tile was picked (or its section closed, with null). */
    onSelect: (tile: AccountProvider | null) => void;
    /** What opens under the picked tile. */
    renderDetail: (tile: ServiceTile) => JSX.Element;
}): JSX.Element {
    const model = props.model;
    const countFor = (id: AccountProvider): number => {
        if (id === "agentmux") return props.agentMuxConnected?.() ? 1 : 0;
        return model.accountsByProvider().get(id)?.length ?? 0;
    };

    return (
        <div class="accounts-gallery">
            {/* Chip cards, the same component as the connected accounts below
                (SPEC_CONNECTORS_CARDS_2026_10_13.md §3.2). */}
            <CardGrid
                size="chip"
                minCardWidth={150}
                class="accounts-gallery-grid"
                ariaLabel="Add an account"
                items={SERVICE_CATALOG}
                keyOf={(tile) => tile.id}
                selectedKey={props.selected()}
                renderDetail={props.renderDetail}
                onClose={() => props.onSelect(null)}
                renderCard={(tile) => {
                    const n = () => countFor(tile.id);
                    return (
                        <Card
                            size="chip"
                            class={n() > 0 ? "account-tile account-tile--connected" : "account-tile"}
                            aria-label={`${tile.displayName} — ${n()} connected, add account`}
                            icon={<ProviderLogo provider={tile.id} size={20} />}
                            title={tile.displayName}
                            subtitle={n() > 0 ? `${n()} connected` : (tile.blurb ?? "Connect")}
                            badge={
                                n() > 0 ? (
                                    <span class="account-tile-count" {...tip(`${n()} connected`)}>
                                        {n()}
                                    </span>
                                ) : undefined
                            }
                            onSelect={() => props.onSelect(props.selected() === tile.id ? null : tile.id)}
                        />
                    );
                }}
            />
        </div>
    );
}

AccountsGallery.displayName = "AccountsGallery";
