// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * CloseButton — the one control that closes, dismisses or removes something: a small ✕ that
 * highlights red on hover. Every such control in the app is this component, so the red, the
 * keyboard focus ring and the accessible name are written once
 * (scripts/check-ui-primitives.mjs rule 11 counts the ✕ controls that are not).
 *
 * `label` is the accessible name and the tooltip ("Close", "Dismiss", "Remove <name>"). The hover
 * is the standard close red (`--window-close-*` in theme.scss, the one the Windows title bar and the
 * pane headers already use): a solid red tile with a white ✕, strong against every theme's surface
 * where a faint red wash would not show. It never changes the control's size or position.
 *
 * Sizes, as variants of the one control:
 *   `sm`      18px box, 10px ✕: a banner, a drawer head, a chip.
 *   `md`      26px box, 12px ✕ (default): a panel or a dialog's head.
 *   `tab`     16px box, 13px ✕: a tab's own close.
 *   `header`  26px box, 25px of font so the ✕ reads at the pane header's size.
 *
 * A host that shows the ✕ only on hover (a tab, a tile) sets its own `opacity` or `visibility`
 * on the class it passes; the control shows itself while it has keyboard focus.
 * Window controls that mimic the OS (macOS traffic lights, the Windows caption) keep their own
 * markup and the same hover red.
 */

import clsx from "clsx";
import { type JSX, Show, splitProps } from "solid-js";
import { Tooltip } from "../tooltip";
import "./close-button.scss";

export type CloseButtonSize = "sm" | "md" | "tab" | "header";

export interface CloseButtonProps extends Omit<JSX.ButtonHTMLAttributes<HTMLButtonElement>, "class" | "classList" | "children" | "aria-label"> {
    /** Words beside the ✕ ("Close menu"): the control is then as wide as its text, which is its accessible name. */
    text?: string;
    /** The accessible name and the tooltip. Default "Close"; say what it does when that helps ("Dismiss", "Remove <name>"). */
    label?: string;
    size?: CloseButtonSize;
    /** Shows a spinner in place of the ✕ and ignores clicks: the close is under way. */
    busy?: boolean;
    /** Default true. Set false where a surrounding element already explains it, or on a control inside another tooltip. */
    tooltip?: boolean;
    tooltipPlacement?: "top" | "bottom" | "left" | "right";
    class?: string;
}

export function CloseButton(props: CloseButtonProps): JSX.Element {
    const [local, rest] = splitProps(props, ["label", "text", "size", "busy", "tooltip", "tooltipPlacement", "class", "type"]);
    const label = () => local.label ?? "Close";
    const button = () => (
        <button
            {...rest}
            type={local.type ?? "button"}
            class={clsx("ui-close", `ui-close--${local.size ?? "md"}`, local.text && "ui-close--text", local.class)}
            aria-label={local.text ? undefined : label()}
            aria-busy={local.busy ? "true" : undefined}
            disabled={rest.disabled || local.busy}
        >
            <i class={local.busy ? "fa-solid fa-spinner fa-spin" : "fa-solid fa-xmark"} aria-hidden="true" />
            <Show when={local.text}>
                <span class="ui-close__text">{local.text}</span>
            </Show>
        </button>
    );
    return (
        <Show when={local.tooltip !== false && !local.text} fallback={button()}>
            <Tooltip content={label()} placement={local.tooltipPlacement ?? "top"} divClassName="ui-tooltip-anchor">
                {button()}
            </Tooltip>
        </Show>
    );
}
