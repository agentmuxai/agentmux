// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The context menu for Connectors → Accounts rows. There is no "Bind to Agent" in it, and no
// frontend bind: an agent's account is changed through `agent.account.switch`
// (SPEC_ACCOUNT_SWITCH_FROM_THE_COMPOSER_NO_BIND_2026_10_26.md).

import { MENU_ICON } from "@/app/util/menu-icons";
import type { Account } from "./identity-model";

/**
 * The context-menu items for an account row. There is no "Bind to Agent" here:
 * an agent's account is changed from the login email in its composer
 * (SPEC_ACCOUNT_SWITCH_FROM_THE_COMPOSER_NO_BIND_2026_10_26.md).
 */
export function buildAccountRowMenu(
    account: Account,
    /** "Delete account…" — opens the shared delete confirmation
     *  (SPEC_ARMORY_ACCOUNTS_DELETE_AND_INLINE_DETAIL_2026_10_04.md §3.1).
     *  Omitted → no delete item. */
    onDelete?: () => void,
): ContextMenuItem[] {
    return [
        {
            label: "Copy account ID",
            icon: MENU_ICON.copy,
            click: () => {
                void import("@/util/clipboard").then(({ writeText }) => writeText(account.id));
            },
        },
        // Last, after a separator, so it is never the item under the
        // pointer when the menu opens.
        ...(onDelete
            ? ([{ type: "separator" }, { label: "Delete account…", icon: MENU_ICON.delete, click: onDelete }] as ContextMenuItem[])
            : []),
    ];
}
