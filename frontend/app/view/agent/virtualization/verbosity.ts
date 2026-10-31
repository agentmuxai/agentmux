// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What the agent pane shows (docs/specs/SPEC_AGENT_PANE_VERBOSITY_MODES_2026_10_29.md): five
 * switches, one per kind of routine row, and three presets (Full, Standard, Replies) that set
 * them. A view-layer filter only: the rows, history, search and copy stay complete, and
 * `deriveDisplayNodes` turns the same nodes into what the list shows.
 *
 * A switch can only hide routine work that completed, or plumbing. `ruleOf` is where each row kind
 * says which switch governs it and when nothing may hide it (the never-hide list, spec 4.4).
 */

import type { DocumentNode, HiddenKind, HiddenRunNode, ViewMode } from "../types";

/** How a switch treats its rows. `auto` is the row's own rule today (disclosure.ts). */
export type Fold = "auto" | "expanded" | "collapsed" | "hidden";

export interface AgentView {
    thinking: Fold;
    /** Commands and their previews, reads, edits, searches, MCP calls, and shells. */
    tools: Fold;
    /** Jekts and agent messages. */
    jekts: Fold;
    notes: "shown" | "hidden";
    /** Startup context, compaction progress, memory reinjected, context delivered, informational CLI notices. */
    context: "auto" | "hidden";
}

export type SwitchKey = keyof AgentView;
export const SWITCH_KEYS: readonly SwitchKey[] = ["thinking", "tools", "jekts", "notes", "context"];

export type PresetName = "full" | "standard" | "replies";
export const PRESET_NAMES: readonly PresetName[] = ["full", "standard", "replies"];

export const VIEW_PRESETS: Record<PresetName, AgentView> = {
    full: { thinking: "auto", tools: "auto", jekts: "auto", notes: "shown", context: "auto" },
    standard: { thinking: "collapsed", tools: "collapsed", jekts: "collapsed", notes: "shown", context: "hidden" },
    replies: { thinking: "hidden", tools: "hidden", jekts: "hidden", notes: "hidden", context: "hidden" },
};

export const PRESET_LABEL: Record<PresetName | "custom", string> = {
    full: "Full",
    standard: "Standard",
    replies: "Replies",
    custom: "Custom",
};

export const SWITCH_LABEL: Record<SwitchKey, string> = {
    thinking: "Thinking",
    tools: "Tool calls",
    jekts: "Jekts and agent messages",
    notes: "AgentMux notes",
    context: "Context and setup",
};

/** The values a switch takes, in the order the control shows them. */
export const SWITCH_VALUES: { [K in SwitchKey]: readonly AgentView[K][] } = {
    thinking: ["auto", "expanded", "collapsed", "hidden"],
    tools: ["auto", "expanded", "collapsed", "hidden"],
    jekts: ["auto", "expanded", "collapsed", "hidden"],
    notes: ["shown", "hidden"],
    context: ["auto", "hidden"],
};

/** The preset whose five values these are, or `custom`. Derived, so there is nothing to keep in sync. */
export function presetOf(view: AgentView): PresetName | "custom" {
    for (const name of PRESET_NAMES) {
        const p = VIEW_PRESETS[name];
        if (SWITCH_KEYS.every((k) => p[k] === view[k])) return name;
    }
    return "custom";
}

export function isFull(view: AgentView): boolean {
    return presetOf(view) === "full";
}

/**
 * A view from a stored value: a preset name (`"standard"`), or an object with some of the five
 * switches (a missing or unknown one takes Full's value). Anything else is `undefined`.
 */
export function parseView(raw: unknown): AgentView | undefined {
    if (typeof raw === "string") return (VIEW_PRESETS as Record<string, AgentView>)[raw.toLowerCase()];
    if (raw == null || typeof raw !== "object" || Array.isArray(raw)) return undefined;
    const rec = raw as Record<string, unknown>;
    const out: Record<string, unknown> = {};
    for (const key of SWITCH_KEYS) {
        const allowed = SWITCH_VALUES[key] as readonly unknown[];
        out[key] = allowed.includes(rec[key]) ? rec[key] : VIEW_PRESETS.full[key];
    }
    return out as unknown as AgentView;
}

export interface ResolvedView {
    view: AgentView;
    /** `pane`: the pane has its own complete view; `default`: it follows the setting. */
    source: "pane" | "default";
}

/** A pane has its own complete view or follows the default: no per-key merging (spec 4.5). */
export function resolveView(globalRaw: unknown, paneRaw: unknown): ResolvedView {
    const own = parseView(paneRaw);
    if (own) return { view: own, source: "pane" };
    return { view: parseView(globalRaw) ?? VIEW_PRESETS.full, source: "default" };
}

/** Full, then Standard, then Replies, then Full; a Custom view goes to Full first. */
export function nextPreset(view: AgentView): PresetName {
    const now = presetOf(view);
    if (now === "custom") return "full";
    return PRESET_NAMES[(PRESET_NAMES.indexOf(now) + 1) % PRESET_NAMES.length];
}

// ── Which switch governs which row ──────────────────────────────────────────

type StoredNode = Exclude<DocumentNode, HiddenRunNode>;

/** `key`: the switch (none: no switch, always shown). `forced`: the never-hide list. */
export interface RowRule {
    key: SwitchKey | null;
    kind?: HiddenKind;
    forced?: boolean;
}

const NO_SWITCH: RowRule = { key: null };

/** A notice that is a problem, not information or success: warnings and errors have no switch. */
function isProblemNotice(node: Extract<StoredNode, { type: "cli_notice" }>): boolean {
    if (node.kind === "install") return node.state === "failed";
    if (node.kind === "compaction_failed") return true;
    if (node.kind === "mcp_health") return node.mcpState === "gave_up";
    if (node.kind === "container_start") return node.startState === "failed";
    if (node.kind === "container_image") return node.imageState === "outdated";
    return false;
}

/** The cloud "expired undelivered" note: something was lost, so it is never hidden. */
function isLossNote(text: string): boolean {
    return /undeliver/i.test(text);
}

/**
 * One entry per row kind, typed so a new `DocumentNode` kind is a compile error until it says
 * which switch governs it (or none).
 */
const RULES: { [T in StoredNode["type"]]: (node: Extract<StoredNode, { type: T }>) => RowRule } = {
    markdown: (n) => (n.metadata?.thinking || n.metadata?.canceled ? { key: "thinking", kind: "thinking" } : NO_SWITCH),
    tool: (n) => ({
        key: "tools",
        kind: "tool",
        // Waiting for the user: outside every fold (Cursor's buried Allow/Deny).
        forced: n.status === "pending_approval" || n.status === "awaiting_answer",
    }),
    shell: () => ({ key: "tools", kind: "tool" }),
    agent_message: () => ({ key: "jekts", kind: "agent_message" }),
    jekt_message: (n) => ({
        key: "jekts",
        kind: "jekt",
        // A sensitive jekt, or one whose sender was not proven, may be the operator's voice arriving by another route.
        forced: n.tier === "sensitive" || n.trust === "unverified",
    }),
    system_note: (n) => ({ key: "notes", kind: "note", forced: isLossNote(n.text) }),
    ambient_narration: () => ({ key: "notes", kind: "note" }),
    user_message: (n) => (n.isStartup ? { key: "context", kind: "setup" } : NO_SWITCH),
    context_delivery: () => ({ key: "context", kind: "setup" }),
    compaction_started: () => ({ key: "context", kind: "setup" }),
    memory_reinjection: () => ({ key: "context", kind: "setup" }),
    cli_notice: (n) => (isProblemNotice(n) ? NO_SWITCH : { key: "context", kind: "setup" }),
    // The conversation and the things that explain it: no switch.
    agent_error: () => NO_SWITCH,
    context_compacted: () => NO_SWITCH,
    session_outcome: () => NO_SWITCH,
    day_divider: () => NO_SWITCH,
    history_link: () => NO_SWITCH,
    resume_preflight: () => NO_SWITCH,
};

export function ruleOf(node: StoredNode): RowRule {
    return (RULES[node.type] as (n: StoredNode) => RowRule)(node);
}

// ── The display list ────────────────────────────────────────────────────────

/** A copy of `node` with a view mode, kept per node so an unchanged row keeps its identity. */
const WITH_MODE = new WeakMap<object, Partial<Record<ViewMode, DocumentNode>>>();

function withMode(node: DocumentNode, mode: ViewMode): DocumentNode {
    if (node.type !== "hidden_run" && node.viewMode === mode) return node;
    let byMode = WITH_MODE.get(node);
    if (!byMode) WITH_MODE.set(node, (byMode = {}));
    return (byMode[mode] ??= { ...node, viewMode: mode } as DocumentNode);
}

export function hiddenRunId(firstHiddenId: string): string {
    return `hidden-run:${firstHiddenId}`;
}

/** The state `deriveDisplayNodes` reads from `documentState`. */
export interface ViewState {
    /** A pinned row stays visible in any view; a pinned run is open. */
    pinnedNodes: ReadonlySet<string>;
    /** A tool held open after it finished on screen stays as a fold until it scrolls off. */
    heldOpenNodes: ReadonlySet<string>;
}

interface PendingRun {
    id: string;
    rows: DocumentNode[];
    counts: Map<HiddenKind, number>;
    failed: number;
}

/**
 * The rows the list shows for `nodes` under `view`. Full returns `nodes` itself. A row a switch
 * hides joins a run with its neighbours, shown as one `hidden_run` line (open runs show their rows
 * again in their full form); Collapsed and Expanded rows are copies carrying a `viewMode`.
 */
export function deriveDisplayNodes(nodes: DocumentNode[], view: AgentView, state: ViewState): DocumentNode[] {
    if (isFull(view)) return nodes;
    const out: DocumentNode[] = [];
    let run: PendingRun | null = null;

    const flush = () => {
        if (!run) return;
        const open = state.pinnedNodes.has(run.id);
        const node: HiddenRunNode = {
            type: "hidden_run",
            id: run.id,
            counts: [...run.counts].map(([kind, count]) => ({ kind, count })),
            failed: run.failed,
            open,
            hiddenIds: run.rows.map((r) => r.id),
        };
        out.push(node);
        if (open) out.push(...run.rows);
        run = null;
    };
    const keep = (node: DocumentNode) => {
        flush();
        out.push(node);
    };

    for (const node of nodes) {
        if (node.type === "hidden_run") {
            keep(node);
            continue;
        }
        const rule = ruleOf(node);
        if (!rule.key || rule.forced || state.pinnedNodes.has(node.id)) {
            keep(node);
            continue;
        }
        const value: string = view[rule.key];
        if (value === "hidden") {
            // A tool that was waiting for the user and has resolved stays as a fold until it scrolls off.
            if (node.type === "tool" && state.heldOpenNodes.has(node.id)) {
                keep(withMode(node, "collapsed"));
                continue;
            }
            const pending: PendingRun = (run ??= { id: hiddenRunId(node.id), rows: [], counts: new Map(), failed: 0 });
            pending.rows.push(node);
            if (rule.kind) pending.counts.set(rule.kind, (pending.counts.get(rule.kind) ?? 0) + 1);
            if (node.type === "tool" && node.status === "failed") pending.failed++;
        } else if (value === "collapsed" || value === "expanded") {
            keep(withMode(node, value));
        } else {
            keep(node);
        }
    }
    flush();
    return out;
}

/** The text of a hidden-run line, e.g. `12 tool calls, 3 thoughts, 1 jekt`. */
export function hiddenRunLabel(counts: HiddenRunNode["counts"]): string {
    const names: Record<HiddenKind, [string, string]> = {
        tool: ["tool call", "tool calls"],
        thinking: ["thought", "thoughts"],
        jekt: ["jekt", "jekts"],
        agent_message: ["agent message", "agent messages"],
        note: ["note", "notes"],
        setup: ["setup row", "setup rows"],
    };
    return counts.map(({ kind, count }) => `${count} ${names[kind][count === 1 ? 0 : 1]}`).join(", ");
}
