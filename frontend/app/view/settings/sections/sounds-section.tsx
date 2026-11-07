// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { type JSX } from "solid-js";

import { NotificationSoundsGroup, ShutdownWarningGroup, ToolTonesGroup, WaitingToneGroup } from "./sounds-section-parts";

export { SOUNDS_SETTINGS } from "./sounds-settings-index";

// ── Section: Sounds & Notifications ───────────────────────────────────────────

export function SoundsSection(): JSX.Element {
    return (
        <div class="settings-section-body">
            <NotificationSoundsGroup />
            <ToolTonesGroup />
            <WaitingToneGroup />
            <ShutdownWarningGroup />
        </div>
    );
}
