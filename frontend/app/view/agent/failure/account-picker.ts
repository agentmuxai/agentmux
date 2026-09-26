// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What to do when the user asks to bind one of an agent's candidate accounts,
 * and the menu rows for the picker. Pure, so the two entry points' difference
 * is unit-tested rather than living untested inside agent-view.tsx.
 * SPEC_COMPOSER_ACCOUNT_SWITCH_AND_JEKT_HEIGHT_CAP_2026_09_26.md Part A.
 *
 * - `"adopt"`: the failure row's "Bind account". The agent is already broken, so
 *   a lone candidate is bound at once.
 * - `"switch"`: the composer chip's link. The agent works and a switch restarts
 *   it, so the user always picks a named destination, even from one candidate.
 */

import { accountLabel, type Account } from "@/app/view/identity/identity-model";

export type BindMode = "adopt" | "switch";

export type BindPlan =
    | { kind: "none" }
    | { kind: "bind"; account: Account }
    | { kind: "pick"; candidates: Account[] };

export function planBind(mode: BindMode, candidates: Account[]): BindPlan {
    if (candidates.length === 0) return { kind: "none" };
    if (mode === "adopt" && candidates.length === 1) return { kind: "bind", account: candidates[0] };
    return { kind: "pick", candidates };
}

/** Menu rows: the login email, else the name (every Claude account is named
 *  `claude-oauth`, so the name alone cannot tell them apart), with an optional
 *  verb in front ("Switch to "). */
export function accountPickerItems(
    candidates: Account[],
    bind: (account: Account) => void,
    labelPrefix = ""
): ContextMenuItem[] {
    return candidates.map((acct) => ({
        label: `${labelPrefix}${accountLabel(acct)}`,
        click: () => bind(acct),
    }));
}
