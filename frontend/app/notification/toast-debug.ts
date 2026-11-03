// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The text a toast's Copy button puts on the clipboard: the whole title and message, then what
// someone needs to report it (when, which build, which window, tab, pane and agent, and whatever
// detail the caller passed in `debug`). Plain text, one fact per line.

/** What the text says about this AgentMux and this window; `liveToastDebugEnv` reads the real ones. */
export interface ToastDebugEnv {
    version?: string;
    build?: string;
    channel?: string;
    platform?: string;
    windowId?: string;
    tabId?: string;
}

/** A `detail` key that looks like a secret: its value is blanked in the copy. */
const SECRET_KEY = /token|secret|password|passwd|authorization|api[-_]?key|cookie|credential|bearer|private[-_]?key/i;
const REDACTED = "[hidden]";
const MAX_DEPTH = 6;
/** The detail object is cut here, so a toast never copies a page of data. */
const MAX_DETAIL_CHARS = 4000;

function redact(value: unknown, depth = 0): unknown {
    if (value == null || typeof value !== "object") return value;
    // Deeper than we look: not copied at all, rather than copied unchecked.
    if (depth > MAX_DEPTH) return "[too deep]";
    if (value instanceof Error) return { name: value.name, message: value.message, stack: value.stack };
    if (Array.isArray(value)) return value.map((v) => redact(v, depth + 1));
    const out: Record<string, unknown> = {};
    for (const [k, v] of Object.entries(value)) out[k] = SECRET_KEY.test(k) ? REDACTED : redact(v, depth + 1);
    return out;
}

function detailText(detail: Record<string, unknown>): string {
    let json: string;
    try {
        json = JSON.stringify(redact(detail), null, 2) ?? "";
    } catch {
        json = "(not printable)";
    }
    return json.length > MAX_DETAIL_CHARS ? `${json.slice(0, MAX_DETAIL_CHARS)}\n… (cut at ${MAX_DETAIL_CHARS} characters)` : json;
}

const pad = (n: number, width = 2) => String(n).padStart(width, "0");

/** `2026-11-02 08:12:03.045 (UTC+03:00)`, in the viewer's own zone. */
function localTime(d: Date): string {
    const offset = -d.getTimezoneOffset();
    const sign = offset < 0 ? "-" : "+";
    const zone = `UTC${sign}${pad(Math.floor(Math.abs(offset) / 60))}:${pad(Math.abs(offset) % 60)}`;
    const date = `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
    const time = `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}.${pad(d.getMilliseconds(), 3)}`;
    return `${date} ${time} (${zone})`;
}

/** The toast as plain text for a bug report. `env` carries what is not on the toast itself. */
export function toastDebugText(n: NotificationType, env: ToastDebugEnv = {}): string {
    const lines: string[] = [];
    if (n.title) lines.push(`Title: ${n.title}`);
    if (n.message) lines.push(`Message: ${n.message}`);
    lines.push(`Kind: ${n.type ?? "info"}`);

    const raised = n.timestamp ? new Date(n.timestamp) : null;
    if (raised && !Number.isNaN(raised.getTime())) {
        lines.push(`Time: ${localTime(raised)}`, `Time (UTC): ${raised.toISOString()}`);
    }

    const build = env.build && env.build !== env.version ? `, build ${env.build}` : "";
    const channel = env.channel ? `, channel ${env.channel}` : "";
    const platform = env.platform ? `, ${env.platform}` : "";
    if (env.version || env.build || env.channel) lines.push(`AgentMux: ${env.version ?? "unknown"}${build}${channel}${platform}`);

    const d = n.debug ?? {};
    const win = d.windowId ?? env.windowId;
    const tab = d.tabId ?? env.tabId;
    if (win) lines.push(`Window: ${win}`);
    if (tab) lines.push(`Tab: ${tab}`);
    if (d.blockId) lines.push(`Pane: ${d.blockId}`);
    if (d.agentName || d.agentId) {
        lines.push(`Agent: ${d.agentName && d.agentId ? `${d.agentName} (${d.agentId})` : (d.agentName ?? d.agentId)}`);
    }
    if (d.code) lines.push(`Code: ${d.code}`);
    if (d.requestId) lines.push(`Request id: ${d.requestId}`);
    if (d.detail && Object.keys(d.detail).length > 0) lines.push(`Detail: ${detailText(d.detail)}`);
    return lines.join("\n");
}

/** A caught error as `debug.detail`: its name, message and stack. */
export function errorDetail(e: unknown): Record<string, unknown> {
    if (e instanceof Error) return { error: e.name, message: e.message, stack: e.stack };
    return { error: String(e) };
}
