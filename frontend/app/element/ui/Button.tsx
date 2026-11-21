// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import clsx from "clsx";
import { type JSX, Show, splitProps } from "solid-js";
import { t } from "@/app/i18n/i18n";
import { Tooltip } from "../tooltip";
import { createPendingPress, type PendingMode } from "./pending-press";
import { densityClass, toneClass, UiIcon, type UiDensity, type UiTone } from "./shared";

import "./ui.scss";

/**
 * `row`: a full-width list row (a place, a menu entry, a list item); pressed
 * marks the selected one. `link`: inline text that acts, such as "Show
 * details". Both default to the `quiet` tone and keep the kit's hover, focus
 * and disabled states. `chrome`: an icon in a pane's header or a search bar,
 * which takes its colour and size from where it sits and shows only opacity
 * for hover and disabled (and the accent colour when pressed); it carries none
 * of the kit's line, tone or density classes.
 */
export type ButtonVariant = "row" | "link" | "chrome";

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
    /**
     * What the button does while its click handler's promise is unsettled (see `PendingMode`). A toggle (`pressed`
     * set) never goes pending.
     */
    pending?: PendingMode;
    class?: string;
}

/**
 * The line-style button. It never fills: tone shows as text and line
 * colour, hover and pressed as a stronger line and a faint tint.
 *
 * It answers a press at once (the pressed look, in the frame of the mouse-down) and, when its click handler returns a
 * promise, locks until the promise settles and shows a spinner if that takes longer than
 * `PENDING_SPINNER_DELAY_MS` (SPEC_BUTTON_PRESSED_AND_PENDING_FEEDBACK_2026_11_20.md §4).
 */
export function Button(props: ButtonProps): JSX.Element {
    const [local, rest] = splitProps(props, [
        "tone",
        "variant",
        "density",
        "icon",
        "busy",
        "pressed",
        "pending",
        "class",
        "children",
        "disabled",
        "type",
        "onClick",
    ]);
    const press = createPendingPress(
        () => local.onClick,
        () => local.pending,
        () => local.pressed !== undefined
    );
    const { pending, spinning, slow } = press;

    // A caller's own `busy` keeps its look (a leading spinner); the automatic one sits over a label-only button so
    // the button keeps the width its label gave it, and replaces the icon of one that has an icon.
    const overlay = () => spinning() && !local.busy && !local.icon;
    return (
        <button
            {...rest}
            type={local.type ?? "button"}
            class={clsx(
                local.variant === "chrome"
                    ? ["ui-button--chrome", local.pressed !== undefined && "ui-button--toggle"]
                    : [
                          "ui-button",
                          toneClass(local.tone ?? (local.variant ? "quiet" : undefined)),
                          local.variant && `ui-button--${local.variant}`,
                          densityClass(local.density),
                      ],
                overlay() && "ui-button--spin-over",
                local.class
            )}
            onClick={press.click}
            disabled={local.disabled || local.busy}
            aria-busy={local.busy || pending() ? "true" : undefined}
            data-pending={pending() ? "true" : undefined}
            aria-pressed={local.pressed === undefined ? undefined : local.pressed ? "true" : "false"}
        >
            <Show when={local.busy || (spinning() && local.icon)} fallback={<Show when={local.icon}>{(icon) => <UiIcon name={icon()} />}</Show>}>
                <UiIcon name="spinner" spin />
            </Show>
            {local.children}
            <Show when={overlay()}>
                <span class="ui-button-spinner" aria-hidden="true">
                    <UiIcon name="spinner" spin />
                </span>
            </Show>
            <Show when={slow()}>
                <span class="ui-button-working" role="status">
                    {t("ui.button.working")}
                </span>
            </Show>
        </button>
    );
}

export interface IconButtonProps extends Omit<ButtonProps, "children" | "icon"> {
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
