// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The one list the status bar's summary opens (docs/specs/SPEC_STATUS_BAR_COMPACT_SUMMARY_AND_VAULT_REQUESTS_2026_11_15.md §2):
 * a section for each part that has rows (Asks, Browser, Vault, Tasks), each row naming who asks, what, how long, and one
 * button that goes to where it is answered. The part that was clicked comes first. It reuses the attention list's frame.
 */

import { AnchoredPopover, type PopoverAnchor } from "@/app/element/anchored-popover";
import { Button } from "@/app/element/ui";
import { useTick } from "@/app/hook/useTick";
import { isParkedItem } from "@/app/notification/attention-go";
import { attentionTint } from "@/app/notification/attention-list";
import { createMemo, For, Show, type Accessor, type JSX } from "solid-js";
import type { Summary, SummaryRow } from "./summary";
import type { PartId } from "./summary-kinds";
import { goesAheadText, rowButton, rowButtonLabel, SUMMARY_TEXT } from "./summary-text";
import { waitedFor } from "./waited-for";

export interface SummaryPopoverProps {
    anchor: PopoverAnchor;
    summary: Accessor<Summary>;
    /** The part that was clicked: its section is listed first. */
    first?: PartId;
    onClose: () => void;
    onOpenSwarm: () => void;
    /** What a row's button runs; the popover closes first. */
    go: (row: SummaryRow) => void;
}

const parkedRow = (row: SummaryRow): boolean => row.click.to === "pane" && isParkedItem(row.click.item);

function RowView(props: { row: SummaryRow; now: number; go: (row: SummaryRow) => void }): JSX.Element {
    const row = () => props.row;
    const anotherWindow = () => {
        const c = row().click;
        return c.to === "pane" && !c.item.local;
    };
    return (
        <li class="attention-list-row" data-part={row().part}>
            <div class="attention-list-text">
                <span class="attention-list-name" style={{ color: attentionTint(row().color) }}>
                    {row().name}
                </span>
                <span class="attention-list-what">
                    {row().what}
                    {anotherWindow() ? SUMMARY_TEXT.anotherWindow : ""}
                </span>
                <Show when={row().goesAheadAtMs}>
                    {(at) => <span class="attention-list-what">{goesAheadText(at(), props.now)}</span>}
                </Show>
            </div>
            <Show when={row().since > 0}>
                <span class="attention-list-age">{waitedFor(row().since, props.now)}</span>
            </Show>
            <Button
                density="compact"
                aria-label={rowButtonLabel(row().click, row().name, parkedRow(row()))}
                onClick={() => props.go(row())}
            >
                {rowButton(row().click, parkedRow(row()))}
            </Button>
        </li>
    );
}

export const SummaryPopover = (props: SummaryPopoverProps): JSX.Element => {
    const tick = useTick(1_000);
    const now = createMemo(() => (tick(), Date.now()));
    // Sections that have rows, the clicked part first.
    const sections = createMemo(() => {
        const parts = props.summary().parts.filter((p) => p.rows.length > 0);
        return [...parts.filter((p) => p.id === props.first), ...parts.filter((p) => p.id !== props.first)];
    });
    const go = (row: SummaryRow) => {
        props.onClose();
        props.go(row);
    };
    return (
        <AnchoredPopover
            anchor={props.anchor}
            placement="top-end"
            onDismiss={props.onClose}
            class="attention-list"
            role="dialog"
            aria-label={SUMMARY_TEXT.title}
            style={{ width: "360px" }}
        >
            <div class="attention-list-header">
                <span class="attention-list-title">{SUMMARY_TEXT.title}</span>
                <span class="attention-list-count">{props.summary().waiting}</span>
            </div>
            <Show when={sections().length > 0} fallback={<div class="attention-list-empty">{SUMMARY_TEXT.empty}</div>}>
                <div class="attention-list-sections">
                    <For each={sections()}>
                        {(part) => (
                            <section class="attention-list-section" aria-label={SUMMARY_TEXT.section[part.id]}>
                                <h3 class="attention-list-section-title">{SUMMARY_TEXT.section[part.id]}</h3>
                                <ul class="attention-list-rows">
                                    <For each={part.rows}>{(row) => <RowView row={row} now={now()} go={go} />}</For>
                                </ul>
                            </section>
                        )}
                    </For>
                </div>
            </Show>
            <div class="attention-list-footer">
                <Button
                    density="compact"
                    onClick={() => {
                        props.onClose();
                        props.onOpenSwarm();
                    }}
                >
                    {SUMMARY_TEXT.swarm}
                </Button>
            </div>
        </AnchoredPopover>
    );
};

SummaryPopover.displayName = "SummaryPopover";
