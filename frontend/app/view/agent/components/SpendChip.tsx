// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * "$4.20 of $50 today": what an agent has used through AgentMux's model gateway (a model route or an API key) against
 * its daily cap, on its Swarm card. Nothing for an agent that has none (a login agent, or one srv has not written a
 * figure for today). Given the pane's block id, the chip is a button: a click opens the Spend cap panel, where a person
 * sets the cap in dollars, removes it, or takes the default (the same panel as My Agents' "Spend cap…").
 */
import { createSignal, Show, type JSX } from "solid-js";
import { AnchoredPopover } from "@/app/element/anchored-popover";
import { Button } from "@/app/element/ui";
import { isRouteCredits } from "@/app/store/agent-unavailable";
import { Tag } from "../../../element/ui/Tag";
import { readSpendToday, spendChipText, spendChipTitle, spendChipTone } from "../spend-stopped";
import { AgentSpendPanel } from "./AgentSpendPanel";

export interface SpendChipProps {
    meta: Record<string, any> | undefined | null;
    now?: () => number;
    /** The agent's pane: with it the chip opens the Spend cap panel. Without it the chip only shows the figure. */
    blockId?: string | null;
    agentName?: string;
}

export function SpendChip(props: SpendChipProps): JSX.Element {
    const today = () => readSpendToday(props.meta, props.now?.());
    return (
        <Show when={today()}>
            {(t) => (
                <div class="spend-chip-line">
                    <Show when={props.blockId} fallback={<Chip t={t()} meta={props.meta} editable={false} />}>
                        {(blockId) => <EditableChip t={t()} meta={props.meta} blockId={blockId()} agentName={props.agentName ?? ""} />}
                    </Show>
                </div>
            )}
        </Show>
    );
}
SpendChip.displayName = "SpendChip";

type Today = NonNullable<ReturnType<typeof readSpendToday>>;

function Chip(props: { t: Today; meta: SpendChipProps["meta"]; editable: boolean }): JSX.Element {
    return (
        <Tag tone={spendChipTone(props.t)} title={spendChipTitle(props.t, isRouteCredits(props.meta), props.editable)}>
            <span data-testid="spend-chip">{spendChipText(props.t)}</span>
        </Tag>
    );
}

function EditableChip(props: { t: Today; meta: SpendChipProps["meta"]; blockId: string; agentName: string }): JSX.Element {
    const [open, setOpen] = createSignal(false);
    const [anchor, setAnchor] = createSignal<HTMLElement>();
    return (
        <>
            <Button
                variant="chrome"
                class="spend-chip-button"
                ref={setAnchor}
                aria-haspopup="dialog"
                aria-expanded={open()}
                data-testid="spend-chip-button"
                onClick={(e) => {
                    // The card under the chip selects its pane on a click; this one is only about the cap.
                    e.stopPropagation();
                    setOpen((o) => !o);
                }}
            >
                <Chip t={props.t} meta={props.meta} editable />
            </Button>
            <Show when={open()}>
                <AnchoredPopover
                    anchor={anchor()}
                    placement="bottom-start"
                    shellClass="anchored-popover--menu"
                    class="menu spend-chip-popover"
                    role="dialog"
                    aria-label="Spend cap"
                    onDismiss={() => setOpen(false)}
                >
                    <AgentSpendPanel blockId={props.blockId} agentName={props.agentName} onClose={() => setOpen(false)} />
                </AnchoredPopover>
            </Show>
        </>
    );
}
