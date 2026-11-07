// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Notifications & tray — docs/specs/SPEC_OS_NOTIFICATIONS_SYSTEM_2026_09_24.md.
// Phase 0 (§4.1): make the existing tray + background-service mode reachable
// from Settings instead of env vars only, and surface auto-start as its own,
// separate toggle (tray spec §7.4 — the two decisions stay independent).


import { type JSX } from "solid-js";

import { createAutostart } from "./notifications-section-state";
import { OsNotificationsGroup, StartAtLoginRow, TestNotificationRow, TrayGroup } from "./notifications-section-parts";

export { NOTIFICATIONS_SETTINGS } from "./notifications-settings-index";

// ── Section: Notifications & Tray ─────────────────────────────────────────────

export function NotificationsSection(): JSX.Element {
    const a = createAutostart();

    return (
        <div class="settings-section-body">
            <OsNotificationsGroup />
            <TestNotificationRow />
            <TrayGroup />
            <StartAtLoginRow a={a} />
        </div>
    );
}
