// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { type JSX } from "solid-js";
import { getPaneScope } from "./scope";

/**
 * The element a pane's backdrop properties sit on (spec §7.4): a div that carries the block's scope as inline custom
 * properties, so everything inside reads the pane's colours. It takes no part in layout (`display: contents`, in
 * backdrop.scss), and a block with no scope gets none of the properties and no `data-backdrop`, so the window's
 * theme applies as before.
 */
export function PaneScope(props: { blockId: string; children?: JSX.Element }): JSX.Element {
    const vars = getPaneScope(props.blockId);
    return (
        <div class="pane-scope" data-backdrop={vars() ? "" : undefined} style={vars() ?? {}}>
            {props.children}
        </div>
    );
}
