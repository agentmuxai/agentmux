// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Pure config/settings signals with no dependencies on other store modules.
// Extracted from global.ts so that block-atom-cache.ts can import them without
// creating a cycle (global.ts → block-atom-cache.ts → global.ts).

import { createMemo, createSignal } from "solid-js";

export const [fullConfigAtom, setFullConfigAtom] = createSignal<FullConfigType>(null);

export const settingsAtom = createMemo<SettingsType>(() => fullConfigAtom()?.settings ?? ({} as SettingsType));

export const hasCustomAIPresetsAtom = createMemo<boolean>(() => {
    const fullConfig = fullConfigAtom();
    if (!fullConfig?.presets) return false;
    for (const presetId in fullConfig.presets) {
        if (presetId.startsWith("ai@") && presetId !== "ai@global" && presetId !== "ai@wave") {
            return true;
        }
    }
    return false;
});
