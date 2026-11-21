// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Spec §4.3: a message is linkified when it is first drawn and the segments are kept by (message id, text hash, index
 * versions), so a long transcript is never scanned whole and a redraw costs a map hit. A roster or task change bumps a
 * version, and a message is linkified again lazily, when it is next drawn.
 */

import { linkify } from "./linkify";
import type { LinkContext, Segment } from "./types";

/** FNV-1a over the text's UTF-16 units: cheap, and the key also holds the length. */
function hashText(text: string): string {
    let h = 0x811c9dc5;
    for (let i = 0; i < text.length; i++) {
        h ^= text.charCodeAt(i);
        h = Math.imul(h, 0x01000193) >>> 0;
    }
    return `${h.toString(36)}.${text.length}`;
}

export type SegmentCache = {
    get(id: string, text: string, ctx: LinkContext): Segment[];
    size(): number;
};

export function createSegmentCache(limit = 2000): SegmentCache {
    const entries = new Map<string, Segment[]>();
    return {
        get(id, text, ctx) {
            const author = ctx.author.kind === "agent" ? `agent:${ctx.author.id}:${ctx.author.confined}:${ctx.author.remote ?? false}` : ctx.author.kind;
            const key = `${id}|${hashText(text)}|${author}|${ctx.final ? 1 : 0}|${ctx.agents.version}.${ctx.tasks.version}|${ctx.board ?? ""}|${ctx.repo ?? ""}`;
            const hit = entries.get(key);
            if (hit) return hit;
            const made = linkify(text, ctx);
            if (entries.size >= limit) entries.delete(entries.keys().next().value as string);
            entries.set(key, made);
            return made;
        },
        size: () => entries.size,
    };
}
