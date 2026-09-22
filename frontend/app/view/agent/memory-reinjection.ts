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

/**
 * Bands a byte count against a threshold — same four bands, same fractional
 * boundaries (0.5 / 0.75 / 0.9) as `AgentComposerStrip.tsx`'s `ctxBand()`,
 * deliberately reused rather than re-decided. See spec §3.4.2.
 */
export function memorySizeBand(bytes: number, healthyThreshold: number): SizeBand {
    const fraction = bytes / healthyThreshold;
    if (fraction >= 0.9) return "critical";
    if (fraction >= 0.75) return "high";
    if (fraction >= 0.5) return "mid";
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
 * Builds the hidden message actually sent to the model — §3.1's template.
 * Every entry's FULL body is included verbatim; this function must never
 * truncate or summarize (the "has to read it all" requirement is the
 * entire point of this feature — a truncating implementation would silently
 * defeat it). A source section is omitted entirely when that source has
 * zero entries, rather than printing an empty "(0 entries)" header.
 */
export function composeReinjectionMessage(entries: MemoryEntryInput[]): string {
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
        "Your memory was reinjected after a context compaction. Below is your\n" +
        "complete Global Memory and Personal Memory content — read all of it now,\n" +
        "not just the index, since your recent working context was just summarized.\n\n" +
        sections +
        "</system-reminder>\n"
    );
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

export interface BuildMemoryReinjectionNodeOptions {
    /** The `compact_boundary` frame's own `timestamp` field — see memoryReinjectionNodeId. */
    frameTimestamp: string | null;
    /** Fallback `at` when frameTimestamp is absent/unparseable — caller's Date.now() at receipt. */
    now: number;
    /** §3.4.2 — the denominator memorySizeBand bands Personal-memory bytes against. */
    healthyThreshold: number;
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

    // Banded on Personal bytes ALONE — §3.4.2's explicit rule, not the
    // combined total. See the "bands sizeBand on PERSONAL bytes alone" test.
    const sizeBand = memorySizeBand(totalSizeBytes.personal, opts.healthyThreshold);

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
