// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// One row of the Audit: when, what kind, what happened, what it cost, and the buttons that open the agents
// it is about (their history at that moment, their live pane).

import { For, Show, type JSX } from "solid-js";

import { Button, Pill } from "@/app/element/ui";
import { revealBlock } from "@/app/util/reveal-block";
import { openOrFocusHistoryTab } from "@/app/view/agent/open-history-tab";
import { makeHistoryAt } from "@/app/view/agent/history/history-start-at";
import { ageMs, formatAge } from "@/app/view/warden-shared/warden-shared";
import { resolveHistoryTarget } from "./audit-history-target";
import type { AuditEntry } from "@/types/rpc/AuditEntry";
import { costLabel, entryLinks, isFailure, KIND_CHIPS, type AuditLinkItem } from "./audit-shared";

const kindLabel = (kind: string): string => KIND_CHIPS.find((c) => c.kind === kind)?.label.replace(/s$/, "") ?? kind;

export const AuditRow = (props: { entry: AuditEntry; now: number; paneBlockId: string }): JSX.Element => {
    const failed = () => isFailure(props.entry);

    const open = async (item: AuditLinkItem) => {
        if (item.kind === "pane") return void revealBlock(item.blockId, { source: "audit" });
        const { link } = item;
        // The reader parses the transcript by the agent's provider, and a time-only link is in whichever
        // conversation that time falls in: neither is on the link.
        const target = await resolveHistoryTarget(link);
        await openOrFocusHistoryTab({
            currentBlockId: props.paneBlockId,
            agentId: target.agentId,
            agentName: target.agentName,
            provider: target.provider,
            at: makeHistoryAt({ at_ms: link.at_ms, at_byte: link.at_byte, zone: target.zone }),
        });
    };

    return (
        <li class="audit-row" data-variant={failed() ? "err" : "ok"} data-kind={props.entry.kind}>
            <span class="audit-row-time" title={new Date(props.entry.ts_ms).toLocaleString()}>
                {formatAge(ageMs(props.entry.ts_ms, props.now))} ago
            </span>
            <Pill tone={failed() ? "error" : "neutral"} class="audit-row-kind">
                {kindLabel(props.entry.kind)}
            </Pill>
            <span class="audit-row-summary">
                {props.entry.summary}
                <Show when={props.entry.reason}>
                    <span class="audit-row-reason"> · {props.entry.reason}</span>
                </Show>
            </span>
            <Show when={props.entry.cost}>{(cost) => <span class="audit-row-cost">{costLabel(cost())}</span>}</Show>
            <span class="audit-row-links">
                <For each={entryLinks(props.entry)}>
                    {(item) => (
                        <Button tone="quiet" density="compact" onClick={() => void open(item)}>
                            {item.label}
                        </Button>
                    )}
                </For>
            </span>
        </li>
    );
};

AuditRow.displayName = "AuditRow";
