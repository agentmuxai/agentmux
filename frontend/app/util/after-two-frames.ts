// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Run `run` two animation frames from now. The first frame applies what was just changed (a display flip, a
 * layout write); the second is the one in which it can be measured. SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md
 * §5.2 #9: three call sites had spelled the nested `requestAnimationFrame` out by hand.
 */
export function afterTwoFrames(run: () => void): void {
    requestAnimationFrame(() => requestAnimationFrame(() => run()));
}
