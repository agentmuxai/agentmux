// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import clsx from "clsx";
import { type JSX, Show, splitProps } from "solid-js";

import { StatusDot, type StatusTone } from "./StatusDot";

import "./card.scss";

export interface CardProps extends Omit<JSX.HTMLAttributes<HTMLDivElement>, "title" | "class" | "onSelect"> {
    /** A logo or icon, about 20px. */
    icon?: JSX.Element;
    title: JSX.Element;
    subtitle?: JSX.Element;
    /** The status dot, top right. */
    status?: { tone: StatusTone; label: string };
    /** Footer line: short facts ("OAuth", "Used by 2 agents"), separated by dots. */
    meta?: JSX.Element[];
    /** Buttons shown on hover, focus and while open; clicks on them don't open the card. */
    actions?: JSX.Element;
    /** A count at the trailing edge of a chip. */
    badge?: JSX.Element;
    /** A colour edge on the left (a remote's colour). */
    edgeColor?: string;
    /** Open: its details are showing. */
    selected?: boolean;
    /** Dimmed content (a hidden remote). */
    muted?: boolean;
    /** Status that needs the user: a danger line. */
    attention?: boolean;
    /** `chip`: one line, for an Add gallery. Default `card`. */
    size?: "card" | "chip";
    /** Click, Enter or Space; Escape too, while open. */
    onSelect?: () => void;
    /** Double-click. */
    onOpen?: () => void;
    class?: string;
}

/**
 * The line-style card Connectors shows for each connected account, MCP
 * server and remote, and as chips in its Add galleries
 * (docs/specs/SPEC_CONNECTORS_CARDS_2026_10_13.md §3.2). The whole card is
 * one button; its actions are separate buttons inside it.
 */
export function Card(props: CardProps): JSX.Element {
    const [local, rest] = splitProps(props, [
        "icon",
        "title",
        "subtitle",
        "status",
        "meta",
        "actions",
        "badge",
        "edgeColor",
        "selected",
        "muted",
        "attention",
        "size",
        "onSelect",
        "onOpen",
        "class",
        "style",
    ]);
    const chip = () => local.size === "chip";
    const onKeyDown = (e: KeyboardEvent) => {
        // Only the card's own keys: Enter in a field or on an action is theirs.
        if (e.target !== e.currentTarget) return;
        if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            local.onSelect?.();
        } else if (e.key === "Escape" && local.selected) {
            e.preventDefault();
            e.stopPropagation();
            local.onSelect?.();
        }
    };
    const stop = (e: Event) => e.stopPropagation();
    return (
        <div
            role="button"
            tabIndex={0}
            aria-expanded={chip() ? undefined : local.selected ? "true" : "false"}
            {...rest}
            class={clsx(
                "ui-card",
                chip() && "ui-card--chip",
                local.selected && "is-selected",
                local.muted && "is-muted",
                local.attention && "is-attention",
                local.edgeColor && "has-edge",
                local.class,
            )}
            style={local.edgeColor ? { "--ui-card-edge": local.edgeColor } : undefined}
            onClick={() => local.onSelect?.()}
            onDblClick={() => local.onOpen?.()}
            onKeyDown={onKeyDown}
        >
            <Show when={local.icon}>
                <span class="ui-card-icon" aria-hidden="true">
                    {local.icon}
                </span>
            </Show>
            <span class="ui-card-text">
                <span class="ui-card-title">{local.title}</span>
                <Show when={local.subtitle}>
                    <span class="ui-card-subtitle">{local.subtitle}</span>
                </Show>
                <Show when={!chip() && local.meta && local.meta.length > 0}>
                    <span class="ui-card-meta">{local.meta}</span>
                </Show>
            </span>
            <Show when={local.status}>
                {(s) => <StatusDot class="ui-card-status" tone={s().tone} label={s().label} />}
            </Show>
            <Show when={local.badge}>
                <span class="ui-card-badge">{local.badge}</span>
            </Show>
            <Show when={local.actions}>
                <span class="ui-card-actions" onClick={stop} onDblClick={stop} onKeyDown={stop}>
                    {local.actions}
                </span>
            </Show>
        </div>
    );
}
