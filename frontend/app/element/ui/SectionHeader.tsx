// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import clsx from "clsx";
import { type JSX, Show } from "solid-js";
import { Dynamic } from "solid-js/web";

import "./card.scss";

export interface SectionHeaderProps {
    /** `title`: a pane's or section's name, with a count and its controls after it. `group`: a small-caps label over a group of cards. */
    variant?: "title" | "group";
    /** The heading's level; default 2 for `title`, 3 for `group`. */
    level?: 2 | 3 | 4;
    /** How many items the section lists, after the title. `title` only. */
    count?: number;
    class?: string;
    /** The label. A `title` header's children come after it instead: a filter, buttons. */
    title: JSX.Element;
    /** Controls after the title and count (a filter, Refresh, Add). `title` only. */
    children?: JSX.Element;
}

/**
 * The header of a pane or of a group inside it. `title` is the row at the top
 * of a Connectors-style pane: the name, how many it lists, then a filter and
 * the actions. `group` is the small label over a group of cards (Connected,
 * Your servers). Both are real headings, so a reader can move between them.
 */
export function SectionHeader(props: SectionHeaderProps): JSX.Element {
    const group = () => props.variant === "group";
    const tag = () => `h${props.level ?? (group() ? 3 : 2)}`;
    return (
        <Show
            when={!group()}
            fallback={
                <Dynamic component={tag()} class={clsx("ui-card-group-heading", props.class)}>
                    {props.title}
                </Dynamic>
            }
        >
            <div class={clsx("ui-card-section-header", props.class)}>
                <Dynamic component={tag()} class="ui-card-section-title">
                    {props.title}
                </Dynamic>
                <Show when={props.count !== undefined}>
                    <span class="ui-card-section-count">{props.count}</span>
                </Show>
                {props.children}
            </div>
        </Show>
    );
}
