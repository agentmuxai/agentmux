// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The motion tokens for JS animations and inline styles: the same values as
// theme.scss's `--duration-*` and `--ease-*` (tokens.test.ts compares them).
// SPEC_MOTION_TIERS_AND_STYLESHEET_DRY_2026_10_15.md §4.7.
// CSS multiplies each duration by `--motion-scale`, which is 1 in both motion
// tiers; these are the unscaled values.

export const DURATION_INSTANT_MS = 80;
export const DURATION_FAST_MS = 100;
export const DURATION_QUICK_MS = 120;
export const DURATION_BASE_MS = 160;
export const DURATION_MODERATE_MS = 240;
export const DURATION_SLOW_MS = 280;

export const EASE_STANDARD = "ease";
export const EASE_OUT = "ease-out";
export const EASE_IN_OUT = "ease-in-out";
export const EASE_EMPHASIZED = "cubic-bezier(0.2, 1, 0.3, 1)";
export const EASE_MATERIAL = "cubic-bezier(0.4, 0, 0.2, 1)";
export const EASE_DECELERATE = "cubic-bezier(0.2, 0, 0, 1)";
export const EASE_BOUNCE = "cubic-bezier(0.34, 1.56, 0.64, 1)";
