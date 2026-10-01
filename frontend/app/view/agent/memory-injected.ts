// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The card for memory delivered through Claude Code's `SessionStart` hook
 * (docs/specs/SPEC_GLOBAL_MEMORY_DELIVERY_2026_09_27.md §7 P2). The sidecar
 * writes one persisted `agentmux_memory_injected` frame into the pane's
 * output once every part of a delivery reached the CLI
 * (`server/memory_delivery_handlers.rs`). The model already has the content;
 * the frame says what it was given, one item per entry, and this turns it
 * into a context delivery card with a row per item
 * (docs/specs/SPEC_CONTEXT_DELIVERY_2026_09_30.md §3.4, CD2a).
 *
 * Frames written before CD2a carry only `label` and `source`; their names and
 * tiers are recovered from the label (spec §4).
 *
 * Shared by the live stream (`useAgentStream.ts`) and history replay
 * (`parseHistoryLines.ts`): the node id is the frame's own `id`, stable for
 * the delivery, so a live frame and a replay of the same line render once.
 */

import { FALLBACK_CONTEXT_WINDOW, memorySizeBand } from "./memory-reinjection";
import type { ContextDeliveryNode, ContextItem } from "./types";

export const MEMORY_INJECTED_SUBTYPE = "agentmux_memory_injected";

type RawFrame = Record<string, unknown>;

export function isMemoryInjectedFrame(rawEvent: unknown): boolean {
    if (!rawEvent || typeof rawEvent !== "object") return false;
    const e = rawEvent as RawFrame;
    return e.type === "system" && e.subtype === MEMORY_INJECTED_SUBTYPE;
}

const REASON: Record<string, ContextDeliveryNode["reason"]> = {
    startup: "startup",
    clear: "clear",
    compact: "compaction",
};

const KINDS: ReadonlyArray<ContextItem["kind"]> = ["global_memory", "personal_memory", "running_summary", "startup_file"];
const OWNERS: ReadonlyArray<NonNullable<ContextItem["owner"]>> = ["agentmux", "user", "external"];

const SYSTEM_PREFIX = "[AgentMux System] ";
const WORKSPACE_PREFIX = "[Workspace] ";

const str = (v: unknown): string | undefined => (typeof v === "string" && v.length > 0 ? v : undefined);
const num = (v: unknown): number => (typeof v === "number" && Number.isFinite(v) ? v : 0);

/** One notice entry as a card item. Old frames: name and tier from the label. */
function toItem(x: RawFrame): ContextItem {
    const label = str(x.label) ?? "";
    const kind: ContextItem["kind"] = KINDS.includes(x.kind as ContextItem["kind"])
        ? (x.kind as ContextItem["kind"])
        : x.source === "personal"
          ? "personal_memory"
          : x.source === "summary"
            ? "running_summary"
            : "global_memory";

    let name = str(x.name);
    let tier: ContextItem["tier"] = x.tier === "system" || x.tier === "workspace" ? x.tier : undefined;
    if (kind === "global_memory" && (!name || !tier)) {
        if (label.startsWith(SYSTEM_PREFIX)) {
            name ??= label.slice(SYSTEM_PREFIX.length);
            tier ??= "system";
        } else if (label.startsWith(WORKSPACE_PREFIX)) {
            name ??= label.slice(WORKSPACE_PREFIX.length);
            tier ??= "workspace";
        }
    }
    const delivered = x.delivered === "partial" || x.delivered === "omitted" ? x.delivered : undefined;

    return {
        kind,
        name: name ?? label,
        ...(tier ? { tier } : {}),
        ...(str(x.path) ? { path: str(x.path) } : {}),
        ...(str(x.bundle_id) ? { bundleId: str(x.bundle_id) } : {}),
        ...(delivered ? { delivered } : {}),
        sizeBytes: num(x.size_bytes),
        tokens: num(x.tokens),
        ...(typeof x.source_tokens === "number" ? { sourceTokens: num(x.source_tokens) } : {}),
        ...(kind === "startup_file" ? startupFields(x) : {}),
    };
}

/** A `startup_file` item's role, owner, count and what it contains. */
function startupFields(x: RawFrame): Partial<ContextItem> {
    const owner = OWNERS.find((o) => o === x.owner);
    const contains = Array.isArray(x.contains) ? x.contains.filter((c): c is string => typeof c === "string") : [];
    return {
        ...(str(x.role) ? { role: str(x.role) } : {}),
        ...(owner ? { owner } : {}),
        ...(typeof x.count === "number" ? { count: x.count } : {}),
        ...(contains.length > 0 ? { contains } : {}),
    };
}

/**
 * The card for an `agentmux_memory_injected` frame, or `null` when `rawEvent`
 * isn't one. `contextWindow` bands Personal Memory's size; replay passes the
 * fallback window. `now` stands in when the frame carries no parseable
 * `timestamp`.
 */
export function buildMemoryInjectedNode(
    rawEvent: unknown,
    opts: { contextWindow?: number; now: number },
): ContextDeliveryNode | null {
    if (!isMemoryInjectedFrame(rawEvent)) return null;
    const e = rawEvent as RawFrame;

    const items = (Array.isArray(e.entries) ? e.entries : [])
        .filter((x): x is RawFrame => !!x && typeof x === "object")
        .map(toItem);

    const timestamp = str(e.timestamp) ?? null;
    const parsed = timestamp != null ? Date.parse(timestamp) : NaN;
    // The whole of Personal Memory, not just what fit: a cut delivery is
    // exactly when the size advice matters (ReAgent P1 on #4041).
    const personalTokens = items
        .filter((i) => i.kind === "personal_memory")
        .reduce((sum, i) => sum + (i.sourceTokens ?? i.tokens), 0);

    return {
        type: "context_delivery",
        id: str(e.id) ?? `memory-injected-${timestamp ?? "notime"}`,
        reason: REASON[str(e.reason) ?? ""] ?? "startup",
        items,
        timestamp: Number.isNaN(parsed) ? opts.now : parsed,
        sizeBand: memorySizeBand(personalTokens, opts.contextWindow ?? FALLBACK_CONTEXT_WINDOW),
    };
}
