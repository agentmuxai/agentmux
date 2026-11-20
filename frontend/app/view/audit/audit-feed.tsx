// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Audit's rows: one list for a channel's entries, or one section per channel when the list shows every
// channel on this computer (and, on the LAN scope, those of the paired computers).

import { createMemo, For, Show, type JSX } from "solid-js";

import type { AuditEntry } from "@/types/rpc/AuditEntry";
import type { AuditPeerRows } from "@/types/rpc/AuditPeerRows";
import type { createAuditSounds } from "./audit-sound";
import { AuditRow } from "./audit-row";
import { sectionsOf } from "./audit-shared";

const sameKeys = (a: string[], b: string[]) => a.length === b.length && a.every((k, i) => k === b[i]);

export const AuditFeed = (props: {
    entries: AuditEntry[];
    /** One section per channel that wrote them. */
    sectioned: boolean;
    /** The channel that answered: what an entry with no channel of its own belongs to. */
    channel?: string;
    /** The paired computers asked for this page: each one's channels are sections of their own, and one that gave no rows says why. */
    peers?: readonly AuditPeerRows[];
    now: number;
    paneBlockId: string;
    /** Which rows show a speaker and which one is lit. */
    sounds: ReturnType<typeof createAuditSounds>;
}): JSX.Element => {
    const rows = (entries: () => AuditEntry[]) => (
        <ul class="audit-feed">
            <For each={entries()}>
                {(entry) => <AuditRow entry={entry} now={props.now} paneBlockId={props.paneBlockId} sound={props.sounds.badge(entry)} lit={props.sounds.lit() === entry.id} />}
            </For>
        </ul>
    );
    const sections = createMemo(() => sectionsOf(props.entries, props.channel, new Set((props.peers ?? []).map((p) => p.host))));
    // A computer that gave no rows says why, so a missing section is not read as "nothing happened there".
    const notes = () => (props.peers ?? []).filter((p) => p.note).map((p) => `${p.host}: ${p.note}`);
    // Keyed by channel, so a new entry redraws its own section's rows and no other.
    const keys = createMemo(() => sections().map((s) => s.key), [], { equals: sameKeys });

    return (
        <Show when={props.sectioned} fallback={rows(() => props.entries)}>
            <Show when={notes().length > 0}>
                <div class="warden-manager-summary">Not listed: {notes().join("; ")}</div>
            </Show>
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
