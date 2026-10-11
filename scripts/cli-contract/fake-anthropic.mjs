// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A stand-in for the Anthropic Messages API, for driving the real Claude Code
// CLI in a test without a model or a credential (the CLI is pointed here with
// ANTHROPIC_BASE_URL and a dummy key). The first request that offers the Bash
// tool is answered with one scripted Bash call; every other request with a
// short text reply that ends the turn. What's under test is what the CLI does
// with AgentMux's hook, not what a model says.
//
// docs/retro/RETRO_BASHWRAP_HOOK_DROPS_BASH_TOOL_FIELDS_2026_10_10.md §6.3.

import http from "node:http";

/**
 * Start the server. `toolInputs` is the Bash call to script, or a list of
 * them, made one per request in order (ids `toolu_contract_1`, `_2`, ...).
 * Resolves to `{ url, requests, close }`; `requests` records each request's
 * path and kind, for a failure message.
 */
export function startFakeAnthropic(toolInputs) {
  const calls = Array.isArray(toolInputs) ? toolInputs : [toolInputs];
  const requests = [];
  let callsSent = 0;
  const server = http.createServer((req, res) => {
    let body = "";
    req.on("data", (chunk) => (body += chunk));
    req.on("end", () => {
      const path = (req.url ?? "").split("?")[0];
      let parsed = {};
      try {
        parsed = body ? JSON.parse(body) : {};
      } catch {
        // Not JSON: answered as an empty request.
      }
      if (req.method !== "POST" || !path.startsWith("/v1/messages")) {
        requests.push({ path, kind: "other" });
        res.writeHead(404, { "content-type": "application/json" });
        res.end(JSON.stringify({ type: "error", error: { type: "not_found_error", message: "not faked" } }));
        return;
      }
      if (path.endsWith("/count_tokens")) {
        requests.push({ path, kind: "count_tokens" });
        res.writeHead(200, { "content-type": "application/json" });
        res.end(JSON.stringify({ input_tokens: 1 }));
        return;
      }
      const offersBash = Array.isArray(parsed.tools) && parsed.tools.some((t) => t?.name === "Bash");
      const content =
        offersBash && callsSent < calls.length
          ? [{ type: "tool_use", id: `toolu_contract_${callsSent + 1}`, name: "Bash", input: calls[callsSent] }]
          : [{ type: "text", text: "OK" }];
      if (content[0].type === "tool_use") callsSent++;
      requests.push({ path, kind: content[0].type, stream: parsed.stream === true });
      const message = {
        id: `msg_contract_${requests.length}`,
        type: "message",
        role: "assistant",
        model: parsed.model ?? "claude-contract-test",
        content,
        stop_reason: content[0].type === "tool_use" ? "tool_use" : "end_turn",
        stop_sequence: null,
        usage: { input_tokens: 1, output_tokens: 1 },
      };
      if (parsed.stream === true) streamMessage(res, message);
      else {
        res.writeHead(200, { "content-type": "application/json" });
        res.end(JSON.stringify(message));
      }
    });
  });
  return new Promise((resolve) => {
    server.listen(0, "127.0.0.1", () => {
      const { port } = server.address();
      resolve({ url: `http://127.0.0.1:${port}`, requests, close: () => new Promise((r) => server.close(r)) });
    });
  });
}

/** The message as the API's server-sent events. */
function streamMessage(res, message) {
  res.writeHead(200, { "content-type": "text/event-stream", "cache-control": "no-cache" });
  const send = (type, data) => res.write(`event: ${type}\ndata: ${JSON.stringify({ type, ...data })}\n\n`);
  send("message_start", { message: { ...message, content: [], stop_reason: null } });
  message.content.forEach((block, index) => {
    if (block.type === "tool_use") {
      send("content_block_start", { index, content_block: { ...block, input: {} } });
      send("content_block_delta", {
        index,
        delta: { type: "input_json_delta", partial_json: JSON.stringify(block.input) },
      });
    } else {
      send("content_block_start", { index, content_block: { type: "text", text: "" } });
      send("content_block_delta", { index, delta: { type: "text_delta", text: block.text } });
    }
    send("content_block_stop", { index });
  });
  send("message_delta", {
    delta: { stop_reason: message.stop_reason, stop_sequence: null },
    usage: { output_tokens: 1 },
  });
  send("message_stop", {});
  res.end();
}
