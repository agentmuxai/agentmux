// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// One section of the Vault's list (vault-list.tsx): a box in the section's colour, its head (the name,
// how many it holds, what it offers) and its rows. The look is the Kanban column's
// (element/ui/_color-section.scss), at the app's normal text sizes.

import { Show, type JSX } from "solid-js";

import { Button } from "@/app/element/ui";
import { SECTION_LABELS, type VaultPaneSection } from "./vault-kinds";

export interface VaultSectionBoxProps {
    id: VaultPaneSection;
    /** What the head says after the name: how many rows, or how many of them are shown. None while unknown. */
    count?: string;
    open: boolean;
    onToggle: () => void;
    /** The head's own controls, after the count (Add). */
    actions?: JSX.Element;
    /** No rows to show: the section is its head alone. */
    empty?: boolean;
    children: JSX.Element;
}

export function VaultSectionBox(props: VaultSectionBoxProps): JSX.Element {
    const label = () => SECTION_LABELS[props.id];
    return (
        <section
            class={`vault-section vault-section--${props.id}`}
            classList={{ "vault-section--folded": !props.open, "vault-section--empty": !!props.empty }}
            aria-label={label()}
            data-section={props.id}
        >
            <div class="vault-section__head">
                <Button
                    density="compact"
                    tone="quiet"
                    icon={props.open ? "chevron-down" : "chevron-right"}
                    class="vault-section__toggle"
                    aria-expanded={props.open}
                    onClick={() => props.onToggle()}
                >
                    <span class="vault-section__label">{label()}</span>
                    <Show when={props.count}>
                        <span class="vault-section__count">{props.count}</span>
                    </Show>
                </Button>
                {props.actions}
            </div>
            {/* Kept mounted while folded: a row's open dialog and a loaded list survive a fold. */}
            <div class="vault-section__body" hidden={!props.open || !!props.empty}>
                {props.children}
            </div>
        </section>
    );
}
