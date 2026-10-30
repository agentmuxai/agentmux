// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import clsx from "clsx";
import { type JSX, Show } from "solid-js";
import { UiIcon } from "./shared";

import "./pane-parts.scss";

export interface EmptyStateProps {
    /** A bold first line: "No remotes yet". The text goes in the children. */
    title?: JSX.Element;
    /** A Font Awesome solid icon name, above or before the text. Decorative. */
    icon?: string;
    /** `loading`: shows a spinner and reads as a status that is still changing. */
    loading?: boolean;
    /**
     * `inline` (the default) is a line or two of quiet text at the top of
     * where the list would be. `fill` centres the message in all the room its
     * parent has, for a pane or a column that has nothing else to show.
     */
    variant?: "inline" | "fill";
    /** What the reader can do about it: a button or two. */
    actions?: JSX.Element;
    class?: string;
    children?: JSX.Element;
}

/**
 * What a list, column or pane shows when it has nothing to list: it is still
 * loading, nothing is there yet, or a filter matched nothing. One look, so
 * "nothing here" reads the same in every pane.
 */
export function EmptyState(props: EmptyStateProps): JSX.Element {
    return (
        <div
            class={clsx("ui-empty-state", `ui-empty-state--${props.variant ?? "inline"}`, props.class)}
            role="status"
            aria-busy={props.loading ? "true" : undefined}
        >
            <Show when={props.loading || props.icon}>
                <UiIcon name={props.loading ? "spinner" : props.icon!} spin={props.loading} />
            </Show>
            <Show when={props.title}>
                <strong class="ui-empty-state-title">{props.title}</strong>
            </Show>
            <Show when={props.children}>
                <div class="ui-empty-state-text">{props.children}</div>
            </Show>
            <Show when={props.actions}>
                <div class="ui-empty-state-actions">{props.actions}</div>
            </Show>
        </div>
    );
}
