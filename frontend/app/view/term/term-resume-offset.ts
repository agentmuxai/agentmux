// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Where the terminal's PTY offset stands after the stored output was loaded from
 * `asked`. srv keeps only the last part of a terminal's output (a circular file),
 * so a load from an offset that has since been dropped starts later in the file:
 * `dataOffset` says where. The offset then has to be the absolute end of what was
 * loaded, or the live chunks the load already covered are written a second time
 * and the saved offset is behind the file. `null` when nothing was dropped (or
 * srv doesn't say), which leaves the usual "asked + length" bookkeeping alone.
 */
export function resumeOffsetAfterTrim(asked: number, dataOffset: number | undefined, length: number): number | null {
    if (dataOffset == null || dataOffset <= asked) return null;
    return dataOffset + length;
}
