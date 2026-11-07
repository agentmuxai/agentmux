// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * LogView — Layer 2's read-only install console
 * (SPEC_UNIVERSAL_INSTALL_DIALOG_2026_09_23.md §4.2). A virtualised list of
 * fixed-height monospace rows rendered straight from the session's retained
 * log, replacing xterm.js: no focus trap, no FitAddon sizing, no scrollback
 * smaller than the log, and native text selection.
 *
 * Only the rows in view (plus overscan) are in the DOM, so a 20,000-line
 * log costs the same as a short one. Lines don't wrap — long lines scroll
 * horizontally — which keeps every row the same height and makes "scroll
 * to line N" exact.
 *
 * Sticks to the bottom while output streams unless the user scrolled up. The
 * agent pane's follow state machine decides: only a user gesture detaches, and
 * a user scroll back within 24 px re-attaches
 * (SPEC_AGENT_PANE_SCROLL_FOLLOW_STATE_MACHINE_2026_09_24.md Phase 3).
 */

import type { JSX } from "solid-js";

import { LogRows } from "./log-view-parts";
import { createLogView, type LogViewProps } from "./log-view-state";
import "./log-view.scss";

export { LOG_ROW_PX, type LogViewApi } from "./log-view-state";

export const LogView = (props: LogViewProps): JSX.Element => {
    const s = createLogView(props);

    return (
        <div
            class="install-log"
            ref={s.setEl}
            role="log"
            aria-label="Install output"
            tabIndex={0}
            onScroll={s.onScroll}
            onContextMenu={s.onContextMenu}
        >
            <LogRows s={s} />
        </div>
    );
};
