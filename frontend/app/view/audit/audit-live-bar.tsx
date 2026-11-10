// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The bar that stays at the top of the Audit: Pause / Resume, the "N new" pill, the totals, and the last sound
// that played (who made it), with a button to that agent's pane.

import { Show, type JSX } from "solid-js";

import { Button } from "@/app/element/ui";
import { revealBlock } from "@/app/util/reveal-block";
import type { AuditTotals } from "@/types/rpc/AuditTotals";
import { newPillLabel, totalsLabel } from "./audit-shared";

export const AuditLiveBar = (props: {
    paused: boolean;
    /** Pause, or (paused) follow the newest entries again. */
    onTogglePause: () => void;
    newCount: number;
    onJumpToNewest: () => void;
    totals: AuditTotals | null;
    /** The last sound that played and the pane of the agent that made it. */
    lastSound: { text: string; blockId: string } | null;
}): JSX.Element => (
    <div class="audit-live-bar">
        <Button tone="quiet" density="compact" icon={props.paused ? "play" : "pause"} pressed={props.paused} onClick={props.onTogglePause}>
            {props.paused ? "Resume" : "Pause"}
        </Button>
        <Show when={props.newCount > 0}>
            <Button tone="accent" density="compact" icon="arrow-up" onClick={props.onJumpToNewest}>
                {newPillLabel(props.newCount)}
            </Button>
        </Show>
        <Show when={props.totals}>{(t) => <span class="audit-totals">{totalsLabel(t())}</span>}</Show>
        <Show when={props.lastSound}>
            {(last) => (
                <Button tone="quiet" density="compact" onClick={() => revealBlock(last().blockId, { source: "audit" })}>
                    {last().text}
                </Button>
            )}
        </Show>
    </div>
);

AuditLiveBar.displayName = "AuditLiveBar";
