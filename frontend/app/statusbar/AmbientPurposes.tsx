// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The "AgentMux internal" part of the token popover: one line per purpose of AgentMux's own model
// calls (name, a failing mark, the total), opened to the six counts. The popover is narrow, so the
// table the Ambient section had is a line per purpose.
// docs/specs/SPEC_WARDEN_SPLIT_INTO_VAULT_HISTORY_AUDIT_2026_10_28.md §6.

import { createSignal, For, onCleanup, onMount, Show, type JSX } from "solid-js";
import { Button } from "@/app/element/ui";
import { AMBIENT_OPEN_MS, ambientAskFailed, ambientSummary, watchAmbient } from "@/app/store/ambient-health";
import type { AmbientPurposeRow } from "@/app/store/ambient-outcomes";

const BUCKETS = ["accepted", "kept", "refused", "skipped", "failed"] as const;

function PurposeLine(props: { row: AmbientPurposeRow }): JSX.Element {
    const [open, setOpen] = createSignal(false);
    return (
        <div class="ambient-purpose" data-state={props.row.unhealthy ? "failing" : undefined} title={props.row.detail}>
            <Button
                tone="quiet"
                density="compact"
                class="ambient-purpose-head"
                icon={open() ? "chevron-down" : "chevron-right"}
                aria-expanded={open()}
                onClick={() => setOpen(!open())}
            >
                <span class="ambient-purpose-name">{props.row.name}</span>
                <Show when={props.row.counts.failed > 0}>
                    <span class="ambient-purpose-failed">{props.row.counts.failed} failed</span>
                </Show>
                <span class="ambient-purpose-total">{props.row.total}</span>
            </Button>
            <Show when={open()}>
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

/** The purposes, asking every few seconds while it is on screen. */
export function AmbientPurposes(): JSX.Element {
    onMount(() => onCleanup(watchAmbient(AMBIENT_OPEN_MS)));
    return (
        <div class="ambient-purposes" data-testid="ambient-purposes">
            <Show when={ambientAskFailed()}>
                <div class="ambient-purposes-note">Could not read the counts; showing the last ones.</div>
            </Show>
            <Show
                when={ambientSummary()}
                fallback={<div class="ambient-purposes-note">No calls have ended since AgentMux's server started.</div>}
            >
                {(s) => <For each={s().rows}>{(row) => <PurposeLine row={row} />}</For>}
            </Show>
            <div class="ambient-purposes-note">
                Requests include those that never reached the model. A purpose is failing when its refusals and
                failures outnumber its accepted and kept answers. Counts restart with AgentMux's server.
            </div>
        </div>
    );
}
