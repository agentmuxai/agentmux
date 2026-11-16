// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// An Anthropic API key as an account of a Claude agent's chip menu
// (docs/specs/SPEC_AGENT_PANE_API_KEY_ACCOUNT_SWITCH_2026_11_16.md). A key is stored under provider
// `anthropic`, not `claude`, so the provider match the menu uses for plans does not see it; these
// helpers are the one place that says what a key is, what it is called and where it is offered.

import { accountLabel, boundAccountEmail, type Account } from "@/app/view/identity/identity-model";
import { resolveProviderAlias } from "../providers";
import { lastLinkedAccountId } from "../providers/provider-id-aliases";

/** The mark a key account carries wherever it is named, so it cannot be mistaken for a plan. */
export const API_MARK = "API";
/** Why a container agent's menu has no key rows. */
export const KEYS_NOT_FOR_CONTAINERS = "API keys aren't offered to container agents yet";

/** An Anthropic API key: provider `anthropic`, kind `api_key`. */
export function isKeyAccount(a: Pick<Account, "provider" | "kind">): boolean {
    return a.kind === "api_key" && a.provider === "anthropic";
}

/** The name the person gave the key: its display name, else its account name. */
export function keyName(a: Pick<Account, "name" | "display_name">): string {
    return a.display_name?.trim() || a.name;
}

/** What the chip and its notices call a key: "Work key · API". */
export function keyLabel(a: Pick<Account, "name" | "display_name">): string {
    return `${keyName(a)} · ${API_MARK}`;
}

/** What a plan or a key is called in the chip: a plan by its login, a key by its name and mark. */
export function chipLabel(a: Account, all: readonly Account[]): string {
    return isKeyAccount(a) ? keyLabel(a) : accountLabel(a, all);
}

/** Whether `a` can be offered in the chip menu of a `providerId` agent: its provider's signed-in plans, and, for Claude,
 *  the Anthropic keys that have not been found invalid. A key saved without a check (`unknown`) is offered. */
export function offered(providerId: string, a: Account, sameProvider: (a: Account) => boolean): boolean {
    if (resolveProviderAlias(providerId) === "claude" && isKeyAccount(a)) return a.status === "valid" || a.status === "unknown";
    return sameProvider(a) && a.status === "valid";
}

/** The account an agent is on: its Anthropic key link when it has one (what pays), else its provider's. */
export function currentAccountId(links: Array<{ provider: string; account_id: string }>, providerId: string): string | undefined {
    if (resolveProviderAlias(providerId) === "claude") {
        const keys = links.filter((l) => l.provider.toLowerCase() === "anthropic");
        if (keys.length > 0) return keys[keys.length - 1].account_id;
    }
    return lastLinkedAccountId(links, providerId);
}

/** What the chip names the account the agent is on: a key by its name and mark, a plan by its login email. */
export function boundLabel(accounts: readonly Account[], linkedId: string | undefined): string | undefined {
    const bound = accounts.find((a) => a.id === linkedId);
    return bound && isKeyAccount(bound) ? keyLabel(bound) : boundAccountEmail(accounts, linkedId);
}

/** The accounts the chip menu offers a providerId agent, newest first. A container agent is not offered a key. */
export function menuChoices(accounts: readonly Account[], providerId: string, container: boolean): Account[] {
    const provider = resolveProviderAlias(providerId);
    return accounts
        .filter((a) => offered(providerId, a, (x) => resolveProviderAlias(x.provider) === provider) && !(container && isKeyAccount(a)))
        .sort((a, b) => b.updated_at.localeCompare(a.updated_at));
}

/** Whether a container agent has keys it is not offered, so its menu says why. */
export function keysHeldBack(accounts: readonly Account[], providerId: string, container: boolean): boolean {
    return container && resolveProviderAlias(providerId) === "claude" && accounts.some((a) => isKeyAccount(a) && offered(providerId, a, () => false));
}

/** One account's row in the chip menu: a plan by its login and quota note, a key by its name, the mark and a key icon. */
export function menuRow(a: Account, all: readonly Account[], current: string | undefined, quotaNote: string | undefined, click: () => void): ContextMenuItem {
    const key = isKeyAccount(a);
    return {
        label: key ? keyName(a) : accountLabel(a, all),
        sublabel: key ? API_MARK : quotaNote,
        ...(key ? { icon: "key" } : {}),
        type: "checkbox",
        checked: a.id === current,
        click,
    };
}
