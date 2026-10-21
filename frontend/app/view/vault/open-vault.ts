// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { openOrFocusPaneByView } from "@/app/store/block-component-registry";
import { sectionKeyOf } from "@/app/block/pane-sections";
import type { VaultPaneSection } from "./vault-kinds";

/** Opens Warden on its Vault section, or switches the Warden already in this tab
 *  there: on the Vault's `section` if given, and with `requestId`, on the form
 *  that fills that agent's request. */
export function openVault(section?: VaultPaneSection, requestId?: string): Promise<void> {
    const meta: Record<string, string> = { [sectionKeyOf("warden")]: "vault" };
    if (section) meta[sectionKeyOf("vault")] = section;
    if (requestId) meta["vault:request"] = requestId;
    return openOrFocusPaneByView("warden", { meta: { view: "warden", ...meta } }, meta);
}
