// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// An Audit pane's instance, loaded with the first one (audit.tsx).

import type { PaneTabHostContext, PaneTabInstance } from "@/app/block/pane-tab-registry";
import { readZoom } from "@/app/store/zoom-factor";
import { AuditManager } from "./audit-manager";
import "./audit-manager.scss";

export function create(ctx: PaneTabHostContext): PaneTabInstance {
    return {
        component: () => (
            // audit-container carries the container the feed's width queries answer; a container
            // element cannot respond to its own container query.
            <div class="audit-container">
                <div class="audit-pane" style={{ zoom: readZoom(ctx.meta()) }}>
                    <AuditManager visible={ctx.visibility() === "active"} />
                </div>
            </div>
        ),
    };
}
