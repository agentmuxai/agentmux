// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The exit code of a Bash tool result, for both BashOutputViewer and the
 * row's exit pill.
 *
 * `agentmux-bashwrap` prepends every captured bash run with a single line like
 * `<exited 0 in 0.60s>` (or `<exited 1 in 1.23s>` on failure). It's the only
 * durable carrier of the exit code through Claude's tool_use_result, which
 * doesn't include an `exitCode` field.
 *
 * The pill used to carry its own `/<exited\s+(\d+)>/`, which never matched the
 * real `<exited N in Ts>` form, so a Claude Bash row never showed its exit
 * code (found by the descriptor parity test).
 */

const EXIT_PREFIX_RE = /^<exited (-?\d+) in [\d.]+s>\n?/;

/** Split the bashwrap exit prefix off captured stdout. */
export function parseExitPrefix(s: string | undefined): { exit: number | undefined; body: string } {
    if (!s) return { exit: undefined, body: "" };
    const m = s.match(EXIT_PREFIX_RE);
    if (!m) return { exit: undefined, body: s };
    return { exit: parseInt(m[1], 10), body: s.slice(m[0].length) };
}

/** A native `exitCode`, else the prefix on stdout (or the loose `{content}`
 *  shape the translator falls back to). */
export function bashExitCode(result: unknown): number | undefined {
    const r = (result ?? {}) as Record<string, unknown>;
    if (typeof r.exitCode === "number") return r.exitCode;
    const out = typeof r.stdout === "string" ? r.stdout : typeof r.content === "string" ? r.content : undefined;
    return parseExitPrefix(out).exit;
}
