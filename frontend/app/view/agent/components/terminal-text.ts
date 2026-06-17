// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * terminalText — extract a terminal-style text body from a tool result, using
 * the same field precedence `CompactResult.summarize()` uses: a raw string
 * result, else `stdout`(+`stderr`), else `output`, else `content`. Returns
 * `null` for a purely structured result (no string body) or an empty body, so
 * callers fall back to the JSON view.
 *
 * Centralizing the precedence lets the feed render task/tool output as a
 * terminal instead of a JSON blob.
 * See docs/specs/SPEC_TOOL_OUTPUT_TEE_AND_TERMINAL_RENDER_2026_06_17.md §4.2.
 */
export function terminalText(result: unknown): string | null {
    const raw = extract(result);
    return raw != null && raw.length > 0 ? raw : null;
}

function extract(result: unknown): string | null {
    if (result == null) return null;
    if (typeof result === "string") return result;
    if (typeof result !== "object") return null;
    const r = result as Record<string, unknown>;

    // Command-shaped results: stdout (+ stderr) is the terminal body.
    const stdout = typeof r.stdout === "string" ? r.stdout : null;
    const stderr = typeof r.stderr === "string" ? r.stderr : null;
    if (stdout != null || stderr != null) {
        return [stdout ?? "", stderr ?? ""].filter((s) => s.length > 0).join("\n");
    }

    // Generic string carriers, same precedence as summarize().
    if (typeof r.output === "string") return r.output;
    if (typeof r.content === "string") return r.content;
    return null;
}
