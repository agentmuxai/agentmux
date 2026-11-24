// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The answer of `fs.open` for a file a container agent can write, until the person has said yes: a sentence that
 * starts `[asks-first:<agent id>] `. A window that can ask turns it into a confirm naming the agent; a caller that
 * can't shows the sentence without the marker.
 */

const ASKS_FIRST = /^\[asks-first:([^\]]*)\] /;

function textOf(err: unknown): string {
    return err instanceof Error ? err.message : String(err);
}

/** The id of the container agent a failed open says the person has to be asked about (empty when srv could not name
 *  it), or null for any other failure. */
export function askedFirstAgent(err: unknown): string | null {
    return ASKS_FIRST.exec(textOf(err))?.[1] ?? null;
}

/** The sentence of a failed open without the marker that goes in front of the one that asks first. */
export function withoutAskMarker(text: string): string {
    return text.replace(ASKS_FIRST, "");
}
