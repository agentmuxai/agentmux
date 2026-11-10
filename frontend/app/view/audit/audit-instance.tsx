// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// An Audit pane's instance, loaded with the first one (audit.tsx).

import type { PaneTabHostContext, PaneTabInstance } from "@/app/store/pane-tab-registry";
import { readZoom } from "@/app/store/zoom-factor";
import { AuditManager } from "./audit-manager";
import { AUDIT_AGENT_META_KEY, AUDIT_SOUND_META_KEY } from "./audit-shared";
import { parseSoundRef } from "./audit-sound";
import "./audit-manager.scss";

export function create(ctx: PaneTabHostContext): PaneTabInstance {
    return {
        component: () => (
            // audit-container carries the container the feed's width queries answer; a container
            // element cannot respond to its own container query.
            <div class="audit-container">
                <div class="audit-pane" style={{ zoom: readZoom(ctx.meta()) }}>
                    <AuditManager
                        paneBlockId={ctx.blockId}
                        visible={ctx.visibility() === "active"}
                        agent={typeof ctx.meta()?.[AUDIT_AGENT_META_KEY] === "string" ? (ctx.meta()?.[AUDIT_AGENT_META_KEY] as string) : undefined}
                        sound={parseSoundRef(ctx.meta()?.[AUDIT_SOUND_META_KEY])}
                    />
                </div>
            </div>
        ),
    };
}
