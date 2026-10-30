// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import clsx from "clsx";
import { type JSX, Show } from "solid-js";
import { Button } from "./Button";
import { UiIcon } from "./shared";

import "./pane-parts.scss";

/** What a notice says: `accent` (information), `warning`, `danger` (an error) or `success`. */
export type NoticeTone = "accent" | "warning" | "danger" | "success";

export interface NoticeProps {
    /** Default `accent`. */
    tone?: NoticeTone;
    /** A bold first line above the text. */
    title?: JSX.Element;
    /** A Font Awesome solid icon name, shown before the text. Decorative: the words carry the meaning. */
    icon?: string;
    /** Buttons at the end of the box. */
    actions?: JSX.Element;
    /** Shows a quiet Dismiss button after `actions`. */
    onDismiss?: () => void;
    class?: string;
    children?: JSX.Element;
}

/**
 * A message box inside a pane: a faint tint of its tone, a 1px line around
 * it, and the main text colour (`line.notice`; the theme's semantic colours
 * are not tuned for small text on a wash of themselves, the same reason as
 * `Pill`). A `danger` notice is an alert and is announced at once; the others
 * are status messages, read when the reader is next free.
 */
export function Notice(props: NoticeProps): JSX.Element {
    const tone = () => props.tone ?? "accent";
    return (
        <div
            class={clsx("ui-notice", `ui-notice--${tone()}`, props.class)}
            role={tone() === "danger" ? "alert" : "status"}
        >
            <Show when={props.icon}>{(icon) => <UiIcon name={icon()} />}</Show>
            <div class="ui-notice-body">
                <Show when={props.title}>
                    <strong class="ui-notice-title">{props.title}</strong>
                </Show>
                {props.children}
            </div>
            <Show when={props.actions || props.onDismiss}>
                <div class="ui-notice-actions">
                    {props.actions}
                    <Show when={props.onDismiss}>
                        {(dismiss) => (
                            <Button tone="quiet" onClick={() => dismiss()()}>
                                Dismiss
                            </Button>
                        )}
                    </Show>
                </div>
            </Show>
        </div>
    );
}
