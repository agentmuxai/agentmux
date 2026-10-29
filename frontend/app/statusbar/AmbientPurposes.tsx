// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The "AgentMux internal" part of the token popover: one line per purpose of AgentMux's own model
// calls (name, a failing mark, the total), opened to the six counts. The popover is narrow, so the
// table the Ambient section had is a line per purpose.
// docs/specs/SPEC_WARDEN_SPLIT_INTO_VAULT_HISTORY_AUDIT_2026_10_28.md §6.

import { createSignal, For, Index, Show, type JSX } from "solid-js";
import { Button } from "@/app/element/ui";
import { ambientAskFailed, ambientSummary } from "@/app/store/ambient-health";
import type { AmbientPurposeRow } from "@/app/store/ambient-outcomes";

const BUCKETS = ["accepted", "kept", "refused", "skipped", "failed"] as const;

function PurposeLine(props: { row: AmbientPurposeRow; open: boolean; onToggle: () => void }): JSX.Element {
    return (
        <div class="ambient-purpose" data-state={props.row.unhealthy ? "failing" : undefined} title={props.row.detail}>
            <Button
                tone="quiet"
                density="compact"
                class="ambient-purpose-head"
                icon={props.open ? "chevron-down" : "chevron-right"}
                aria-expanded={props.open}
                onClick={props.onToggle}
            >
                <span class="ambient-purpose-name">{props.row.name}</span>
                <Show when={props.row.counts.failed > 0 || props.row.unhealthy}>
                    <span class="ambient-purpose-failed">
                        {props.row.counts.failed > 0 ? `${props.row.counts.failed} failed` : `${props.row.counts.refused} refused`}
                    </span>
                </Show>
                <span class="ambient-purpose-total">{props.row.total}</span>
            </Button>
            <Show when={props.open}>
                <div class="ambient-purpose-counts">
                    <div class="ambient-purpose-count">
                        <span>Requests</span>
                        <span>{props.row.total}</span>
                    </div>
                    <For each={BUCKETS}>
                        {(bucket) => (
                            <div class="ambient-purpose-count">
                                <span>{bucket[0].toUpperCase() + bucket.slice(1)}</span>
                                <span>{props.row.counts[bucket]}</span>
                            </div>
                        )}
                    </For>
                </div>
            </Show>
        </div>
    );
}

/** The purposes. The popover asks for the counts (`TokenBreakdownPopover`), so they keep arriving
 *  whether or not this part is on screen. */
export function AmbientPurposes(): JSX.Element {
    // Which purposes are open, kept here: every ask replaces the row objects, and a row's own state
    // would close under the reader.
    const [opened, setOpened] = createSignal<ReadonlySet<string>>(new Set());
    const toggle = (purpose: string) =>
        setOpened((prev) => {
            const next = new Set(prev);
            if (!next.delete(purpose)) next.add(purpose);
            return next;
        });
    return (
        <div class="ambient-purposes" data-testid="ambient-purposes">
            <Show when={ambientAskFailed()}>
                <div class="ambient-purposes-note">Could not read the counts; showing the last ones.</div>
            </Show>
            <Show
                when={ambientSummary()}
                fallback={<div class="ambient-purposes-note">No calls have ended since AgentMux's server started.</div>}
            >
                {(s) => (
                    <Index each={s().rows}>
                        {(row) => (
                            <PurposeLine
                                row={row()}
                                open={opened().has(row().purpose)}
                                onToggle={() => toggle(row().purpose)}
                            />
                        )}
                    </Index>
                )}
            </Show>
            <div class="ambient-purposes-note">
                Requests include those that never reached the model. A purpose is failing when its refusals and
                failures outnumber its accepted and kept answers. Counts restart with AgentMux's server.
            </div>
        </div>
    );
}
