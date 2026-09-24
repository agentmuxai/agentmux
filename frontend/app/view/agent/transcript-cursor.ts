// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The agent pane's position in its transcript stream — the consumer side of
 * Phase 5a (docs/specs/SPEC_AGENT_PANE_BOUNDED_LIVE_WINDOW_MIGRATION_2026_09_23.md
 * §6.3.7 item 7).
 *
 * Every transcript append event names where its records landed, per stream
 * (`pos: [{ stream, gen, line, lines }]`). The cursor pins one stream — the
 * one the pane's history reads were served from — and keeps `next`, the next
 * line it expects:
 *
 * - `line < next`: already seen (a duplicate), dropped.
 * - `line == next`: delivered to the parser.
 * - `line > next`: a gap — lines another writer appended (another block of
 *   the same agent, another srv instance), or events lost while the socket
 *   was down. The cursor reads `[next, line)` with `expectGen`, delivers it,
 *   then the event. Events arriving meanwhile wait behind the read, so the
 *   parser always sees lines in order.
 *
 * Until the history load says where it ended (`settle`), events are held:
 * delivering them first would put live records above the history they
 * follow, and history covering them would repeat them.
 *
 * Anything the cursor can't place — an event without a position for the
 * pinned stream (an uncounted file, a failed mirror) — is delivered as
 * before 5a, and counted.
 *
 * Pure (no Solid, no RPC): `useAgentStream` supplies the effects.
 */

import { base64ToArray } from "@/util/util";

/** Where one append's records landed in one stream: lines `line .. lines`. */
export interface StreamPos {
    stream: string;
    gen: string;
    line: number;
    lines: number;
}

/** The part of a `blockfile` event the cursor reads. */
export interface TranscriptFileEvent {
    fileop: string;
    data64?: string;
    pos?: StreamPos[];
    echo?: string;
}

/** Where the history load ended: it showed lines `< next` of `stream`/`gen`. */
export interface CursorPin {
    stream: string;
    gen: string;
    next: number;
}

export interface CursorReadResult {
    lines: string[];
    stream?: string;
    gen?: string;
    genMismatch?: boolean;
}

export interface TranscriptCursorDeps {
    /** Parse complete records (newline-terminated) as live input. */
    deliver(text: string): void;
    /**
     * An echo's records (the user's own message, already on screen). Not
     * parsed; handed over so the pane can pair it with the node it echoes.
     */
    echo(text: string): void;
    /**
     * Whether a line read to fill a gap is the echo of a message this pane
     * already shows (its echo event was lost). Such a line is dropped.
     */
    isOwnEcho(line: string): boolean;
    /** A `truncate`, `replace` or `delete` of the stream: today's reset. */
    reset(fileop: "truncate" | "replace" | "delete"): void;
    /** Read lines `[offset, offset + limit)` of generation `expectGen`. */
    readRange(offset: number, limit: number, expectGen: string): Promise<CursorReadResult>;
    log(message: string, level?: "warn"): void;
}

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

/**
 * Largest gap filled by reading. A bigger one (a pane waking after hours
 * behind a busy shared zone) is skipped: its lines stay on disk and appear on
 * the next load, rather than parsing thousands of lines at once.
 */
export const GAP_FILL_MAX_LINES = 5_000;
/** Lines per read while filling a gap. */
export const GAP_FILL_CHUNK_LINES = 1_000;

type Item = { kind: "event"; ev: TranscriptFileEvent } | { kind: "count"; count: number; stream: string; gen: string };

/**
 * What the cursor knows before its first positioned event, when the history
 * load didn't name a position: `empty` — history showed nothing, so a first
 * event past line 0 is a gap; `unknown` — no idea, start at the first event.
 */
type Unpinned = "empty" | "unknown";

export class TranscriptCursor {
    private pin: CursorPin | null = null;
    private unpinned: Unpinned = "unknown";
    private settled = false;
    private queue: Item[] = [];
    private draining = false;
    private disposed = false;
    readonly stats: TranscriptCursorStats = {
        delivered: 0,
        duplicates: 0,
        gapsFilled: 0,
        gapLinesFilled: 0,
        linesSkipped: 0,
        unpositioned: 0,
        genChanges: 0,
        echoes: 0,
        ownEchoesDropped: 0,
    };

    constructor(private readonly deps: TranscriptCursorDeps) {}

    /** The pinned position, for tests and diagnostics. */
    position(): Readonly<CursorPin> | null {
        return this.pin ? { ...this.pin } : null;
    }

    isSettled(): boolean {
        return this.settled;
    }

    /**
     * The history load finished. `pin`: it showed lines `< next` of that
     * stream and generation. `"empty"`: it showed nothing (an empty or
     * missing file). `null`: it can't say (failed, or a snapshot without
     * line positions). Only the first call counts.
     */
    settle(pin: CursorPin | "empty" | null): void {
        if (this.settled || this.disposed) return;
        this.settled = true;
        if (pin && pin !== "empty") {
            this.pin = { ...pin };
        } else {
            this.unpinned = pin === "empty" ? "empty" : "unknown";
        }
        this.drain();
    }

    push(ev: TranscriptFileEvent): void {
        if (this.disposed) return;
        this.queue.push({ kind: "event", ev });
        this.drain();
    }

    /**
     * A line count read for the pinned stream (the poll): lines below
     * `count` that no event brought are read now. Lines another writer
     * appends to a shared zone arrive this way.
     */
    observeCount(count: number, stream: string, gen: string): void {
        if (this.disposed) return;
        this.queue.push({ kind: "count", count, stream, gen });
        this.drain();
    }

    dispose(): void {
        this.disposed = true;
        this.queue = [];
    }

    /**
     * Handles queued items in order. Synchronous until an item needs a read;
     * then the rest wait for it, and draining resumes when it settles.
     */
    private drain(): void {
        if (this.draining || !this.settled) return;
        this.draining = true;
        while (this.queue.length > 0 && !this.disposed) {
            const item = this.queue.shift()!;
            let pending: Promise<void> | void;
            try {
                pending = item.kind === "event" ? this.handleEvent(item.ev) : this.handleCount(item);
            } catch (err) {
                // A parser throw mustn't wedge the queue: the item is consumed
                // (its lines were counted before delivery), the rest go on.
                this.logError(err);
                continue;
            }
            if (pending) {
                pending.catch((err) => this.logError(err)).finally(() => {
                    this.draining = false;
                    this.drain();
                });
                return;
            }
        }
        this.draining = false;
    }

    private logError(err: unknown): void {
        this.deps.log(`transcript cursor: ${(err as any)?.message ?? String(err)}`, "warn");
    }

    private handleEvent(ev: TranscriptFileEvent): Promise<void> | void {
        if (ev.fileop === "truncate" || ev.fileop === "delete") {
            this.deps.reset(ev.fileop);
            // The stream starts again from nothing.
            this.pin = null;
            this.unpinned = "empty";
            return;
        }
        if (ev.fileop === "replace") {
            this.deps.reset("replace");
            // The new content is history, not live: join after it.
            const p = this.pick(ev.pos);
            this.pin = p ? { stream: p.stream, gen: p.gen, next: p.lines } : null;
            this.unpinned = "unknown";
            return;
        }
        if (ev.fileop !== "append" || !ev.data64) return;

        let p = this.pick(ev.pos);
        if (!p) {
            this.stats.unpositioned++;
            if (ev.echo) {
                this.stats.echoes++;
                this.deps.echo(decode(ev.data64));
                return;
            }
            this.deliver(decode(ev.data64));
            return;
        }

        if (!this.pin) {
            this.pin = { stream: p.stream, gen: p.gen, next: this.unpinned === "empty" ? 0 : p.line };
        } else {
            p = this.migrate(ev.pos!, p);
            if (p.gen !== this.pin.gen) {
                // Another file, as far as the cursor can tell: replaced or
                // rewritten by a writer whose event this pane didn't get
                // (`agent:session:archive` deletes a shared zone with no block
                // to announce it on). An older build's appends no longer
                // change the generation (the counter is caught up, not
                // re-counted), and generation and position alone can't tell a
                // rare re-count from a replacement (Codex on #3663) — so
                // nothing is carried across: the cursor joins at this event,
                // and never fills from the new file at the old one's line.
                this.stats.genChanges++;
                this.deps.log(`transcript cursor: ${p.stream} generation changed ${this.pin.gen} → ${p.gen} (joining at line ${p.line})`);
                this.pin = { stream: p.stream, gen: p.gen, next: p.line };
            }
        }

        const pin = this.pin;
        if (p.lines <= pin.next) {
            this.stats.duplicates++;
            return;
        }
        if (p.line > pin.next) {
            const at = p;
            return this.fill(pin.next, p.line).then(() => this.apply(ev, at));
        }
        this.apply(ev, p);
    }

    private handleCount(item: { count: number; stream: string; gen: string }): Promise<void> | void {
        const pin = this.pin;
        // A count in another generation is another file's: the next event
        // joins it (handleEvent). Only the pinned file's lines are filled.
        if (!pin || item.stream !== pin.stream || item.gen !== pin.gen) return;
        if (item.count <= pin.next) return;
        return this.fill(pin.next, item.count);
    }

    /**
     * The event's records, less any the cursor already has (a gap read can
     * run into an event whose own delivery was still in flight).
     */
    private apply(ev: TranscriptFileEvent, p: StreamPos): void {
        const pin = this.pin;
        if (this.disposed) return;
        // A reset or generation change while the gap was read: this event's
        // position no longer describes the pinned stream.
        if (!pin || pin.stream !== p.stream || pin.gen !== p.gen) return;
        if (p.lines <= pin.next) {
            this.stats.duplicates++;
            return;
        }
        let text = decode(ev.data64!);
        if (p.line < pin.next) {
            const records = splitRecords(text).slice(pin.next - p.line);
            text = records.length > 0 ? records.join("\n") + "\n" : "";
        }
        pin.next = p.lines;
        if (!text) return;
        if (ev.echo) {
            this.stats.echoes++;
            this.deps.echo(text);
            return;
        }
        this.deliver(text);
    }

    /** Reads and delivers lines `[from, to)` of the pinned generation. */
    private async fill(from: number, to: number): Promise<void> {
        const pin = this.pin!;
        const { stream, gen } = pin;
        const skip = (why: string) => {
            // Still the same pin? Then move past the lines it can't have.
            if (this.pin === pin && pin.next < to) {
                this.stats.linesSkipped += to - pin.next;
                pin.next = to;
            }
            this.deps.log(`transcript cursor: skipped ${stream} lines [${from}, ${to}): ${why}`, "warn");
        };
        if (to - from > GAP_FILL_MAX_LINES) {
            skip(`gap of ${to - from} lines is over ${GAP_FILL_MAX_LINES}`);
            return;
        }
        let at = from;
        let filled = 0;
        while (at < to) {
            let res: CursorReadResult;
            try {
                res = await this.deps.readRange(at, Math.min(GAP_FILL_CHUNK_LINES, to - at), gen);
            } catch (err: any) {
                skip(`read failed: ${err?.message ?? String(err)}`);
                return;
            }
            if (this.disposed || this.pin !== pin) return;
            if (res.genMismatch || res.stream !== stream || res.gen !== gen) {
                skip(`read served ${res.stream ?? "?"}/${res.gen ?? "?"}${res.genMismatch ? " (generation gone)" : ""}`);
                return;
            }
            if (res.lines.length === 0) {
                skip("read returned no lines");
                return;
            }
            const keep: string[] = [];
            for (const line of res.lines) {
                if (this.deps.isOwnEcho(line)) {
                    this.stats.ownEchoesDropped++;
                } else {
                    keep.push(line);
                }
            }
            // An event delivered meanwhile can't have moved `next`: events
            // wait behind this read. Only the lines past it are new.
            const lines = res.lines.length;
            at += lines;
            filled += lines;
            pin.next = Math.max(pin.next, at);
            if (keep.length > 0) this.deliver(keep.join("\n") + "\n");
        }
        this.stats.gapsFilled++;
        this.stats.gapLinesFilled += filled;
    }

    private deliver(text: string): void {
        this.stats.delivered++;
        this.deps.deliver(text);
    }

    /**
     * The event's position in the pinned stream, or — before a pin — in the
     * stream the pane's reads are served from: the agent's global zone when
     * the event has one, else the block's own file.
     */
    private pick(pos: StreamPos[] | undefined): StreamPos | null {
        if (!pos || pos.length === 0) return null;
        if (this.pin) return pos.find((p) => p.stream === this.pin!.stream) ?? null;
        return pos.find((p) => p.stream.startsWith("g:")) ?? pos.find((p) => p.stream.startsWith("b:")) ?? null;
    }

    /**
     * Pinned to the block's own file, and the agent's global zone now has
     * content too (its first write happened after this pane loaded): reads
     * are served from the global zone from now on, so follow it. Only at a
     * contiguous point, so nothing between is lost.
     */
    private migrate(pos: StreamPos[], p: StreamPos): StreamPos {
        const pin = this.pin!;
        if (!pin.stream.startsWith("b:") || p.gen !== pin.gen || p.line !== pin.next) return p;
        const g = pos.find((q) => q.stream.startsWith("g:"));
        if (!g) return p;
        this.deps.log(`transcript cursor: following ${g.stream} from line ${g.line} (was ${pin.stream})`);
        this.pin = { stream: g.stream, gen: g.gen, next: g.line };
        return g;
    }
}

/** Most unmatched texts an `EchoLedger` keeps, per side. */
const ECHO_LEDGER_MAX = 64;

/**
 * Pairs the user messages this pane shows (accepted, optimistic nodes) with
 * their transcript records, by text. Only needed when a record's echo event
 * was lost (a socket drop) and the record comes back through a gap read: it
 * must not become a second node. 5b gives the pairing positional ids; until
 * then equal text is the match, which errs toward dropping a record that
 * already has a node with the same text on screen.
 */
export class EchoLedger {
    /** Accepted messages whose record hasn't been seen yet. */
    private shown: string[] = [];
    /** Echoes seen before their message was accepted. */
    private early: string[] = [];

    accepted(text: string): void {
        if (takeFirst(this.early, text)) return;
        pushBounded(this.shown, text);
    }

    /** An echo event's records. */
    echoed(records: string): void {
        for (const line of splitRecords(records)) {
            const text = userText(line);
            if (text === null) continue;
            if (!takeFirst(this.shown, text)) pushBounded(this.early, text);
        }
    }

    /** A gap line: the record of a message already shown? Consumes the match. */
    isOwnEcho(line: string): boolean {
        const text = userText(line);
        return text !== null && takeFirst(this.shown, text);
    }
}

/** The text of a stdin user record (`{"type":"user","message":{"content":"…"}}`). */
function userText(line: string): string | null {
    if (!line.includes('"user"')) return null;
    try {
        const rec = JSON.parse(line);
        const content = rec?.type === "user" ? rec.message?.content : undefined;
        return typeof content === "string" ? content : null;
    } catch {
        return null;
    }
}

function takeFirst(list: string[], text: string): boolean {
    const i = list.indexOf(text);
    if (i < 0) return false;
    list.splice(i, 1);
    return true;
}

function pushBounded(list: string[], text: string): void {
    list.push(text);
    if (list.length > ECHO_LEDGER_MAX) list.shift();
}

export type HistorySettlement = CursorPin | "empty" | null;

/**
 * Carries the history load's outcome to the stream hook: the two hooks mount
 * independently, and either may be first. `settle` keeps the first value;
 * `onSettle` runs its callback at once if the value is already known.
 */
export interface TranscriptSettleLatch {
    settle(outcome: HistorySettlement): void;
    onSettle(cb: (outcome: HistorySettlement) => void): () => void;
}

export function createTranscriptSettleLatch(): TranscriptSettleLatch {
    let settled = false;
    let value: HistorySettlement = null;
    const waiters = new Set<(outcome: HistorySettlement) => void>();
    return {
        settle(outcome) {
            if (settled) return;
            settled = true;
            value = outcome;
            for (const cb of [...waiters]) cb(value);
            waiters.clear();
        },
        onSettle(cb) {
            if (settled) {
                cb(value);
                return () => {};
            }
            waiters.add(cb);
            return () => waiters.delete(cb);
        },
    };
}

/**
 * Where a history read leaves the cursor: the lines it showed end at
 * `min(offset, total) + lines.length`. Only a read that names its stream and
 * generation can say — anything else is `null` (the cursor starts at the
 * first live event).
 */
export function historyPin(
    offset: number,
    resp: { lines?: string[] | null; total?: number; stream?: string; gen?: string } | null | undefined,
): CursorPin | null {
    if (!resp?.stream || !resp.gen) return null;
    const start = typeof resp.total === "number" ? Math.min(offset, resp.total) : offset;
    return { stream: resp.stream, gen: resp.gen, next: start + (resp.lines?.length ?? 0) };
}

function decode(data64: string): string {
    return new TextDecoder().decode(base64ToArray(data64));
}

/** Non-blank lines, the rule the backend counts by (`filestore/lines.rs`). */
export function splitRecords(text: string): string[] {
    return text.split("\n").filter((l) => l.trim() !== "");
}
