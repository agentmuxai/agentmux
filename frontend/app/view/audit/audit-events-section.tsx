// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Audit pane's Events section: the list of everything agents did (audit-manager.tsx). `.audit-pane` is the
// container its width queries answer; it sits inside the pane's zoom, so it measures the width the content is laid out in.

import type { JSX } from "solid-js";
import { AuditManager } from "./audit-manager";
import { parseMessageRef } from "./audit-message";
import type { AuditSectionProps } from "./audit-section-props";
import { AUDIT_AGENT_META_KEY, AUDIT_MESSAGE_META_KEY, AUDIT_SOUND_META_KEY } from "./audit-shared";
import { parseSoundRef } from "./audit-sound";

export default function AuditEventsSection(props: AuditSectionProps): JSX.Element {
    const agent = () => {
        const a = props.meta?.[AUDIT_AGENT_META_KEY];
        return typeof a === "string" ? a : undefined;
    };
    return (
        <div class="audit-container">
            <div class="audit-pane">
                <AuditManager
                    paneBlockId={props.paneBlockId}
                    visible={props.visible}
                    agent={agent()}
                    sound={parseSoundRef(props.meta?.[AUDIT_SOUND_META_KEY])}
                    message={parseMessageRef(props.meta?.[AUDIT_MESSAGE_META_KEY])}
                />
            </div>
        </div>
    );
}
