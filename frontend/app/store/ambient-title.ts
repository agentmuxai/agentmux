// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Is this stored value a real title? The frontend's port of
 * `crates/srv/src/ambient/validate.rs` (`is_usable_title`), used before a stored
 * `term:ambient_summary` is shown, so a value that is about the ABSENCE of a title
 * never reaches the swarm row, the pane header or a tooltip.
 *
 * Why it exists: on 2026-10-02 the swarm row for an agent read `(none yet)`. That is
 * the title prompt's own placeholder, echoed back by the model, accepted by a
 * validator that only knew the bare words "none" and "n/a", stored, and then fed back
 * as the current title. The backend now rejects such text before it is stored and
 * never feeds it back, but values already in a database from before that, and values
 * written by an older build, are still there. This judges them on read, so they stop
 * displaying with no migration.
 *
 * The two implementations cannot drift: both test suites load the one corpus in
 * `crates/srv/src/ambient/title_corpus.json`. Change a list here only together with
 * the Rust lists and that file. See
 * docs/specs/SPEC_AMBIENT_SWARM_SUMMARY_HARDENING_2026_10_02.md section 5.1.
 */

/** Whole replies that are about the absence of a title, compared in `absenceForm`. */
const ABSENCE_EXACT = new Set([
    "none",
    "n a",
    "na",
    "null",
    "nothing",
    "empty",
    "unknown",
    "untitled",
    "untitled session",
    "untitled conversation",
    "untitled chat",
    "tbd",
    "to be determined",
    "placeholder",
    "pending",
    // The abstain token the title prompts give the model for "no change".
    "keep",
    "not set",
    "not yet",
    "no title",
    "no summary",
    "no goal",
    "no task",
    "no activity",
    "no recent activity",
    "nothing yet",
]);

/** Specific enough that a real title is very unlikely to begin with them. */
const ABSENCE_PREFIXES = [
    "none yet",
    "no title",
    "no summary yet",
    "no goal established",
    "no goal yet",
    "no goal set",
    "no task yet",
    "no activity yet",
    "nothing yet",
    "not set",
    "not yet established",
    "title unavailable",
    "title not",
];

/** First-person refusals and offers: a model talking to the reader. */
const REFUSAL_PHRASES = [
    "i don't have",
    "i do not have",
    "i don't know",
    "i cannot",
    "i can't",
    "i'm unable",
    "i am unable",
    "i'm not able",
    "i would need",
    "i'd need",
    "i need to see",
    "i need more",
    "if you'd like",
    "if you would like",
    "if you want me",
    "without knowing",
    "not enough context",
    "not enough information",
    "as an ai",
];

/** Bounds for a stored title; generous because an older build may have written a longer one. */
const MAX_WORDS = 28;
const MAX_CHARS = 200;

const ALNUM = /[\p{L}\p{N}]/u;
const ALPHA = /\p{L}/u;

/** Lower-case with typographic apostrophes straightened, for matching only. */
function normalized(text: string): string {
    return text.replace(/[’‘]/g, "'").toLowerCase();
}

/**
 * Every non-alphanumeric run collapsed to one space, trimmed: `(None yet)`,
 * `"none yet."` and `none-yet` all become `none yet`, while `No-op rename threshold`
 * becomes `no op rename threshold`.
 */
function absenceForm(text: string): string {
    let out = "";
    let gap = false;
    for (const c of normalized(text)) {
        if (ALNUM.test(c)) {
            if (gap && out.length > 0) out += " ";
            gap = false;
            out += c;
        } else {
            gap = true;
        }
    }
    return out;
}

/** A reply that is entirely a parenthetical or bracketed note about the title. */
function isWrappedNote(text: string): boolean {
    const t = text.trim();
    return (t.startsWith("(") && t.endsWith(")")) || (t.startsWith("[") && t.endsWith("]"));
}

function isAbsence(text: string): boolean {
    if (isWrappedNote(text)) return true;
    const form = absenceForm(text);
    if (ABSENCE_EXACT.has(form)) return true;
    return ABSENCE_PREFIXES.some((p) => form === p || form.startsWith(`${p} `));
}

/** Whether `phrase` occurs in `lower` as whole words ("has an AI summary bug" is not "as an ai"). */
function hasPhrase(lower: string, phrase: string): boolean {
    let from = 0;
    for (;;) {
        const at = lower.indexOf(phrase, from);
        if (at < 0) return false;
        const before = at === 0 ? "" : lower[at - 1];
        const after = lower[at + phrase.length] ?? "";
        if (!(before && ALNUM.test(before)) && !(after && ALNUM.test(after))) return true;
        from = at + 1;
    }
}

/** Is `stored` a real title, as opposed to empty text, a paragraph, a placeholder, or a refusal? */
export function isUsableTitle(stored: string | null | undefined): boolean {
    if (typeof stored !== "string") return false;
    const text = stored.trim();
    if (text.length === 0 || text.includes("\n")) return false;
    if (Array.from(text).length > MAX_CHARS) return false;
    if (text.split(/\s+/).length > MAX_WORDS) return false;
    if (!ALPHA.test(text)) return false;
    if (isAbsence(text)) return false;
    const lower = normalized(text);
    return !REFUSAL_PHRASES.some((p) => hasPhrase(lower, p));
}
