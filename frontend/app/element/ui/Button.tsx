// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import clsx from "clsx";
import { type JSX, Show, splitProps } from "solid-js";
import { Tooltip } from "../tooltip";
import { densityClass, toneClass, UiIcon, type UiDensity, type UiTone } from "./shared";

import "./ui.scss";

/**
 * `row`: a full-width list row (a place, a menu entry, a list item); pressed
 * marks the selected one. `link`: inline text that acts, such as "Show
 * details". Both default to the `quiet` tone and keep the kit's hover, focus
 * and disabled states.
 */
export type ButtonVariant = "row" | "link";

export interface ButtonProps extends Omit<JSX.ButtonHTMLAttributes<HTMLButtonElement>, "class" | "classList"> {
    /** Default `neutral` (`quiet` for a `row` or `link`). Use `accent` for the one main action in a group. */
    tone?: UiTone;
    variant?: ButtonVariant;
    density?: UiDensity;
    /** Font Awesome icon name, shown before the label. */
    icon?: string;
    /** Shows a spinner in place of the icon and disables the button. */
    busy?: boolean;
    /** For a toggle button: renders `aria-pressed` and the pressed look. */
    pressed?: boolean;
    class?: string;
}

/**
 * The line-style button. It never fills: tone shows as text and line
 * colour, hover and pressed as a stronger line and a faint tint.
 */
export function Button(props: ButtonProps): JSX.Element {
    const [local, rest] = splitProps(props, ["tone", "variant", "density", "icon", "busy", "pressed", "class", "children", "disabled", "type"]);
    return (
        <button
            {...rest}
            type={local.type ?? "button"}
            class={clsx(
                "ui-button",
                toneClass(local.tone ?? (local.variant ? "quiet" : undefined)),
                local.variant && `ui-button--${local.variant}`,
                densityClass(local.density),
                local.class
            )}
            disabled={local.disabled || local.busy}
            aria-busy={local.busy ? "true" : undefined}
            aria-pressed={local.pressed === undefined ? undefined : local.pressed ? "true" : "false"}
        >
            <Show when={local.busy} fallback={<Show when={local.icon}>{(icon) => <UiIcon name={icon()} />}</Show>}>
                <UiIcon name="spinner" spin />
            </Show>
            {local.children}
        </button>
    );
}

export interface IconButtonProps extends Omit<ButtonProps, "children" | "icon" | "busy"> {
    icon: string;
    /** Required: becomes the accessible name and the tooltip. */
    label: string;
    /** Default true. Set false where a surrounding element already explains it. */
    tooltip?: boolean;
    tooltipPlacement?: "top" | "bottom" | "left" | "right";
}

/** A square, icon-only line button. Defaults to the `quiet` tone. */
export function IconButton(props: IconButtonProps): JSX.Element {
    const [local, rest] = splitProps(props, ["icon", "label", "tooltip", "tooltipPlacement", "tone", "class"]);
    const button = () => (
        <Button
            {...rest}
            tone={local.tone ?? "quiet"}
            class={clsx("ui-icon-button", local.class)}
            icon={local.icon}
            aria-label={local.label}
        />
    );
    return (
        <Show when={local.tooltip !== false} fallback={button()}>
            <Tooltip content={local.label} placement={local.tooltipPlacement ?? "top"} divClassName="ui-tooltip-anchor">
                {button()}
            </Tooltip>
        </Show>
    );
}
