// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import clsx from "clsx";
import type { JSX } from "solid-js";
import { Show } from "solid-js";

import { CloseButton } from "./CloseButton";
import "./side-panel.scss";

export interface SidePanelProps {
    /** What the panel is, for assistive technology ("Task 12"). */
    label: string;
    /** The panel's heading. */
    title: JSX.Element;
    /** Something before the heading, such as a task number. */
    lead?: JSX.Element;
    /** Close it. The head's close button calls this. */
    onClose: () => void;
    /**
     * Beside the content instead of over it: a column of its own in a row layout, with a width of its own
     * (`--side-panel-width`, 320px), no shadow, and the content's flow left alone.
     */
    docked?: boolean;
    class?: string;
    children?: JSX.Element;
}

/**
 * A panel along the right edge of the pane it is in, over the content, with a head (a lead, a heading and a
 * close button) and a body that scrolls by itself. The Kanban's task drawer is one. It is positioned against the
 * nearest positioned ancestor, which is the pane's view; `docked` makes it a flex child beside the content.
 */
export function SidePanel(props: SidePanelProps): JSX.Element {
    return (
        <aside class={clsx("ui-side-panel", props.docked && "ui-side-panel--docked", props.class)} aria-label={props.label}>
            <header class="ui-side-panel__head">
                <Show when={props.lead}>{(lead) => lead()}</Show>
                <h3 class="ui-side-panel__title">{props.title}</h3>
                <CloseButton onClick={props.onClose} />
            </header>
            {props.children}
        </aside>
    );
}
