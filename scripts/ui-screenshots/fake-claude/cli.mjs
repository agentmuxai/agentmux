#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A stand-in for the Claude Code CLI that plays a scripted conversation, for
// screenshots of the agent pane — see
// docs/specs/SPEC_UI_MANUAL_SCREENSHOT_TOOLING_2026_09_19.md §10. Installed into
// a capture instance's own CLI folder by install-fake-claude.mjs, it is what
// srv starts for a Claude agent there, so the pane, srv's turn state, session
// stats and token usage all come from a real session: only the model's replies
// are scripted. It never touches the network or any file.
//
//   claude --version                 → the pinned version
//   claude auth status --json        → signed in as the demo account
//   claude --input-format stream-json --output-format stream-json ...
//                                    → reads user messages on stdin, writes
//                                      stream-json on stdout
//
// The script is `conversation.json` next to this file (or $FAKE_CLAUDE_SCRIPT):
//   { "model", "account", "turns": [ { "match"?, "steps": [...], "usage"? } ] }
// A user message plays the first unplayed turn whose `match` it contains, else
// the next unplayed turn. Steps:
//   { "thinking": "..." }                      streamed thinking
//   { "text": "..." }                          streamed text
//   { "tool": "Read", "input": {...}, "result": "..." | {...}, "extra": {...} }
//        a tool call and its result; `extra` is the structured
//        `tool_use_result` (a Read's line range, an Edit's patch)
//   { "ask": {...AskUserQuestion input} }      asks, and waits for the answer
//   { "text": "...", "holdMs": ms }            streamed text that pauses before
//                                              it ends: a turn caught mid-reply
//   { "wait": ms }                             pauses between steps
// An interrupt (the pane's Stop) ends the turn being played.

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";

const VERSION = "2.1.288";
const here = dirname(fileURLToPath(import.meta.url));
const args = process.argv.slice(2);

function loadScript() {
    const path = process.env.FAKE_CLAUDE_SCRIPT || join(here, "conversation.json");
    try {
        return JSON.parse(readFileSync(path, "utf8"));
    } catch {
        return { turns: [] };
    }
}

if (args.includes("--version") || args.includes("-v")) {
    process.stdout.write(`${VERSION} (Claude Code)\n`);
    process.exit(0);
}
if (args[0] === "auth" && args[1] === "status") {
    const s = loadScript();
    process.stdout.write(
        JSON.stringify({ loggedIn: true, authMethod: "claude.ai", emailAddress: s.account ?? "dev@acme.example" }) + "\n"
    );
    process.exit(0);
}

const script = loadScript();
const model = script.model ?? "claude-sonnet-5-5";
const speed = script.msPerChunk ?? 35;
const sessionId = randomId(8) + "-" + randomId(4) + "-" + randomId(4) + "-" + randomId(4) + "-" + randomId(12);
const played = new Set();
let current = null; // { cancelled, wake }
let pendingAsk = null; // { requestId, resolve }

function randomId(n) {
    let s = "";
    while (s.length < n) s += Math.random().toString(16).slice(2);
    return s.slice(0, n);
}

function emit(obj) {
    process.stdout.write(JSON.stringify(obj) + "\n");
}

function sleep(ms, turn) {
    return new Promise((resolve) => {
        const t = setTimeout(resolve, ms);
        if (turn) {
            turn.wake = () => {
                clearTimeout(t);
                resolve();
            };
        }
    });
}

/** Text split into small chunks, as a model streams it. */
function chunks(text) {
    return text.match(/\S+\s*|\s+/g)?.reduce((acc, w, i) => {
        if (i % 3 === 0) acc.push(w);
        else acc[acc.length - 1] += w;
        return acc;
    }, []) ?? [text];
}

/** Streams one content block as its own assistant message, then the message.
 *  `holdMs` pauses before the block ends, so the reply is still streaming. */
async function streamBlock(turn, block, holdMs = 0) {
    const id = "msg_" + randomId(24);
    emit({ type: "stream_event", event: { type: "message_start", message: { id, type: "message", role: "assistant", model, content: [], usage: { input_tokens: 0, output_tokens: 0 } } }, session_id: sessionId });
    if (block.type === "tool_use") {
        emit({ type: "stream_event", event: { type: "content_block_start", index: 0, content_block: { type: "tool_use", id: block.id, name: block.name, input: {} } }, session_id: sessionId });
        emit({ type: "stream_event", event: { type: "content_block_delta", index: 0, delta: { type: "input_json_delta", partial_json: JSON.stringify(block.input) } }, session_id: sessionId });
    } else {
        const key = block.type === "thinking" ? "thinking" : "text";
        const deltaType = block.type === "thinking" ? "thinking_delta" : "text_delta";
        emit({ type: "stream_event", event: { type: "content_block_start", index: 0, content_block: { type: block.type, [key]: "" } }, session_id: sessionId });
        for (const c of chunks(block[key])) {
            if (turn.cancelled) return false;
            emit({ type: "stream_event", event: { type: "content_block_delta", index: 0, delta: { type: deltaType, [key]: c } }, session_id: sessionId });
            await sleep(speed, turn);
        }
        if (holdMs) await sleep(holdMs, turn);
        if (turn.cancelled) return false;
    }
    emit({ type: "stream_event", event: { type: "content_block_stop", index: 0 }, session_id: sessionId });
    emit({ type: "assistant", message: { id, type: "message", role: "assistant", model, content: [block], stop_reason: null, usage: { input_tokens: 0, output_tokens: 0 } }, parent_tool_use_id: null, session_id: sessionId });
    emit({ type: "stream_event", event: { type: "message_delta", delta: { stop_reason: block.type === "tool_use" ? "tool_use" : "end_turn" } }, session_id: sessionId });
    emit({ type: "stream_event", event: { type: "message_stop" }, session_id: sessionId });
    return !turn.cancelled;
}

function toolResult(id, result, extra) {
    const content = typeof result === "string" ? result : JSON.stringify(result ?? "");
    emit({
        type: "user",
        message: { role: "user", content: [{ type: "tool_result", tool_use_id: id, content, is_error: false }] },
        parent_tool_use_id: null,
        session_id: sessionId,
        ...(extra ? { tool_use_result: extra } : {}),
    });
}

async function playTurn(turnDef) {
    const turn = { cancelled: false, wake: null };
    current = turn;
    const started = Date.now();
    emit({ type: "system", subtype: "init", session_id: sessionId, model, cwd: process.cwd(), tools: ["Read", "Edit", "Write", "Bash", "Grep", "Glob", "TodoWrite", "Task", "AskUserQuestion"], claude_code_version: VERSION, permissionMode: "default" });
    for (const step of turnDef ? turnDef.steps : [{ text: "(This demo has no scripted reply for that.)" }]) {
        if (turn.cancelled) break;
        if (step.wait) {
            await sleep(step.wait, turn);
        } else if (step.thinking) {
            await streamBlock(turn, { type: "thinking", thinking: step.thinking, signature: "" });
        } else if (step.text) {
            await streamBlock(turn, { type: "text", text: step.text }, step.holdMs);
        } else if (step.tool) {
            const id = "toolu_" + randomId(24);
            if (!(await streamBlock(turn, { type: "tool_use", id, name: step.tool, input: step.input ?? {} }))) break;
            await sleep(step.runMs ?? 600, turn);
            if (turn.cancelled) break;
            toolResult(id, step.result, step.extra);
        } else if (step.ask) {
            const id = "toolu_" + randomId(24);
            if (!(await streamBlock(turn, { type: "tool_use", id, name: "AskUserQuestion", input: step.ask }))) break;
            const requestId = randomId(16);
            const answer = new Promise((resolve) => (pendingAsk = { requestId, resolve }));
            emit({ type: "control_request", request_id: requestId, request: { subtype: "can_use_tool", tool_name: "AskUserQuestion", tool_use_id: id, input: step.ask } });
            const cancelled = new Promise((resolve) => (turn.wake = resolve));
            const response = await Promise.race([answer, cancelled]);
            pendingAsk = null;
            if (turn.cancelled) break;
            toolResult(id, `User answered: ${JSON.stringify(response?.updatedInput?.answers ?? response ?? {})}`);
        }
    }
    const usage = turnDef?.usage ?? { input_tokens: 1200, output_tokens: 480, cache_creation_input_tokens: 0, cache_read_input_tokens: 18000 };
    emit({
        type: "result",
        subtype: turn.cancelled ? "error_during_execution" : "success",
        is_error: false,
        num_turns: (turnDef?.steps ?? []).length,
        duration_ms: Date.now() - started,
        duration_api_ms: Date.now() - started,
        total_cost_usd: turnDef?.costUsd ?? 0.0421,
        usage,
        session_id: sessionId,
    });
    current = null;
}

/** AgentMux's own context message at launch, which the pane doesn't show. */
const SESSION_CONTEXT = /^\s*# Session Context/;

function pickTurn(text) {
    // Answered with an empty turn, so no scripted turn is used up on it.
    if (SESSION_CONTEXT.test(text)) return { steps: [] };
    const turns = script.turns ?? [];
    let i = turns.findIndex((t, k) => !played.has(k) && t.match && text.toLowerCase().includes(t.match.toLowerCase()));
    if (i < 0) i = turns.findIndex((t, k) => !played.has(k) && !t.match);
    if (i < 0) return null;
    played.add(i);
    return turns[i];
}

function userText(message) {
    const c = message?.content;
    if (typeof c === "string") return c;
    if (Array.isArray(c)) return c.map((b) => (b?.type === "text" ? b.text : "")).join(" ");
    return "";
}

const queue = [];
let busy = false;
async function drain() {
    if (busy) return;
    busy = true;
    while (queue.length) await playTurn(pickTurn(queue.shift()));
    busy = false;
}

const rl = createInterface({ input: process.stdin });
rl.on("line", (line) => {
    let msg;
    try {
        msg = JSON.parse(line);
    } catch {
        return;
    }
    if (msg.type === "user") {
        queue.push(userText(msg.message));
        drain();
    } else if (msg.type === "control_request") {
        // Only an interrupt is answered; srv's other requests (settings,
        // context usage) are optional and go unanswered.
        if (msg.request?.subtype === "interrupt") {
            if (current) {
                current.cancelled = true;
                current.wake?.();
            }
            emit({ type: "control_response", response: { subtype: "success", request_id: msg.request_id, response: {} } });
        }
    } else if (msg.type === "control_response") {
        if (pendingAsk && msg.response?.request_id === pendingAsk.requestId) pendingAsk.resolve(msg.response.response);
    }
});
rl.on("close", () => process.exit(0));
