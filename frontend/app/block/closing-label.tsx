// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { JSX } from "solid-js";

/** What replaces the pane's buttons from the click on X until the pane is gone
 *  (SPEC_PANE_CLOSING_INDICATOR_AND_FASTER_CLOSE_2026_10_28.md §2). In a row too narrow for the
 *  words the label clips to the spinner, and the title says it. */
export function ClosingLabel(props: { tabs: number }): JSX.Element {
    const text = () => (props.tabs > 1 ? `Closing ${props.tabs} tabs…` : "Closing…");
    return (
        <div class="block-frame-closing" role="status" title={text()}>
            <i class="fa-solid fa-spinner fa-spin" aria-hidden="true" />
            <span class="block-frame-closing-text">{text()}</span>
        </div>
    );
}
