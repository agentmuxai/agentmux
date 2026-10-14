// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A request, left on a pane's block meta, for the agent to catch up
 * (SPEC_RESUME_PHASE_CATCH_UP_TURN_2026_10_13.md, phase 2). The surface that
 * resumes an agent (My agents) is not the pane that runs it, and the pane's
 * catch-up turn is started by its own stream controller
 * (`memory-reinjection-controller.ts`), so the request travels as meta: set
 * once the launch succeeds, read and cleared by the pane when it mounts.
 */

import { setBlockMeta } from "@/app/store/block-meta";

/** The block meta key the request lives under. */
export const CATCH_UP_META_KEY = "agent:catchup";

/** An older request is not about a resume happening now (an app closed before the pane read it). */
export const CATCH_UP_REQUEST_MAX_AGE_MS = 2 * 60_000;

// A key outside the typed block meta, which merges as any other.
const metaOf = (value: string | null): MetaType => ({ [CATCH_UP_META_KEY]: value }) as unknown as MetaType;

/** What a request can ask for. */
export type CatchUpRequestReason = "resumed";

/** Ask the pane of `blockId` to catch up once it is running. */
export function requestCatchUp(blockId: string, reason: CatchUpRequestReason, now: number = Date.now()): Promise<void> {
    return setBlockMeta(blockId, metaOf(`${reason}:${now}`));
}

/**
 * The reason a stored request asks for, or `null` for none: nothing stored, a
 * value this build does not know, or a request too old to be about now.
 */
export function parseCatchUpRequest(value: unknown, now: number): CatchUpRequestReason | null {
    if (typeof value !== "string") return null;
    const [reason, stamp] = value.split(":");
    if (reason !== "resumed") return null;
    const at = Number(stamp);
    if (!Number.isFinite(at) || now - at > CATCH_UP_REQUEST_MAX_AGE_MS) return null;
    return reason;
}

/**
 * Whether a launch is the user picking a conversation back up: it resumes a
 * session. A fork is a new branch from one, and a greenfield launch has no
 * conversation to catch up on.
 */
export function launchResumesConversation(resumedSessionId: string, forkSession: boolean | undefined): boolean {
    return resumedSessionId.trim() !== "" && !forkSession;
}

/** Remove the request, whether or not it was acted on. */
export function clearCatchUpRequest(blockId: string): Promise<void> {
    return setBlockMeta(blockId, metaOf(null));
}
