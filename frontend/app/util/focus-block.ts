// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Deprecated name for `revealBlock` (./reveal-block.ts), which also switches a
 * multi-tab pane to the block, un-magnifies, and places the caret. Kept so
 * existing callers keep working; SPEC_REVEAL_BLOCK_ONE_PATH_2026_09_27.md
 * Phase 2 moves them over and removes this file.
 */
export { revealBlock as focusBlock } from "./reveal-block";
