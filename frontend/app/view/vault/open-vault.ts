// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { openOrFocusPaneByView } from "@/app/store/block-component-registry";
import type { VaultPaneSection } from "./vault-kinds";

/** Opens the Vault, or switches the one already in this tab: on its `section` if given, and with
 *  `requestId`, on the form that fills that agent's request. */
export function openVault(section?: VaultPaneSection, requestId?: string): Promise<void> {
    const meta: Record<string, string> = {};
    if (section) meta["vault:section"] = section;
    if (requestId) meta["vault:request"] = requestId;
    return openOrFocusPaneByView("vault", { meta: { view: "vault", ...meta } }, Object.keys(meta).length > 0 ? meta : undefined);
}
