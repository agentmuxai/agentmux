// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The resolver (spec §4.1): `linkify(text, context) -> Segment[]`. Pure, the same on every surface, and faithful: the
 * segments' labels joined are the input, byte for byte. The steps are protect, recognise, choose, resolve, emit.
 */

import { findProtected, insideAny, overlaps, type Protected } from "./protect";
import { KINDS } from "./registry";
import { taskKind } from "./kinds/task";
import type { Candidate, EntityKind, LinkContext, Range, Segment, TraceLine } from "./types";

/** A text longer than this is left as it is: no surface shows an agent message this long to read links in. */
const MAX_LENGTH = 200_000;

const BIDI_AND_CONTROL = /[\u200B-\u200F\u2028-\u202E\u2066-\u2069\uFEFF]/;

const CODE_SPAN_TASK = /^#\d{1,7}$/;

type Considered = { cand: Candidate; kind: EntityKind; skip?: string };

function changedByStripping(text: string, range: Range): boolean {
    return BIDI_AND_CONTROL.test(text.slice(Math.max(0, range.start - 1), Math.min(text.length, range.end + 1)));
}

/** A code span that is wholly one task number is read by the task kind (D4). */
function codeSpanTasks(text: string, spans: Range[], ctx: LinkContext): Candidate[] {
    const found: Candidate[] = [];
    for (const span of spans) {
        const ticks = /^`+/.exec(text.slice(span.start, span.end))?.[0].length ?? 1;
        const start = span.start + ticks;
        const end = span.end - ticks;
        const inner = text.slice(start, end);
        if (!CODE_SPAN_TASK.test(inner)) continue;
        for (const c of taskKind.recognise(inner, ctx)) {
            if (c.start === 0 && c.end === inner.length) found.push({ ...c, start: start + c.start, end: start + c.end });
        }
    }
    return found;
}

function recogniseAll(text: string, ctx: LinkContext, prot: Protected): Considered[] {
    const out: Considered[] = [];
    for (const kind of KINDS) {
        for (const cand of kind.recognise(text, ctx)) {
            const skip = insideAny(cand, prot.ranges)
                ? "protected: inside code or an existing link"
                : changedByStripping(text, cand)
                  ? "control or direction characters"
                  : !ctx.final && cand.end === text.length
                    ? "touches the end of text still arriving"
                    : undefined;
            out.push({ cand, kind, skip });
        }
    }
    for (const cand of codeSpanTasks(text, prot.codeSpans, ctx)) out.push({ cand, kind: taskKind });
    return out;
}

/** Overlaps are settled by priority, then the longer match, then the earlier. */
function choose(considered: Considered[]): { chosen: Considered[]; lost: Map<Considered, string> } {
    const live = considered.filter((c) => !c.skip);
    live.sort((a, b) => b.kind.priority - a.kind.priority || b.cand.end - b.cand.start - (a.cand.end - a.cand.start) || a.cand.start - b.cand.start);
    const chosen: Considered[] = [];
    const lost = new Map<Considered, string>();
    for (const c of live) {
        const winner = chosen.find((w) => overlaps(w.cand, c.cand));
        if (winner) lost.set(c, `overlapped by a ${winner.kind.id}`);
        else chosen.push(c);
    }
    chosen.sort((a, b) => a.cand.start - b.cand.start);
    return { chosen, lost };
}

export type LinkifyRun = { segments: Segment[]; trace: TraceLine[] };

function emit(text: string, chosen: Considered[], ctx: LinkContext, trace: Map<Considered, string>): Segment[] {
    const segments: Segment[] = [];
    let at = 0;
    const plain = (to: number) => {
        if (to > at) segments.push({ type: "text", text: text.slice(at, to) });
        at = to;
    };
    for (const c of chosen) {
        const resolved = c.kind.resolve(c.cand, ctx);
        if (!resolved) {
            trace.set(c, "no such entity");
            continue;
        }
        plain(c.cand.start);
        const link: Segment = {
            type: "link",
            kind: resolved.kind ?? c.kind.id,
            label: text.slice(c.cand.start, c.cand.end),
            destination: resolved.destination,
            state: resolved.state,
            name: resolved.name,
            ...(resolved.reason ? { reason: resolved.reason } : {}),
        };
        segments.push(link);
        at = c.cand.end;
        trace.set(c, "linked");
    }
    plain(text.length);
    return segments;
}

export function runLinkify(text: string, ctx: LinkContext): LinkifyRun {
    if (!text || text.length > MAX_LENGTH) return { segments: text ? [{ type: "text", text }] : [], trace: [] };
    const considered = recogniseAll(text, ctx, findProtected(text));
    const { chosen, lost } = choose(considered);
    const outcome = new Map<Considered, string>();
    for (const c of considered) if (c.skip) outcome.set(c, c.skip);
    for (const [c, why] of lost) outcome.set(c, why);
    const segments = emit(text, chosen, ctx, outcome);
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
