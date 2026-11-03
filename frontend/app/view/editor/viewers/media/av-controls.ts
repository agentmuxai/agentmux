// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The pure parts of the audio and video viewer's extra controls: playback
 * speed in quarter steps, frame stepping and the keys that drive them. YouTube's
 * keys, as the media report recommends: `<` and `>` slow down and speed up,
 * `,` and `.` step a frame back and forward while paused.
 * docs/reports/REPORT_MEDIA_VIEWER_ZOOM_AND_FEATURES_2026_10_09.md §4 row 7.
 */

export const MIN_RATE = 0.25;
export const MAX_RATE = 4;
export const RATE_STEP = 0.25;

/** What a video's frame lasts until the real rate is measured. */
export const DEFAULT_FRAME_SECONDS = 1 / 30;

/** One quarter step up (1) or down (-1) from `rate`, kept in [MIN_RATE, MAX_RATE]. */
export function stepRate(rate: number, direction: 1 | -1): number {
    const stepped = Math.round((rate + direction * RATE_STEP) / RATE_STEP) * RATE_STEP;
    return Math.min(MAX_RATE, Math.max(MIN_RATE, stepped));
}

/** "1×", "0.75×", "1.5×". */
export function formatRate(rate: number): string {
    return `${Number(rate.toFixed(2))}×`;
}

/** Where a one-frame step from `time` lands, kept inside [0, duration]. */
export function stepFrameTime(time: number, direction: 1 | -1, frameSeconds: number, duration: number): number {
    const target = time + direction * frameSeconds;
    const end = Number.isFinite(duration) ? duration : target;
    return Math.min(end, Math.max(0, target));
}

/** A frame length worth believing: between 240 and 10 frames a second. */
export function plausibleFrameSeconds(seconds: number): boolean {
    return Number.isFinite(seconds) && seconds >= 1 / 240 && seconds <= 1 / 10;
}

export type AvKeyAction = "slower" | "faster" | "frame-back" | "frame-forward" | "loop";

/** The action a key press names, or null. Modified keys are the app's own. */
export function avKeyAction(e: { key: string; ctrlKey?: boolean; metaKey?: boolean; altKey?: boolean }): AvKeyAction | null {
    if (e.ctrlKey || e.metaKey || e.altKey) return null;
    switch (e.key) {
        case "<":
            return "slower";
        case ">":
            return "faster";
        case ",":
            return "frame-back";
        case ".":
            return "frame-forward";
        case "l":
        case "L":
            return "loop";
        default:
            return null;
    }
}
