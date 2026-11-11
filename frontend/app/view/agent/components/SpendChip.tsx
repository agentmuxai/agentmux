// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * "$4.20 of $20 today": what an agent on a model route has used against its daily cap, on its Swarm card and My Agents
 * row. Nothing for an agent that has none (a login agent, or one srv has not written a figure for today).
 */
import { Show, type JSX } from "solid-js";
import { Tag } from "../../../element/ui/Tag";
import { readSpendToday, spendChipText, spendChipTitle, spendChipTone } from "../spend-stopped";

export function SpendChip(props: { meta: Record<string, any> | undefined | null; now?: () => number }): JSX.Element {
    const today = () => readSpendToday(props.meta, props.now?.());
    return (
        <Show when={today()}>
            {(t) => (
                <div class="spend-chip-line">
                    <Tag tone={spendChipTone(t())} title={spendChipTitle(t())}>
                        <span data-testid="spend-chip">{spendChipText(t())}</span>
                    </Tag>
                </div>
            )}
        </Show>
    );
}
SpendChip.displayName = "SpendChip";
