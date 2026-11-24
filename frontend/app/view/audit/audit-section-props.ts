// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** What the pane hands each of its sections (audit-instance.tsx). `visible` is true only while the pane is on screen and this
 *  section is the one shown, so a hidden section reads nothing and polls nothing. */
export interface AuditSectionProps {
    paneBlockId: string;
    visible: boolean;
    meta: Record<string, unknown> | undefined;
}
