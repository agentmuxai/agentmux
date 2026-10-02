// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The model the CLI ACTUALLY used for the last reply, next to the one the menu
 * selected (docs/specs/SPEC_RUNTIME_MENU_REMAINING_GAPS_2026_10_01.md §4.3, Stage A).
 *
 * The argv carries an alias (`sonnet`); the CLI resolves it against the account
 * and the provider, and a resolution can differ from what the menu implies (the
 * `sonnet` row read "Sonnet 5.5" while CLI 2.1.280 resolved it to Sonnet 5). The
 * stream reports the resolved id on every main-agent reply, so the menu can say
 * so. This is deliberately conservative: it flags only a different model FAMILY,
 * only when the process was spawned with the selection, and it says what it is
 * (the LAST reply) so a change made since then does not read as a fault.
 */

import type { RuntimeAgreement } from "./process-runtime";

export type ModelFamily = "opus" | "sonnet" | "haiku";

const FAMILIES: ModelFamily[] = ["opus", "sonnet", "haiku"];

/** The family of an alias (`sonnet`) or a concrete id (`claude-sonnet-5-5`); `undefined` for anything else. */
export function modelFamily(model: string | null | undefined): ModelFamily | undefined {
    if (!model) return undefined;
    const m = model.toLowerCase();
    const found = FAMILIES.filter((f) => m.includes(f));
    // "claude-opus-4-sonnet-…" would be two families: no claim.
    return found.length === 1 ? found[0] : undefined;
}

export interface LastReplyModel {
    /** One line for the menu. */
    text: string;
    /** The reply's model is in a different family than the selection. */
    differs: boolean;
}

/**
 * What to say about the last reply's model, or `null` for nothing.
 *
 * @param selected   the model the menu shows (alias or id)
 * @param reported   the resolved id on the last main-agent reply
 * @param agreement  whether the process was spawned with the selection
 */
export function lastReplyModel(
    selected: string,
    reported: string | null | undefined,
    agreement: RuntimeAgreement,
): LastReplyModel | null {
    if (!reported) return null;
    const base = `Last reply used ${reported}`;
    const want = modelFamily(selected);
    const got = modelFamily(reported);
    // Only a settled, agreeing process can be contradicted by a reply: while a
    // restart is pending or the process differs, an older reply is expected to differ.
    const judge = agreement.kind === "agrees" && want !== undefined && got !== undefined;
    if (judge && want !== got) {
        return {
            text: `${base}, not ${want}. If you changed the model since, the next reply will show it.`,
            differs: true,
        };
    }
    return { text: base, differs: false };
}
