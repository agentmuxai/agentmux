// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Audit's rows: one list for a channel's entries, or one section per channel when the list shows every
// channel on this computer.

import { createMemo, For, Show, type JSX } from "solid-js";

import type { AuditEntry } from "@/types/rpc/AuditEntry";
import { AuditRow } from "./audit-row";
import { sectionsOf } from "./audit-shared";

const sameKeys = (a: string[], b: string[]) => a.length === b.length && a.every((k, i) => k === b[i]);

export const AuditFeed = (props: {
    entries: AuditEntry[];
    /** One section per channel that wrote them. */
    sectioned: boolean;
    /** The channel that answered: what an entry with no channel of its own belongs to. */
    channel?: string;
    now: number;
    paneBlockId: string;
}): JSX.Element => {
    const rows = (entries: () => AuditEntry[]) => (
        <ul class="audit-feed">
            <For each={entries()}>{(entry) => <AuditRow entry={entry} now={props.now} paneBlockId={props.paneBlockId} />}</For>
        </ul>
    );
    const sections = createMemo(() => sectionsOf(props.entries, props.channel));
    // Keyed by channel, so a new entry redraws its own section's rows and no other.
    const keys = createMemo(() => sections().map((s) => s.key), [], { equals: sameKeys });

    return (
        <Show when={props.sectioned} fallback={rows(() => props.entries)}>
            <For each={keys()}>
                {(key) => {
                    const section = () => sections().find((s) => s.key === key);
                    return (
                        <section class="audit-section">
                            <h3 class="audit-section-label">{section()?.label}</h3>
                            {rows(() => section()?.entries ?? [])}
                        </section>
                    );
                }}
            </For>
        </Show>
    );
};

AuditFeed.displayName = "AuditFeed";
