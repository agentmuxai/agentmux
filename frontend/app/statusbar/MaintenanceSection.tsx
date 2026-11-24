// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { Show, type JSX } from "solid-js";
import "./_maintenance-section.scss";
import { MessageRows } from "./maintenance-rows";
import { createMaintenance } from "./maintenance-state";
import { CHECK_ROWS, NOTICE_ROWS, STORE_ROWS, UPDATE_ROWS } from "./maintenance-table";

// ── Component ────────────────────────────────────────────────────────────────

export const MaintenanceSection = (): JSX.Element => {
    const m = createMaintenance();
    // Its buttons start a download, an upgrade and a restart: they take input from the person only.
    return (
        <div data-operator-only style={{ display: "contents" }}>
            <div class="instance-panel-section-title">
                Maintenance
                <Show when={m.badge() != null}>
                    <span class="maintenance-badge">{m.badge()}</span>
                </Show>
            </div>

            {/* The update, what keeps it from going ahead, the Store's lines, then Check for update. */}
            <MessageRows m={m} defs={UPDATE_ROWS} />
            <MessageRows m={m} defs={NOTICE_ROWS} />
            <MessageRows m={m} defs={STORE_ROWS} />
            <MessageRows m={m} defs={CHECK_ROWS} />
        </div>
    );
};

MaintenanceSection.displayName = "MaintenanceSection";
