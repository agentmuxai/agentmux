// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * When the pane asks for a new session title, and when it keeps the old one.
 *
 * The title used to be re-requested on every human message, which spent a model
 * call per turn and let the title wander between near-synonyms. The owner chose
 * the `muxterm` schedule (spec section 9, decision 5): with no usable title, ask
 * on every message until there is one; once there is one, re-evaluate after
 * human turns 2, 5 and 8, then every third (11, 14, ...). A new title replaces
 * the old one only if it is news, not a rewording of the same goal.
 *
 * The backend's empty-title recovery (activity_watcher.rs) covers the agents this
 * cannot: ones driven by jekts or tools, with no human message to trigger it.
 * docs/specs/SPEC_AMBIENT_SWARM_SUMMARY_HARDENING_2026_10_02.md section 5.6.
 */

/** Block meta key: human (non-hidden) messages submitted this session. Cleared at
 *  the session boundary with the title (useBlockActivity.ts). */
export const META_HUMAN_TURNS = "term:human_turns";

/** Is human turn `n` (1-based) one where an existing title is re-evaluated? */
export function isReevaluationTurn(n: number): boolean {
    if (!Number.isInteger(n) || n < 2) return false;
    if (n === 2 || n === 5 || n === 8) return true;
    return n > 8 && (n - 8) % 3 === 0;
}

/** Should the pane ask for a title on human turn `n`? Always while there is no
 *  usable title (so a missing one is recovered at the next message); otherwise
 *  only on a re-evaluation turn. */
export function shouldRequestTitle(hasUsableTitle: boolean, n: number): boolean {
    return !hasUsableTitle || isReevaluationTurn(n);
}

/** Words that carry no topic, ignored when comparing two titles. */
const STOP_WORDS = new Set([
    "a", "an", "the", "and", "or", "of", "to", "for", "in", "on", "with", "by", "at", "from", "into", "its", "it",
]);

function topicWords(title: string): Set<string> {
    return new Set(
        title
            .toLowerCase()
            .replace(/[^a-z0-9 ]+/g, " ")
            .split(/\s+/)
            .filter((w) => w.length > 0 && !STOP_WORDS.has(w))
    );
}

/** Below this overlap (Jaccard over topic words) a new title counts as news. */
export const NEWS_MAX_OVERLAP = 0.5;

/**
 * Is `candidate` a different goal from `current`, or a rewording of it? A
 * rewording ("Fix the login race" → "Fix login race condition") keeps the old
 * title, so the title stays stable; a real change of topic replaces it.
 */
export function isTitleNews(current: string, candidate: string): boolean {
    const a = topicWords(current);
    const b = topicWords(candidate);
    if (a.size === 0 || b.size === 0) return a.size !== b.size;
    let shared = 0;
    for (const w of a) if (b.has(w)) shared++;
    const overlap = shared / (a.size + b.size - shared);
    return overlap < NEWS_MAX_OVERLAP;
}
