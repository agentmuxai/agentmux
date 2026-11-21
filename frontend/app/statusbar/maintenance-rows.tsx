// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// How the Maintenance section draws a row: one component for a message with its buttons, used by every state
// (maintenance-table.ts lists them). The message is one block (its glyph and its text) and the buttons are another
// under it. When both fit on one line they share it; when they do not, the buttons go to a row of their own under
// the message, which then has the section's whole width. One rule for every state, decided by what fits, not by which
// state it is (_maintenance-section.scss).

import { For, Index, Show, type JSX } from "solid-js";

import { tip } from "@/app/element/tip";
import { Button } from "@/app/element/ui";
import type { MaintenanceState } from "./maintenance-state";
import type { RowDef, RowSpec } from "./maintenance-table";

/** One message and its buttons. `spec` is read again when the state changes, so a button keeps its place (and focus). */
export function MessageRow(props: { spec: () => RowSpec }): JSX.Element {
    const spec = props.spec;
    return (
        <div class="maintenance-row">
            <div class="maintenance-message">
                <Show when={spec().icon}>
                    {(icon) => (
                        <span class={`maintenance-icon maintenance-icon--${icon().tone}`} aria-hidden="true">
                            {icon().glyph}
                        </span>
                    )}
                </Show>
                <span class={`maintenance-row-text${spec().tone ? ` maintenance-row-text--${spec().tone}` : ""}`} {...tip(spec().tip)}>
                    {spec().text}
                </span>
            </div>
            <Show when={spec().actions.length > 0}>
                <div class="maintenance-actions">
                    <Index each={spec().actions}>
                        {(action) => (
                            <Button
                                density="compact"
                                tone={action().primary ? "accent" : undefined}
                                class={`maintenance-btn${action().primary ? " maintenance-btn--primary" : ""}`}
                                disabled={action().disabled}
                                onClick={() => action().onClick()}
                            >
                                {action().label}
                            </Button>
                        )}
                    </Index>
                </div>
            </Show>
        </div>
    );
}

/** The rows a list of definitions gives for the section's state, each drawn the same way. */
export function MessageRows(props: { m: MaintenanceState; defs: RowDef[] }): JSX.Element {
    return (
        <For each={props.defs}>
            {(def) => (
                <Index each={def.rows(props.m)}>{(spec) => <MessageRow spec={spec} />}</Index>
            )}
        </For>
    );
}
