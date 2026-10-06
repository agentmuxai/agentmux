// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import clsx from "clsx";
import { For, type JSX, Show } from "solid-js";
import { useField } from "./Field";
import { densityClass, rovingTarget, UiIcon, type UiDensity } from "./shared";

import "./ui.scss";

export interface SegmentedOption<T extends string> {
    value: T;
    label: string;
    icon?: string;
    /** Hide the label and show only the icon; the label stays the accessible name. */
    iconOnly?: boolean;
    disabled?: boolean;
}

export interface SegmentedControlProps<T extends string> {
    options: SegmentedOption<T>[];
    value: T;
    onChange: (value: T) => void;
    density?: UiDensity;
    /** Needed unless the control sits in a `Field`, whose label names it. */
    ariaLabel?: string;
    disabled?: boolean;
    class?: string;
}

/**
 * A short, mutually exclusive choice shown as joined line buttons: the
 * selected option takes the pressed look. A radio group to assistive tech,
 * with arrow-key selection.
 */
export function SegmentedControl<T extends string>(props: SegmentedControlProps<T>): JSX.Element {
    const field = useField();
    const buttons: HTMLButtonElement[] = [];
    const isDisabled = (i: number) => !!props.disabled || !!props.options[i]?.disabled;
    const selectedIndex = () => props.options.findIndex((o) => o.value === props.value);
    // The one option reachable with Tab: the selected one, or the first.
    const tabStop = () => Math.max(0, selectedIndex());

    const onKeyDown = (e: KeyboardEvent, index: number) => {
        // Left/Right and Up/Down both move, as in a native radio group.
        const target =
            rovingTarget(e.key, index, props.options.length, "horizontal", isDisabled) ??
            rovingTarget(e.key, index, props.options.length, "vertical", isDisabled);
        if (target == null) return;
        e.preventDefault();
        props.onChange(props.options[target].value);
        buttons[target]?.focus();
    };

    return (
        <div
            role="radiogroup"
            class={clsx("ui-segmented", densityClass(props.density), props.class)}
            aria-label={props.ariaLabel}
            aria-labelledby={props.ariaLabel ? undefined : field?.labelId}
            aria-describedby={field?.describedBy()}
        >
            <For each={props.options}>
                {(option, i) => (
                    <button
                        ref={(el) => (buttons[i()] = el)}
                        type="button"
                        role="radio"
                        class="ui-segmented-option"
                        aria-checked={option.value === props.value ? "true" : "false"}
                        aria-label={option.iconOnly ? option.label : undefined}
                        tabIndex={i() === tabStop() ? 0 : -1}
                        disabled={isDisabled(i())}
                        onClick={() => props.onChange(option.value)}
                        onKeyDown={(e) => onKeyDown(e, i())}
                    >
                        <Show when={option.icon}>{(icon) => <UiIcon name={icon()} />}</Show>
                        <Show when={!option.iconOnly}>{option.label}</Show>
                    </button>
                )}
            </For>
        </div>
    );
}
