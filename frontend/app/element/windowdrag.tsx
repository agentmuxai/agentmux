// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import clsx from "clsx";
import { JSX } from "solid-js";
import { useWindowDrag } from "@/app/hook/useWindowDrag";
import type { Properties as CSSProperties } from "csstype";

import "./windowdrag.scss";

interface WindowDragProps {
    class?: string;
    style?: CSSProperties;
    children?: JSX.Element;
    ref?: HTMLDivElement | ((el: HTMLDivElement) => void);
}

const WindowDrag = (props: WindowDragProps): JSX.Element => {
    const { dragProps } = useWindowDrag();

    return (
        <div
            ref={props.ref as any}
            class={clsx("window-drag", props.class)}
            style={props.style as any}
            {...(dragProps as any)}
        >
            {props.children}
        </div>
    );
};

export { WindowDrag };
