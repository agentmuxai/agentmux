// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import clsx from "clsx";
import type { JSX } from "solid-js";
import { densityClass, type UiDensity } from "./shared";

import "./pane-parts.scss";

export interface PaneBodyProps {
    /** Space around the content. */
    padded?: boolean;
    /** The whole body scrolls. Leave it off for a pane with a fixed header and a `PaneScroll` below it. */
    scroll?: boolean;
    density?: UiDensity;
    class?: string;
    children?: JSX.Element;
}

/**
 * The root of a pane's view: it fills the pane, lays its children out in a
 * column, and lets them shrink (`min-height: 0`), which is what makes a list
 * inside it scroll instead of pushing the pane taller. Every pane that wrote
 * `display: flex; flex-direction: column; height: 100%; min-height: 0` as its
 * own root rule uses this instead.
 */
export function PaneBody(props: PaneBodyProps): JSX.Element {
    return (
        <div
            class={clsx(
                "ui-pane-body",
                props.padded && "ui-pane-body--padded",
                props.scroll && "ui-pane-body--scroll",
                densityClass(props.density),
                props.class
            )}
        >
            {props.children}
        </div>
    );
}

export interface PaneScrollProps {
    class?: string;
    children?: JSX.Element;
}

/** The part of a `PaneBody` that scrolls and takes the room its header and footer leave. */
export function PaneScroll(props: PaneScrollProps): JSX.Element {
    return <div class={clsx("ui-pane-scroll", props.class)}>{props.children}</div>;
}
