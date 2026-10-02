// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A fake Anthropic API for probing the Claude CLI without an account.
 *
 * Point the CLI at it (`ANTHROPIC_BASE_URL`) and it records every request and
 * answers `POST /v1/messages` with a canned reply, streaming or not. The
 * `model` it reports back is whatever the CLI SENT, so the recorded requests
 * are the ground truth for "which model did the CLI resolve this to" - with no
 * credentials, no cost, and no real network.
 *
 * Part of the verification harness in docs/specs/SPEC_RUNTIME_MENU_REMAINING_GAPS_2026_10_01.md §7.
 */

import http from "node:http";

/** One SSE frame. */
function frame(event) {
    return `event: ${event.type}\ndata: ${JSON.stringify(event)}\n\n`;
}

/** The streaming events of a one-block text reply, as the real API sends them. */
export function streamEvents(model, text) {
    const message = {
        id: "msg_probe_0001",
        type: "message",
        role: "assistant",
        model,
        content: [],
        stop_reason: null,
        stop_sequence: null,
        usage: { input_tokens: 10, output_tokens: 1, cache_creation_input_tokens: 0, cache_read_input_tokens: 0 },
    };
    return [
        { type: "message_start", message },
        { type: "content_block_start", index: 0, content_block: { type: "text", text: "" } },
        { type: "content_block_delta", index: 0, delta: { type: "text_delta", text } },
        { type: "content_block_stop", index: 0 },
        { type: "message_delta", delta: { stop_reason: "end_turn", stop_sequence: null }, usage: { output_tokens: 3 } },
        { type: "message_stop" },
    ];
}

/** The streaming events of a reply that calls one tool, as the real API sends them. */
export function toolUseStreamEvents(model, tool) {
    const message = {
        id: "msg_probe_tool",
        type: "message",
        role: "assistant",
        model,
        content: [],
        stop_reason: null,
        stop_sequence: null,
        usage: { input_tokens: 10, output_tokens: 1, cache_creation_input_tokens: 0, cache_read_input_tokens: 0 },
    };
    return [
        { type: "message_start", message },
        { type: "content_block_start", index: 0, content_block: { type: "tool_use", id: tool.id, name: tool.name, input: {} } },
        { type: "content_block_delta", index: 0, delta: { type: "input_json_delta", partial_json: JSON.stringify(tool.input) } },
        { type: "content_block_stop", index: 0 },
        { type: "message_delta", delta: { stop_reason: "tool_use", stop_sequence: null }, usage: { output_tokens: 20 } },
        { type: "message_stop" },
    ];
}

/** The non-streaming form of the same reply. */
export function messageBody(model, text) {
    return {
        id: "msg_probe_0001",
        type: "message",
        role: "assistant",
        model,
        content: [{ type: "text", text }],
        stop_reason: "end_turn",
        stop_sequence: null,
        usage: { input_tokens: 10, output_tokens: 3, cache_creation_input_tokens: 0, cache_read_input_tokens: 0 },
    };
}

/**
 * Start the server on a free local port.
 * @param {{ reply?: string, script?: (n: number, body: object) => ({ toolUse: { id: string, name: string, input: object } } | { text: string } | undefined) }} [opts]
 *   `script` decides each /v1/messages reply by its 0-based index: a tool call
 *   to make the CLI ask for permission, or text. Default: always `reply`.
 * @returns {Promise<{ url: string, requests: Array<object>, close: () => Promise<void> }>}
 */
export async function startFakeAnthropic({ reply = "ok", script } = {}) {
    const requests = [];
    let messageCount = 0;
    const server = http.createServer((req, res) => {
        const chunks = [];
        req.on("data", (c) => chunks.push(c));
        req.on("end", () => {
            const raw = Buffer.concat(chunks).toString("utf8");
            let body = null;
            try {
                body = raw ? JSON.parse(raw) : null;
            } catch {
                /* not JSON: recorded as null */
            }
            const path = (req.url ?? "").split("?")[0];
            requests.push({
                method: req.method,
                path,
                model: body?.model,
                stream: body?.stream === true,
                // Which request fields the CLI sent, so a test can see e.g. whether an
                // effort setting was passed on, without storing a whole prompt.
                bodyKeys: body && typeof body === "object" ? Object.keys(body).sort() : [],
                body,
                hasApiKey: Boolean(req.headers["x-api-key"]),
                userAgent: req.headers["user-agent"],
            });

            if (req.method === "POST" && path === "/v1/messages") {
                const model = typeof body?.model === "string" ? body.model : "unknown-model";
                const planned = script?.(messageCount++, body);
                if (body?.stream) {
                    res.writeHead(200, { "content-type": "text/event-stream", "cache-control": "no-cache" });
                    const events = planned?.toolUse
                        ? toolUseStreamEvents(model, planned.toolUse)
                        : streamEvents(model, planned?.text ?? reply);
                    for (const e of events) res.write(frame(e));
                    res.end();
                } else {
                    res.writeHead(200, { "content-type": "application/json" });
                    res.end(JSON.stringify(messageBody(model, reply)));
                }
                return;
            }
            if (req.method === "POST" && path === "/v1/messages/count_tokens") {
                res.writeHead(200, { "content-type": "application/json" });
                res.end(JSON.stringify({ input_tokens: 10 }));
                return;
            }
            res.writeHead(404, { "content-type": "application/json" });
            res.end(JSON.stringify({ type: "error", error: { type: "not_found_error", message: `no such route: ${path}` } }));
        });
    });
    await new Promise((resolve, reject) => {
        server.once("error", reject);
        server.listen(0, "127.0.0.1", resolve);
    });
    const { port } = server.address();
    return {
        url: `http://127.0.0.1:${port}`,
        requests,
        close: () =>
            new Promise((resolve) => {
                server.closeAllConnections?.();
                server.close(() => resolve());
            }),
    };
}
