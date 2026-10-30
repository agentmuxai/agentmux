// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import clsx from "clsx";
import type { JSX } from "solid-js";

export interface KbdProps {
    children: JSX.Element;
    class?: string;
}

/** A key cap, for a hint that names a key: `<Kbd>Enter</Kbd> to open`. */
export function Kbd(props: KbdProps): JSX.Element {
    return <kbd class={clsx("ui-kbd", props.class)}>{props.children}</kbd>;
}
