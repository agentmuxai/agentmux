// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The resolver (spec §4.1): `linkify(text, context) -> Segment[]`. Pure, the same on every surface, and faithful: the
 * segments' labels joined are the input, byte for byte. The steps are protect, recognise, resolve, choose, emit.
 */

import { findProtected, type Protected } from "./protect";
import { RangeSet } from "./range-set";
import { KINDS } from "./registry";
import { taskKind } from "./kinds/task";
import type { Candidate, EntityKind, LinkContext, Range, Resolved, Segment, TraceLine } from "./types";

/** A text longer than this is left as it is: no surface shows an agent message this long to read links in. */
const MAX_LENGTH = 200_000;

/** Soft hyphen, Arabic letter mark, Mongolian vowel separator, zero-width and direction marks and overrides and
 *  isolates, invisible operators, the byte order mark, and the control characters other than tab and line breaks. */
const INVISIBLE = /[\u0000-\u0008\u000B\u000C\u000E-\u001F\u007F-\u009F\u00AD\u061C\u180E\u200B-\u200F\u2028-\u202E\u2060-\u206F\uFEFF]/;

const CODE_SPAN_TASK = /^#\d{1,7}$/;

type Considered = { cand: Candidate; kind: EntityKind; resolved: Resolved | null; skip?: string };

/** A candidate with an invisible character in it or touching it is not linked: stripping them would change what it is. */
function touchesInvisible(text: string, range: Range): boolean {
    return INVISIBLE.test(text.slice(Math.max(0, range.start - 1), Math.min(text.length, range.end + 1)));
}

/** A code span that is wholly one task number is read by the task kind (D4). */
function codeSpanTasks(text: string, spans: Range[], ctx: LinkContext): Candidate[] {
    const found: Candidate[] = [];
    for (const span of spans) {
        const ticks = /^`+/.exec(text.slice(span.start, span.end))?.[0].length ?? 1;
        const start = span.start + ticks;
        const inner = text.slice(start, span.end - ticks);
        if (!CODE_SPAN_TASK.test(inner)) continue;
        for (const c of taskKind.recognise(inner, ctx)) {
            if (c.start === 0 && c.end === inner.length) found.push({ ...c, start: start + c.start, end: start + c.end });
        }
    }
    return found;
}

function skipReason(text: string, cand: Candidate, ctx: LinkContext, prot: Protected): string | undefined {
    if (prot.ranges.find(cand) !== undefined) return "protected: inside code or an existing link";
    if (touchesInvisible(text, cand)) return "control or direction characters";
    if (!ctx.final && cand.end === text.length) return "touches the end of text still arriving";
    return undefined;
}

/** Recognise, then resolve: a candidate with no entity is dropped here, so it cannot hide a lower one that has. */
function recogniseAll(text: string, ctx: LinkContext, prot: Protected): Considered[] {
    const out: Considered[] = [];
    const add = (kind: EntityKind, cand: Candidate, allowedInCode: boolean) => {
        const skip = allowedInCode ? undefined : skipReason(text, cand, ctx, prot);
        out.push({ cand, kind, skip, resolved: skip ? null : kind.resolve(cand, ctx) });
    };
    for (const kind of KINDS) for (const cand of kind.recognise(text, ctx)) add(kind, cand, false);
    for (const cand of codeSpanTasks(text, prot.codeSpans, ctx)) add(taskKind, cand, true);
    return out;
}

/** Overlaps are settled by priority, then the longer match, then the earlier. A candidate with no entity that names
 *  its kind outright (`PR 5`, `task 5`) still claims its words, so a number that is no pull request is not a task. */
function choose(considered: Considered[]): { chosen: Considered[]; outcome: Map<Considered, string> } {
    const outcome = new Map<Considered, string>();
    const live: Considered[] = [];
    for (const c of considered) {
        if (c.skip) outcome.set(c, c.skip);
        else if (!c.resolved && !c.cand.claims) outcome.set(c, "no such entity");
        else live.push(c);
    }
    // A whole agent name that holds another match (the number of `Loap #3`, which is also a task) is the match: the
    // number is a part of the name there, not a reference of its own.
    const byStart = [...live].sort((a, b) => a.cand.start - b.cand.start);
    const holds = new Set<Considered>();
    byStart.forEach((a, index) => {
        if (a.kind.id !== "agent") return;
        for (let next = index + 1; next < byStart.length && byStart[next].cand.start < a.cand.end; next++) {
            if (byStart[next].cand.end <= a.cand.end) holds.add(a);
        }
    });
    live.sort((a, b) => Number(holds.has(b)) - Number(holds.has(a)) || b.kind.priority - a.kind.priority || b.cand.end - b.cand.start - (a.cand.end - a.cand.start) || a.cand.start - b.cand.start);
    const taken = new RangeSet<Considered>();
    const chosen: Considered[] = [];
    for (const c of live) {
        const winner = taken.find(c.cand);
        if (winner) {
            outcome.set(c, `overlapped by a ${winner.kind.id}`);
            continue;
        }
        taken.add(c.cand, c);
        if (c.resolved) chosen.push(c);
        else outcome.set(c, "no such entity");
    }
    chosen.sort((a, b) => a.cand.start - b.cand.start);
    return { chosen, outcome };
}

function emit(text: string, chosen: Considered[], outcome: Map<Considered, string>): Segment[] {
    const segments: Segment[] = [];
    let at = 0;
    for (const c of chosen) {
        const resolved = c.resolved as Resolved;
        if (c.cand.start > at) segments.push({ type: "text", text: text.slice(at, c.cand.start) });
        segments.push({
            type: "link",
            kind: resolved.kind ?? c.kind.id,
            label: text.slice(c.cand.start, c.cand.end),
            destination: resolved.destination,
            state: resolved.state,
            name: resolved.name,
            ...(resolved.reason ? { reason: resolved.reason } : {}),
        });
        at = c.cand.end;
        outcome.set(c, "linked");
    }
    if (at < text.length) segments.push({ type: "text", text: text.slice(at) });
    return segments;
}

type LinkifyRun = { segments: Segment[]; trace: TraceLine[] };

function runLinkify(text: string, ctx: LinkContext): LinkifyRun {
    if (!text || text.length > MAX_LENGTH) return { segments: text ? [{ type: "text", text }] : [], trace: [] };
    const considered = recogniseAll(text, ctx, findProtected(text));
    const { chosen, outcome } = choose(considered);
    const segments = emit(text, chosen, outcome);
    const trace = considered.map((c) => ({ kind: c.kind.id, start: c.cand.start, end: c.cand.end, text: text.slice(c.cand.start, c.cand.end), outcome: outcome.get(c) ?? "" }));
    return { segments, trace };
}

export function linkify(text: string, ctx: LinkContext): Segment[] {
    return runLinkify(text, ctx).segments;
}

/** The debug view's data (spec §5): every candidate seen, and why it was or was not made a link. */
export function explainLinks(text: string, ctx: LinkContext): TraceLine[] {
    return runLinkify(text, ctx).trace;
}
