// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The stacking layers TypeScript sets inline, at theme.scss's `--z-*` numbers
// (zindex.test.ts compares them; SPEC_MOTION_TIERS_AND_STYLESHEET_DRY_2026_10_15.md
// §6.5 item 17). Numbers, not `var(--z-…)`: the perf HUD works with the main
// stylesheet off, and a misspelt variable would silently make a layer `auto`.

/** The media-capture indicator, just under the context menus. */
export const Z_CAPTURE_INDICATOR = 9998;
/** The context menus and the tab colour panel. */
export const Z_TRANSIENT = 9999;
/** The JS context menu's full-window overlay, over every dialog. */
export const Z_JS_MENU = 99999;
/** The startup splash; index.html sets it before the stylesheet loads. */
export const Z_SPLASH = 99999;
/** The dev diagnostics panel, under the performance HUD. */
export const Z_DEV_DIAGNOSTICS = 999998;
/** The performance HUD, over everything. */
export const Z_DEV_HUD = 999999;
