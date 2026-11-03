// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Counters for the dev HUD / debug log. */
export interface TranscriptCursorStats {
    delivered: number;
    duplicates: number;
    gapsFilled: number;
    gapLinesFilled: number;
    /** Lines the cursor gave up on (too many, read failed, generation gone). */
    linesSkipped: number;
    unpositioned: number;
    genChanges: number;
    echoes: number;
    ownEchoesDropped: number;
}
