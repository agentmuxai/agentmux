// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The context meter's reading for an ACP agent that reports its context fill:
 * `usage_update`'s `used` tokens of a `size`-token window, on the model the
 * session's `configOptions` name. Claude Code's per-call usage
 * (main-agent-usage.ts) is the other source; a pane reads one or the other.
 *
 * Read only for a format measured to send it: OpenCode 1.19.0 sends one
 * `usage_update` per turn, with `used` the turn's last call's whole prompt
 * (uncached + cached), and names its model in `session/new`'s `configOptions`
 * (`{id: "model", currentValue: "opencode/big-pickle"}`).
 */

/** The context fill one `usage_update` reported. */
export interface AcpContext {
    tokens: number;
    /** The window it was measured against; null when the agent sent none. */
    window: number | null;
    /** The session's model, when the agent has named it. */
    model: string | undefined;
}

export function readsAcpContext(outputFormat: string): boolean {
    return outputFormat === "opencode-acp";
}

/** Fed every raw line in order; returns the reading a `usage_update` carries. */
export interface AcpContextTracker {
    observe(rawEvent: unknown): AcpContext | null;
}

/** A window outside this range is not a context window (context-reading.ts). */
const MIN_WINDOW = 1_000;
const MAX_WINDOW = 100_000_000;

export function createAcpContextTracker(): AcpContextTracker {
    let model: string | undefined;
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
            const used = update.used;
            if (typeof used !== "number" || !Number.isFinite(used) || used <= 0) return null;
            const size = update.size;
            const window = typeof size === "number" && Number.isInteger(size) && size >= MIN_WINDOW && size <= MAX_WINDOW ? size : null;
            return { tokens: Math.round(used), window, model };
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
