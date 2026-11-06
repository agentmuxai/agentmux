// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { Show, type JSX } from "solid-js";
import "./_maintenance-section.scss";
import { CheckRow, Notices, UpdateRow } from "./maintenance-parts";
import { createMaintenance } from "./maintenance-state";

// ── Component ────────────────────────────────────────────────────────────────

export const MaintenanceSection = (): JSX.Element => {
    const m = createMaintenance();
    return (
        <>
            <div class="instance-panel-section-title">
                Maintenance
                <Show when={m.badge() != null}>
                    <span class="maintenance-badge">{m.badge()}</span>
                </Show>
            </div>

            {/* ── Update row ─────────────────────────────────────────────── */}
            <UpdateRow m={m} />
            <Notices m={m} />
            <CheckRow m={m} />
        </>
    );
};

MaintenanceSection.displayName = "MaintenanceSection";
