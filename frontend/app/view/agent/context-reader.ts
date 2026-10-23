// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The context meter's reading for a provider that isn't Claude Code: how full
 * its context is, from what the provider itself reports. Claude Code's
 * per-call usage (main-agent-usage.ts) is the other source; a pane reads one or
 * the other. Live (useAgentStream.ts) and history (parseHistoryLines.ts) feed
 * the same reader every raw line in order.
 *
 * Two sources, best first; counting tokens ourselves would be the last resort,
 * and isn't done:
 *
 *   - `agentmux_context`, for any provider: a frame srv writes into the stream
 *     from the provider's own record of its last model call. Codex: srv reads
 *     the thread's session file after each turn (subprocess/codex_context.rs),
 *     since `codex exec --json` reports only the thread's running totals.
 *     `{"type":"agentmux_context","tokens":94399,"window":258400,"model":"gpt-6-astra"}`
 *   - an ACP agent's `usage_update`, for a format measured to send it:
 *     OpenCode 1.19.0 sends one per turn, with `used` the turn's last call's
 *     whole prompt (uncached + cached), and names its model in `session/new`'s
 *     `configOptions` (`{id: "model", currentValue: "opencode/big-pickle"}`).
 *
 * Never a turn's total: Codex's `turn.completed` usage sums the whole thread
 * (three one-word turns read 14,295, 28,608, 42,939), and a difference of two
 * sums every model call of the turn, so an agentic turn reads many times its
 * context (store/agent-pane-state/context-reading.ts).
 */

/** The context fill a provider reported. */
export interface ReportedContext {
    tokens: number;
    /** The window it was measured against; null when the provider sent none. */
    window: number | null;
    /** The model it was measured on; null when the report names none (the
     *  pane's last model stands). For ACP, the agent's name until it names one,
     *  so the window is kept for a pane whose restored tail starts after
     *  `session/new`. */
    model: string | null;
}

/** Fed every raw line in order; returns the reading a line carries. */
export interface ContextReader {
    observe(rawEvent: unknown): ReportedContext | null;
}

/** The ACP agent each format that sends `usage_update` belongs to. */
const ACP_CONTEXT_AGENTS: Record<string, string> = { "opencode-acp": "OpenCode" };

/** A reader for `outputFormat`'s lines. Every format reads `agentmux_context`. */
export function contextReaderFor(outputFormat: string): ContextReader {
    const agent = ACP_CONTEXT_AGENTS[outputFormat];
    const acp = agent ? createAcpContextReader(agent) : null;
    return {
        observe(rawEvent) {
            return agentmuxContext(rawEvent) ?? acp?.observe(rawEvent) ?? null;
        },
    };
}

/** A window outside this range is not a context window (context-reading.ts). */
const MIN_WINDOW = 1_000;
const MAX_WINDOW = 100_000_000;

function windowOrNull(size: unknown): number | null {
    return typeof size === "number" && Number.isInteger(size) && size >= MIN_WINDOW && size <= MAX_WINDOW ? size : null;
}

function tokensOrNull(n: unknown): number | null {
    return typeof n === "number" && Number.isFinite(n) && n > 0 ? Math.round(n) : null;
}

/** The reading an `agentmux_context` frame carries. */
export function agentmuxContext(rawEvent: unknown): ReportedContext | null {
    if (!rawEvent || typeof rawEvent !== "object") return null;
    const ev = rawEvent as { type?: unknown; tokens?: unknown; window?: unknown; model?: unknown };
    if (ev.type !== "agentmux_context") return null;
    const tokens = tokensOrNull(ev.tokens);
    if (tokens == null) return null;
    return { tokens, window: windowOrNull(ev.window), model: stringOrUndefined(ev.model) ?? null };
}

export function createAcpContextReader(agent: string): ContextReader {
    let model: string = agent;
    return {
        observe(rawEvent) {
            if (!rawEvent || typeof rawEvent !== "object") return null;
            const ev = rawEvent as { result?: any; params?: any };
            // `session/new` / `session/load` answer with the options in force;
            // `config_option_update` reports a change.
            const fromResult = modelOption(ev.result?.configOptions) ?? stringOrUndefined(ev.result?.models?.currentModelId);
            if (fromResult) model = fromResult;
            const update = ev.params?.update;
            if (!update || typeof update !== "object") return null;
            if (update.sessionUpdate === "config_option_update") {
                model = modelOption(update.configOptions) ?? model;
                return null;
            }
            if (update.sessionUpdate !== "usage_update") return null;
            const tokens = tokensOrNull(update.used);
            if (tokens == null) return null;
            return { tokens, window: windowOrNull(update.size), model };
        },
    };
}

/** The `model` option's current value in an ACP `configOptions` list. */
function modelOption(options: unknown): string | undefined {
    if (!Array.isArray(options)) return undefined;
    const option = options.find((o) => o && typeof o === "object" && (o.id === "model" || o.category === "model"));
    return stringOrUndefined(option?.currentValue);
}

function stringOrUndefined(v: unknown): string | undefined {
    return typeof v === "string" && v ? v : undefined;
}
