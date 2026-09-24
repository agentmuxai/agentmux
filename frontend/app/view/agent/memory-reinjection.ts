// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Pure computation for hidden memory reinjection after compaction. See
 * docs/specs/SPEC_HIDDEN_MEMORY_REINJECTION_AFTER_COMPACTION_2026_09_22.md.
 *
 * Everything in this file is pure and side-effect-free by design — no MCP
 * calls, no reducer dispatch, no send-path wiring. Those live at the call
 * site (not yet built — see the spec's §2 "assumed, not verified" list);
 * this module is the foundation they compose on top of, mirroring
 * compact-boundary.ts's own live/replay-shared extraction (§3.2 of the
 * spec cites the same rationale: two consumers must not be able to drift
 * on what counts as valid input).
 *
 * `sizeBytes` on every entry is ALWAYS caller-supplied, never derived from
 * `body.length` — mirrors agent_native_memory.rs's own real-size-vs-
 * content-length distinction (a file's real on-disk size can diverge from
 * its in-memory string length; deriving one from the other silently loses
 * that fidelity, exactly the bug agent_native_memory_upsert's own doc
 * comment guards against for the same reason).
 */

import { estimateTokenCount } from "@/util/format-count";
import type { MemoryReinjectionNode } from "./types";

export interface MemoryEntryInput {
    label: string;
    source: "global" | "personal";
    /** Full body — sent to the model verbatim, never truncated. Never stored in the resulting node (§3.2's hiding mechanism). */
    body: string;
    /** Real on-disk size in bytes — caller-supplied, see module doc comment. */
    sizeBytes: number;
}

export type SizeBand = "low" | "mid" | "high" | "critical";

/** Proposed starting default — see spec §3.4.2 for the real-data reasoning behind 0.10, and §3.4.4 for why it's tunable, not empirically settled. */
export const DEFAULT_MEMORY_SIZE_FRACTION = 0.1;

/**
 * Bands Personal memory's estimated token total against a FRACTION OF THE
 * PANE'S CONTEXT WINDOW — same four bands, same fractional boundaries
 * (0.5 / 0.75 / 0.9) as `AgentComposerStrip.tsx`'s `ctxBand()`, and
 * (revised 2026-09-22, second pass) the same DENOMINATOR CHOICE: banding
 * against a real per-pane reference point rather than an arbitrary absolute
 * number. The same memory size reads differently on a 32K-context pane than
 * a 200K-context one, by design — see spec §3.4.2.
 */
export function memorySizeBand(
    personalEstimatedTokens: number,
    contextWindow: number,
    fraction: number = DEFAULT_MEMORY_SIZE_FRACTION,
): SizeBand {
    const healthyThreshold = contextWindow * fraction;
    const f = personalEstimatedTokens / healthyThreshold;
    if (f >= 0.9) return "critical";
    if (f >= 0.75) return "high";
    if (f >= 0.5) return "mid";
    return "low";
}

/**
 * §3.1 suppression rule: nothing to inject, nothing sent. Avoids a pointless
 * turn (and its token cost) when both memory sources are empty.
 */
export function shouldReinject(entries: MemoryEntryInput[]): boolean {
    return entries.length > 0;
}

/**
 * Why this particular reinjection turn is firing — both are the same
 * underlying situation from the model's perspective ("what I had in context
 * is gone"), but the specifics differ enough to be worth telling the model:
 * a compaction leaves a summary behind, a fresh session leaves nothing at
 * all. See `composeReinjectionMessage`'s doc comment and
 * SPEC_HIDDEN_MEMORY_REINJECTION_AFTER_COMPACTION_2026_09_22.md §3.3's
 * "fresh session" addendum (§3.3a).
 */
export type ReinjectionReason = "compaction" | "fresh_session";

const REASON_CLAUSE: Record<ReinjectionReason, string> = {
    compaction: "Your recent conversation was just compacted into a summary.",
    fresh_session:
        "AgentMux could not resume this agent's prior session, so a fresh one was started. Any record of the prior conversation AgentMux had came with your first message, in an <agentmux-continuation> block.",
};

/**
 * Builds the hidden message actually sent to the model — §3.1's template.
 * Every entry's FULL body is included verbatim; this function must never
 * truncate or summarize (the "has to read it all" requirement is the
 * entire point of this feature — a truncating implementation would silently
 * defeat it). A source section is omitted entirely when that source has
 * zero entries, rather than printing an empty "(0 entries)" header.
 *
 * `reason` only changes the second sentence (`REASON_CLAUSE`) — the leading
 * `REINJECTION_SIGNATURE` sentence is identical regardless, since it is what
 * both `isMemoryReinjectionMessage` (TypeScript) and `is_hidden_reinjection_
 * text` (Rust, `agentmux-srv/src/server/app_api/session.rs`) key detection
 * on; a per-reason signature would mean two strings to keep in sync on both
 * sides instead of one.
 */
export function composeReinjectionMessage(entries: MemoryEntryInput[], reason: ReinjectionReason): string {
    const globalEntries = entries.filter((e) => e.source === "global");
    const personalEntries = entries.filter((e) => e.source === "personal");

    const renderSection = (label: string, section: MemoryEntryInput[]): string => {
        if (section.length === 0) return "";
        const noun = section.length === 1 ? "entry" : "entries";
        const bodies = section.map((e) => e.body).join("\n---\n");
        return `# ${label} (${section.length} ${noun})\n${bodies}\n`;
    };

    const sections = [renderSection("Global Memory", globalEntries), renderSection("Personal Memory", personalEntries)]
        .filter((s) => s.length > 0)
        .join("\n");

    return (
        "<system-reminder>\n" +
        `${REINJECTION_SIGNATURE} ${REASON_CLAUSE[reason]} Below is your\n` +
        "complete Global Memory and Personal Memory content — read all of it now,\n" +
        "not just the index.\n\n" +
        sections +
        "</system-reminder>\n"
    );
}

/** The literal sentence `composeReinjectionMessage` always emits regardless of `reason` — the recognition signature `isMemoryReinjectionMessage`/`parseReinjectionMessage`/Rust's `is_hidden_reinjection_text` key on. */
const REINJECTION_SIGNATURE = "Your memory was reinjected because your working context was just reset.";

/**
 * True if `text` is a message `composeReinjectionMessage` produced —
 * recognizes a hidden reinjection turn re-encountered on HISTORY REPLAY
 * (`stream-parser.ts`'s `tryParseMemoryReinjection`, mirroring the
 * already-established `tryParseJekt` pattern for the same "special marker
 * occupying the whole user-message payload" shape). The live path never
 * needs this — it already knows a send is a reinjection at the moment it
 * makes it (`memory-reinjection-controller.ts`). This function exists
 * purely for the replay side of §3.2's "History replay" requirement.
 */
export function isMemoryReinjectionMessage(text: string): boolean {
    return text.startsWith("<system-reminder>") && text.includes(REINJECTION_SIGNATURE);
}

/**
 * Best-effort reconstruction of a `MemoryReinjectionNode`'s summary fields
 * from a replayed reinjection message's raw text — NOT a perfect inverse of
 * `composeReinjectionMessage`. Two things are genuinely unrecoverable from
 * the message text alone, and this function degrades honestly rather than
 * fabricating them:
 *
 * - **Per-entry labels.** `composeReinjectionMessage` never writes them
 *   into the message (only raw bodies, joined by `\n---\n`) — replayed
 *   entries get synthetic `"Entry N"` labels instead of their real names.
 * - **Real on-disk `sizeBytes`.** The live node's byte totals come from
 *   `agent_native_memory`'s stored `size_bytes` / Global Memory's
 *   `instructions.length` at send time (§3.4.1) — on replay, only the
 *   recovered text chunk's own `.length` is available, which is a real
 *   approximation (not necessarily equal to the original on-disk size).
 *
 * Splitting on `\n---\n` is itself approximate: a body that happens to
 * contain that exact literal sequence on its own line would be split
 * mid-entry. Acceptable for a replay DISPLAY approximation (this feeds a
 * label-only row, never anything sent back to a model) — flagged here so
 * it isn't mistaken for a guarantee.
 *
 * Returns `null` if `text` doesn't match the recognized shape at all.
 */
export function parseReinjectionMessage(
    text: string,
): { globalMemoryCount: number; personalMemoryCount: number; perEntryTokens: MemoryReinjectionNode["perEntryTokens"] } | null {
    if (!isMemoryReinjectionMessage(text)) return null;

    const parseSection = (label: "global" | "personal", heading: string): MemoryReinjectionNode["perEntryTokens"] => {
        const re = new RegExp(`# ${heading} \\(\\d+ entr(?:y|ies)\\)\\n([\\s\\S]*?)(?:\\n# |\\n?</system-reminder>)`);
        const match = re.exec(text);
        if (!match) return [];
        const chunks = match[1].split("\n---\n").filter((c) => c.trim().length > 0);
        return chunks.map((body, i) => ({
            label: `Entry ${i + 1}`,
            source: label,
            tokens: estimateTokenCount(body),
            sizeBytes: body.length,
        }));
    };

    const globalEntries = parseSection("global", "Global Memory");
    const personalEntries = parseSection("personal", "Personal Memory");

    return {
        globalMemoryCount: globalEntries.length,
        personalMemoryCount: personalEntries.length,
        perEntryTokens: [...globalEntries, ...personalEntries],
    };
}

/**
 * Dedup key — exactly one reinjection per real `compact_boundary`, keyed
 * the same way `compact-boundary.ts`'s `contextCompactedNodeId` already
 * dedups: by the frame's own `timestamp`, never a client-generated one, so
 * a live delivery and a later history-replay overlap of the same boundary
 * can't fire this twice. See spec §3.1/§3.2.
 */
export function memoryReinjectionNodeId(frameTimestamp: string | null): string {
    return `memory-reinjected-${frameTimestamp ?? "notime"}`;
}

/**
 * Fallback context window (tokens) when no model id has been observed yet.
 * 200K matches Haiku's real context window — the smaller of the two values
 * this codebase's own TokensIn handling cites for Claude ("Opus/Sonnet 1M,
 * Haiku 200K") — so an unknown model under-estimates its budget rather than
 * over-estimates it: the safer failure direction for a size-band warning
 * (reads as more urgent than reality, never less). Shared by the live send
 * path (`useAgentStream.ts`) and the replay reconstruction below so the two
 * can't independently drift on the same fallback value.
 */
export const FALLBACK_CONTEXT_WINDOW = 200_000;

/**
 * Reconstructs a `MemoryReinjectionNode` from a replayed reinjection
 * message — the "History replay" half of §3.2, using
 * `parseReinjectionMessage`'s best-effort extraction (see that function's
 * doc comment for exactly what's approximated: synthetic per-entry labels,
 * text-length-derived `sizeBytes` rather than real on-disk sizes).
 * `contextWindow` isn't reliably known at replay time the way it is for a
 * live send, so `sizeBand` uses `FALLBACK_CONTEXT_WINDOW` unless the caller
 * has something better — the replayed band may not exactly match what was
 * shown live, which is an accepted, documented approximation, not a
 * correctness bug in the underlying mechanism.
 *
 * Returns `null` when `text` isn't a recognized reinjection message —
 * callers should fall through to normal user-message handling in that case.
 */
export function buildMemoryReinjectionNodeFromReplay(
    text: string,
    opts: { eventTimestamp: number; contextWindow?: number },
): MemoryReinjectionNode | null {
    const parsed = parseReinjectionMessage(text);
    if (!parsed) return null;

    const estimatedTokens = parsed.perEntryTokens.reduce((sum, e) => sum + e.tokens, 0);
    const totalSizeBytes = {
        global: parsed.perEntryTokens.filter((e) => e.source === "global").reduce((sum, e) => sum + e.sizeBytes, 0),
        personal: parsed.perEntryTokens.filter((e) => e.source === "personal").reduce((sum, e) => sum + e.sizeBytes, 0),
    };
    const personalEstimatedTokens = parsed.perEntryTokens
        .filter((e) => e.source === "personal")
        .reduce((sum, e) => sum + e.tokens, 0);

    return {
        type: "memory_reinjection",
        // Keyed on this event's own timestamp, not a compact_boundary's —
        // that frame isn't available to a replay parse of just this one
        // user-message line. Acceptable: parseHistoryLines.ts processes the
        // full historical file once per mount, not incrementally alongside
        // a live session, so this id only needs to be stable WITHIN one
        // replay pass, not necessarily identical to whatever id the live
        // path once produced for the same turn.
        id: `memory-reinjected-replay-${opts.eventTimestamp}`,
        globalMemoryCount: parsed.globalMemoryCount,
        personalMemoryCount: parsed.personalMemoryCount,
        estimatedTokens,
        perEntryTokens: parsed.perEntryTokens,
        totalSizeBytes,
        sizeBand: memorySizeBand(personalEstimatedTokens, opts.contextWindow ?? FALLBACK_CONTEXT_WINDOW),
        at: opts.eventTimestamp,
    };
}

export interface BuildMemoryReinjectionNodeOptions {
    /** The `compact_boundary` frame's own `timestamp` field — see memoryReinjectionNodeId. */
    frameTimestamp: string | null;
    /** Fallback `at` when frameTimestamp is absent/unparseable — caller's Date.now() at receipt. */
    now: number;
    /** §3.4.2 — the pane's own context window; memorySizeBand bands Personal-memory estimated tokens against `contextWindow * fraction`. */
    contextWindow: number;
    /** Overrides memorySizeBand's default fraction (§3.4.2/§3.4.4) — rarely needed, present for testability/tuning. */
    sizeBandFraction?: number;
}

/**
 * Computes the label-only `MemoryReinjectionNode` for the transcript from a
 * real entry list — never includes body text, per §3.2's hiding mechanism.
 * Callers gate on `shouldReinject(entries)` before calling this; this
 * function does not itself suppress on an empty entries array (that's a
 * distinct concern — whether to send at all — from what to compute once
 * the decision to send has already been made).
 */
export function buildMemoryReinjectionNode(
    entries: MemoryEntryInput[],
    opts: BuildMemoryReinjectionNodeOptions,
): MemoryReinjectionNode {
    const globalEntries = entries.filter((e) => e.source === "global");
    const personalEntries = entries.filter((e) => e.source === "personal");

    const perEntryTokens = entries.map((e) => ({
        label: e.label,
        source: e.source,
        tokens: estimateTokenCount(e.body),
        sizeBytes: e.sizeBytes,
    }));

    const totalSizeBytes = {
        global: globalEntries.reduce((sum, e) => sum + e.sizeBytes, 0),
        personal: personalEntries.reduce((sum, e) => sum + e.sizeBytes, 0),
    };

    const estimatedTokens = perEntryTokens.reduce((sum, e) => sum + e.tokens, 0);
    const personalEstimatedTokens = perEntryTokens
        .filter((e) => e.source === "personal")
        .reduce((sum, e) => sum + e.tokens, 0);

    // Banded on Personal's estimated tokens ALONE, against the pane's own
    // context window — §3.4.2's explicit rule, not the combined total and
    // not an arbitrary absolute number. See the "bands sizeBand on PERSONAL
    // estimated tokens alone" test.
    const sizeBand = memorySizeBand(personalEstimatedTokens, opts.contextWindow, opts.sizeBandFraction);

    const parsedFrameAt = opts.frameTimestamp != null ? Date.parse(opts.frameTimestamp) : NaN;
    const at = Number.isNaN(parsedFrameAt) ? opts.now : parsedFrameAt;

    return {
        type: "memory_reinjection",
        id: memoryReinjectionNodeId(opts.frameTimestamp),
        globalMemoryCount: globalEntries.length,
        personalMemoryCount: personalEntries.length,
        estimatedTokens,
        perEntryTokens,
        totalSizeBytes,
        sizeBand,
        at,
    };
}
