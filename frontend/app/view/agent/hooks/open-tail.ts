// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * `agent:open_tail`: the agent pane's first screen from one small read
 * (SPEC_AGENT_OPEN_FIRST_SCREEN_THEN_REST_2026_10_19.md §3.1).
 *
 * It replaces the open's three or four calls (the snapshot, the line count, a
 * 10,000-line range) with one: the newest ~64 KB of the transcript, cut at line
 * boundaries, with the snapshot overlay and where the slice sits in the
 * stream. The pane paints it, lets the user send, and resumes live records
 * from `end_line`, the same line positions the transcript cursor already
 * keeps (transcript-cursor.ts). The rest of the restore window fills in
 * afterwards (history-fill.ts).
 *
 * Until srv serves it (and for any file srv can't give line positions for), a
 * read returns `null` and the pane opens the way it always did.
 */

import { TabRpcClient } from "@/app/store/rpc-util";

/** A first screen: about what fits one pane, plus a margin. */
export const OPEN_TAIL_BYTES = 64 * 1024;

export interface OpenTailRequest {
    block_id: string;
    filename: "output";
    /** With it, the agent's snapshot comes back in `snapshot`. */
    definition_id?: string;
    tail_bytes?: number;
    /** The same trimming rule as `blockfile:read_range`'s. */
    tail_turns?: number;
}

export interface OpenTailResult {
    /** Contiguous from `start_line`, cut at line boundaries. */
    lines: string[];
    /** Receive-time stamps (unix ms) parallel to `lines`; absent without a sidecar. */
    stamps?: number[];
    /** The stream and generation the slice came from, in one reader snapshot. */
    stream?: string;
    gen?: string;
    /** The line number of `lines[0]`. */
    start_line?: number;
    /** One past the last line: `start_line + lines.length`. The cursor's `next`. */
    end_line?: number;
    /** Lines the stream held at that snapshot; the writer may be ahead of `end_line`. */
    total_lines?: number;
    start_byte?: number;
    end_byte?: number;
    size?: number;
    has_more_before?: boolean;
    /** `agent:session:read`'s content and modts, when `definition_id` was given. */
    snapshot?: string;
    snapshot_modts?: number;
}

/** An open tail with the line positions the pane needs, checked. */
export interface OpenTail extends OpenTailResult {
    stream: string;
    gen: string;
    start_line: number;
    end_line: number;
    total_lines: number;
}

/**
 * Set once srv answers "unknown command": an older srv doesn't serve the
 * command, so later opens skip the call instead of paying a round trip each.
 */
let unsupported = false;

export function __resetOpenTailForTests(): void {
    unsupported = false;
}

const UNKNOWN_COMMAND = /unknown command/i;

/**
 * Whether `r` can open a pane: line positions present and consistent. A file
 * srv has no line counter for (no stream or generation) can't be resumed by
 * line, so it isn't usable.
 */
export function usableOpenTail(r: OpenTailResult | null | undefined): r is OpenTail {
    if (!r || !Array.isArray(r.lines)) return false;
    if (typeof r.stream !== "string" || typeof r.gen !== "string" || r.stream === "" || r.gen === "") return false;
    if (!Number.isInteger(r.start_line) || !Number.isInteger(r.end_line) || !Number.isInteger(r.total_lines)) return false;
    const start = r.start_line as number;
    const end = r.end_line as number;
    if (start < 0 || end !== start + r.lines.length || (r.total_lines as number) < end) return false;
    // Stamps, when given, run parallel to the lines.
    if (r.stamps !== undefined && r.stamps.length !== r.lines.length) return false;
    return true;
}

/**
 * Read the first screen. `null`: use the open's usual path (srv doesn't serve
 * the command, the read failed, or its answer has no usable line positions).
 */
export async function readOpenTail(
    req: OpenTailRequest,
    opts?: { call?: (req: OpenTailRequest) => Promise<OpenTailResult>; timeout?: number },
): Promise<OpenTail | null> {
    if (unsupported) return null;
    const call =
        opts?.call ??
        ((r: OpenTailRequest) => TabRpcClient.rpcCall("agent:open_tail", r, { timeout: opts?.timeout ?? 5000 }) as Promise<OpenTailResult>);
    try {
        const res = await call(req);
        return usableOpenTail(res) ? res : null;
    } catch (err) {
        if (UNKNOWN_COMMAND.test(String((err as { message?: string } | null)?.message ?? err))) unsupported = true;
        return null;
    }
}
