// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Aggregates every section's colocated settings registry into one flat,
// searchable array — see docs/specs/SPEC_SETTINGS_PANE_SEARCH_2026_09_21.md
// §3.2. Plain module-level data, so it exists on import regardless of which
// section is currently mounted (the gap the search feature needs closed).

import type { SettingsIndexEntry } from "./settings-model";
import { APPEARANCE_SETTINGS } from "./sections/appearance-section";
import { WINDOW_SETTINGS } from "./sections/window-panes-section";
import { TERMINAL_SETTINGS } from "./sections/terminal-section";
import { SOUNDS_SETTINGS } from "./sections/sounds-section";
import { RECORDING_SETTINGS } from "./sections/recording-section";
import { ADVANCED_SETTINGS } from "./sections/advanced-section";

export const SETTINGS_INDEX: SettingsIndexEntry[] = [
    ...Object.values(APPEARANCE_SETTINGS),
    ...Object.values(WINDOW_SETTINGS),
    ...Object.values(TERMINAL_SETTINGS),
    ...Object.values(SOUNDS_SETTINGS),
    ...Object.values(RECORDING_SETTINGS),
    ...Object.values(ADVANCED_SETTINGS),
];
