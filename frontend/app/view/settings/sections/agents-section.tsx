// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Agents: what concerns the agents themselves rather than a pane. For now,
// their ids: forks whose id an older build chained, and updating them
// (fork-slug-migration.tsx).

import type { JSX } from "solid-js";

import { SectionHeader } from "../settings-controls";
import { FORK_SLUG_SETTING, ForkSlugMigration } from "./fork-slug-migration";

// ── Search index — see appearance-section.tsx's header comment for the pattern. ──

export const AGENTS_SETTINGS = {
    outdatedIds: FORK_SLUG_SETTING,
};

export function AgentsSection(): JSX.Element {
    return (
        <div class="settings-section-body">
            <SectionHeader label="Agent ids" />
            <ForkSlugMigration />
        </div>
    );
}
