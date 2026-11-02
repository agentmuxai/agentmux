// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Writes the Animations setting (docs/specs/SPEC_ANIMATIONS_SETTINGS_PAGE_2026_11_01.md §3.2): the
// Settings page and the command palette both go through here, so a choice means the same from either.

import {
    type AnimationPreset,
    type AnimationSettings,
    type AnimationValues,
    clampPercent,
    DEFAULT_PRESET,
    DEFAULT_VALUES,
    effectiveValues,
} from "@/app/motion/animation-settings";
import { RpcApi } from "./rpc-api";
import { TabRpcClient } from "./rpc-util";

const write = (values: Record<string, unknown>) => void RpcApi.SetConfigCommand(TabRpcClient, values as any);

/** Choose a preset. The custom numbers stay stored, so "Use my custom values" brings them back. */
export function setAnimationPreset(preset: AnimationPreset): void {
    write({ "animation:preset": preset });
}

/**
 * Edit one or more numbers. The choice becomes Custom, starting from the numbers the page shows now
 * (so editing Quick's duration keeps Quick's other numbers), each clamped and snapped.
 */
export function editAnimation(current: AnimationSettings, patch: Partial<AnimationValues>): void {
    const next = { ...effectiveValues(current), ...patch };
    write({
        "animation:preset": "custom",
        "animation:speed": clampPercent(next.speed, DEFAULT_VALUES.speed),
        "animation:polish": clampPercent(next.polish, DEFAULT_VALUES.polish),
        "animation:reducemovement": next.reduceMovement,
    });
}

/** Smooth, 100, 100, off. */
export function resetAnimation(): void {
    write({
        "animation:preset": DEFAULT_PRESET,
        "animation:speed": DEFAULT_VALUES.speed,
        "animation:polish": DEFAULT_VALUES.polish,
        "animation:reducemovement": DEFAULT_VALUES.reduceMovement,
    });
}
