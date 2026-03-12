// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import clsx from "clsx";
import { JSX, mergeProps, splitProps } from "solid-js";
import { Dynamic } from "solid-js/web";

import "./button.scss";

interface ButtonProps extends JSX.ButtonHTMLAttributes<HTMLButtonElement> {
    className?: string;
    children?: JSX.Element;
    as?: string | ((props: any) => JSX.Element);
}

function Button(inProps: ButtonProps): JSX.Element {
    const props = mergeProps({ as: "button", className: "" }, inProps);
    const [local, rest] = splitProps(props, ["children", "disabled", "className", "as"]);

    // Check if the className contains any of the categories: solid, outlined, or ghost
    const containsButtonCategory = /(solid|outline|ghost)/.test(local.className);
    // If no category is present, default to 'solid'
    const categoryClassName = containsButtonCategory ? local.className : `solid ${local.className}`;

    // Check if the className contains any of the color options: green, grey, red, or yellow
    const containsColor = /(green|grey|red|yellow)/.test(categoryClassName);
    // If no color is present, default to 'green'
    const finalClassName = containsColor ? categoryClassName : `green ${categoryClassName}`;

    return (
        <Dynamic
            component={local.as}
            tabIndex={local.disabled ? -1 : 0}
            class={clsx("wave-button", finalClassName)}
            disabled={local.disabled}
            {...rest}
        >
            {local.children}
        </Dynamic>
    );
}

export { Button };
