// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What a row's button does (docs/specs/SPEC_STATUS_BAR_COMPACT_SUMMARY_AND_VAULT_REQUESTS_2026_11_15.md §2): a pane is
 * revealed (`goToAttention`, the one reveal path), a Vault request opens the Vault's Requests list at it, the Kanban's
 * row opens the Kanban, and a request that belongs to no pane goes to where its kind is answered (`KIND_PLACES`).
 */

import { goToAttention } from "@/app/notification/attention-go";
import { openVersionPanel } from "@/app/store/app-update";
import { openOrFocusPaneByView } from "@/app/store/block-component-registry";
import { openSettingsAt } from "@/app/view/settings/open-settings";
import { openVaultAt } from "@/app/view/vault/open-vault";
import type { SummaryClick } from "./summary";
import { KIND_PLACES, type AnswerPlace } from "./summary-kinds";

/** The Settings section that answers a request of each kind; the upgrade and the Vault have their own place, and a desktop request
 *  is answered in its own prompt (Settings holds only the global switch), so it goes to the Swarm until srv names its pane. */
const SETTINGS_SECTION: Partial<Record<AnswerPlace, string>> = {
    connections: "devices",
    widgets: "widgets",
};

/** Where a request with no pane of its own is answered. */
async function goToAnswerPlace(kind: string, key: string): Promise<void> {
    const place = KIND_PLACES[kind]?.answer ?? "pane";
    if (place === "upgrade") {
        openVersionPanel();
        return;
    }
    if (place === "vault") return openVaultAt(key);
    const section = SETTINGS_SECTION[place];
    if (section) return openSettingsAt(section);
    // A kind answered in a pane, announced with no pane: the Swarm is where every agent can be found.
    return openOrFocusPaneByView("swarm");
}

/** Takes the person to what `click` names; resolves once the pane is shown. */
export async function goToSummaryRow(click: SummaryClick): Promise<void> {
    switch (click.to) {
        case "pane":
            return goToAttention(click.item);
        case "vault":
            return openVaultAt(click.id);
        case "kanban":
            return openOrFocusPaneByView("kanban");
        case "app":
            return goToAnswerPlace(click.kind, click.key);
    }
}
