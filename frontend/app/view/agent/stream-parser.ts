// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * NDJSON Stream Parser for Claude Code output
 *
 * Parses NDJSON stream from Claude Code (--output-format stream-json)
 * and converts events into DocumentNode objects for rendering.
 */

import {
    AgentErrorNode,
    AgentMessageEvent,
    AGENT_MESSAGE_ICONS,
    DIRECTION_ICONS,
    DocumentNode,
    ErrorResultEvent,
    JektDeliveryTier,
    JektMessageNode,
    JektTier,
    JektTrust,
    STATUS_ICONS,
    StreamEvent,
    TextEvent,
    ThinkingEvent,
    TOOL_ICONS,
    ToolCallEvent,
    ToolChunkEvent,
    ToolLogChunk,
    ToolNode,
    ToolResultEvent,
    UserMessageEvent,
} from "./types";

/**
 * Detects the auto-generated startup payload by its literal first
 * heading. `buildStartupPayload` emits `# Session Context` on the
 * first line — this regex pins that contract. Updating the heading
 * there without updating this regex will break the startup
 * collapse-on-hover render. The L1 test on
 * `ClaudeCodeStreamParser.userMessageToNode` asserts both
 * positive and negative cases.
 *
 * Spec:
 * `docs/specs/SPEC_USER_INPUT_VISIBILITY_AND_STARTUP_COLLAPSE_2026_05_24.md`.
 */
export const STARTUP_HEADING_RE = /^# Session Context\b/;

/**
 * Matches a full `[JEKT:...]...[/JEKT]` marker block spanning the whole
 * message. Produced by `wrap_jekt_message` (`agentmux-srv/src/backend/
 * reactive/sanitize.rs`) and `wrapJektMessage` (`muxbus-cloud/muxbus/server/
 * src/index.ts`) — the two current producers of this format. Group 1 is the
 * structured tag's field string (`FROM=... TO=... TIER=...`); group 2 is
 * everything between the tag line and the closing `[/JEKT]`.
 *
 * Spec: docs/specs/SPEC_JEKT_SECURITY_AND_VISIBILITY_2026_07_01.md §3.1.
 */
const JEKT_BLOCK_RE = /^\[JEKT:([^\]\n]+)\]\r?\n([\s\S]*?)\r?\n\[\/JEKT\]\s*$/;

const VALID_JEKT_TIERS: ReadonlySet<string> = new Set(["info", "coord", "sensitive"]);
const VALID_JEKT_DELIVERY_TIERS: ReadonlySet<string> = new Set(["host", "lan", "wan"]);

/** Parses the `KEY=value` tokens out of a jekt structured-tag string. */
function parseJektTagFields(tag: string): Record<string, string> {
    const fields: Record<string, string> = {};
    const re = /(\w+)=(\S+)/g;
    let m: RegExpExecArray | null;
    while ((m = re.exec(tag))) {
        fields[m[1]] = m[2];
    }
    return fields;
}

/**
 * Strips the presentational scaffolding both jekt-wrapping implementations
 * add around the actual message (divider lines, the "From: X | To: Y | ..."
 * header line, the sensitive-tier warning, the reply hint), leaving just
 * what the sender actually typed/sent. Best-effort: any line that doesn't
 * match a known scaffolding shape is kept, so an unrecognized wrapper
 * variant degrades to showing extra context rather than losing content.
 */
function stripJektEnvelope(body: string, tier: JektTier): string {
    const lines = body.split("\n");
    let start = 0;
    let end = lines.length;

    // Structural positions guaranteed by wrap_jekt_message/wrapJektMessage:
    // [start] divider, [start+1] "From: X | To: Y | ts=Z" header, and — only
    // for TIER=sensitive — a "⚠ ..." warning line plus a blank line right
    // after it. Only strip at these fixed offsets, never mid-body, so a
    // well-formed message that happens to contain a dash line or its own
    // "From:"/"Reply:" text is never mistaken for scaffolding.
    if (/^─+$/.test(lines[start] ?? "")) start++;
    if (/^From:.*\|.*\|/.test(lines[start] ?? "")) start++;
    if (tier === "sensitive" && /^⚠/.test(lines[start] ?? "")) {
        start++;
        if ((lines[start] ?? "") === "") start++;
    }
    if (/^Reply:/.test(lines[end - 1] ?? "")) end--;
    if (/^─+$/.test(lines[end - 1] ?? "")) end--;

    return lines.slice(start, end).join("\n").trim();
}

/**
 * Extract relevant detail from tool params for summary/tooltip display.
 * Returns the full text, untruncated — callers decide how to clip/wrap it
 * (the .agent-tool-name CSS rule clips with `text-overflow: ellipsis` based
 * on actual row width, so the ellipsis position recomputes for free on zoom
 * and pane resize; a hover tooltip instead word-wraps the same full string).
 * Pre-truncating here would freeze either presentation at a fixed character
 * count. (See SPEC_DYNAMIC_TOOL_SUMMARY_TRUNCATION.md.) Exported so both
 * generateToolSummary and a tool-block tooltip share one per-tool-kind
 * switch instead of drifting out of sync.
 */
export function extractToolDetail(tool: string, params: Record<string, any>): string {
    switch (tool) {
        case "Read":
        case "Edit":
        case "Write":
            return params.file_path || "";
        case "Bash":
            return params.command || "";
        case "Grep":
            return params.pattern || "";
        case "Glob":
            return params.pattern || "";
        case "Agent":
            return params.description || params.prompt || "";
        case "Workflow":
            return params.title || params.description || "";
        case "web_search":
        case "WebSearch":
            return params.query || "";
        case "WebFetch":
        case "web_fetch":
            try {
                const u = new URL(params.url || "");
                return u.host + (u.pathname === "/" ? "" : u.pathname);
            } catch {
                return params.url || "";
            }
        default:
            return "";
    }
}

/**
 * Shared default skip-callback — returns the same empty `Set`
 * reference each call so the no-op path stays allocation-free.
 */
const EMPTY_SKIP_SET: ReadonlySet<string> = new Set<string>();
const STATIC_EMPTY_SKIP_SET_FN = (): ReadonlySet<string> => EMPTY_SKIP_SET;

export class ClaudeCodeStreamParser {
    private buffer: string = "";
    private nodeIdCounter: number = 0;
    /**
     * Optional callback returning the set of ids the parser must
     * avoid when generating new ones — typically the document
     * store's current `nodeIdSet`. Called **on-demand at each id
     * generation**, NOT captured at construction time.
     *
     * Why on-demand: resumed-session snapshots are restored
     * **asynchronously** (`useHistoryPagination` dispatches
     * `HistoryLoaded` after an RPC round-trip). Capturing a static
     * set at parser construction leaves the parser with an empty
     * skip-set even though the document IS populated by the time
     * the first live event arrives. Codex P1 #2 on PR #1101
     * caught that race after the static-Set version was pushed.
     *
     * **Counter-based ids stay deterministic across instances** so
     * that `parseHistoryLines` (in `useHistoryPagination`) and the
     * live `useAgentStream` parser produce matching ids for the
     * same NDJSON line — that's the contract the
     * `agent-document-store` reducer's `HistoryLoaded` /
     * `StreamFlush` dedup relies on. The skip-callback is the
     * only deviation: the live parser skips over ids that happen
     * to be in the document at id-generation time.
     * `parseHistoryLines` doesn't supply a callback so its
     * counter sequence is unchanged.
     */
    private skipIdsFn: () => ReadonlySet<string> = STATIC_EMPTY_SKIP_SET_FN;
    private pendingToolCalls: Map<string, ToolCallEvent> = new Map();
    // Original call time, keyed by tool_use id — toolResultToNode() reads
    // this so a completed tool's peek-tooltip time reflects when it was
    // CALLED, not when the result arrived (reagent P1 on PR #2392: without
    // this, toolResultToNode() built a fresh node with no timestamp at all,
    // and useAgentStream.ts's generic backfill stamped Date.now() at
    // *result* time instead).
    private pendingToolTimestamps: Map<string, number> = new Map();
    private currentAgentId?: string;
    // Mutable node objects for accumulated text/thinking — content is appended in-place
    private currentTextNode: { type: "markdown"; id: string; content: string } | null = null;
    private currentThinkingNode: { type: "markdown"; id: string; content: string; timestamp?: number; metadata: { thinking: true } } | null = null;
    // True for the dedicated parser instance parseHistoryLines.ts creates to
    // batch-replay persisted NDJSON at pane-reopen time — false (default) for
    // the live useAgentStream.ts parser. thinking/tool_call events carry no
    // wire-level timestamp of their own (checked directly — neither
    // ThinkingEvent nor ToolCallEvent has one; only a few frame kinds like
    // the real compact_boundary system frame do), so there is no accurate
    // "when did this actually happen" available during replay. Stamping
    // Date.now() anyway would silently show every replayed thinking clump /
    // tool call as "just now" (reagent P2 on PR #2392) — leaving timestamp
    // unset here means the peek tooltip correctly shows no time rather than
    // a confidently wrong one.
    private isReplay: boolean;

    /**
     * `skipIds` accepts either a static `ReadonlySet<string>` (for
     * tests and single-pass uses where the document doesn't grow
     * out-of-band) OR a `() => ReadonlySet<string>` callback (for
     * the live `useAgentStream` parser that needs to observe
     * async snapshot restore). The callback form is the
     * production path.
     */
    constructor(opts?: {
        skipIds?: ReadonlySet<string> | (() => ReadonlySet<string>);
        /** See `isReplay`'s own doc comment above. Defaults to `false`
         *  (live-parsing behavior) — `parseHistoryLines.ts` is the only
         *  caller that passes `true`. */
        isReplay?: boolean;
    }) {
        if (opts?.skipIds) {
            this.skipIdsFn = typeof opts.skipIds === "function"
                ? opts.skipIds
                : ((s) => () => s)(opts.skipIds);
        }
        this.isReplay = opts?.isReplay ?? false;
    }

    /**
     * Generate the next node id of the given form (`node_N` /
     * `msg_N` / `user_N`), skipping any value that's already in
     * the document **as of right now** (per `skipIdsFn()`). The
     * skip-loop advances the counter past collisions so
     * subsequent ids don't repeat work. Pure increment when no
     * skip callback is supplied.
     */
    private nextIdOf(prefix: string): string {
        const skip = this.skipIdsFn();
        let id = `${prefix}_${this.nodeIdCounter++}`;
        while (skip.has(id)) {
            id = `${prefix}_${this.nodeIdCounter++}`;
        }
        return id;
    }

    /**
     * Parse NDJSON stream line by line
     */
    async *parse(stream: ReadableStream<Uint8Array>): AsyncGenerator<DocumentNode> {
        const reader = stream.getReader();
        const decoder = new TextDecoder();

        try {
            while (true) {
                const { done, value } = await reader.read();
                if (done) break;

                this.buffer += decoder.decode(value, { stream: true });
                const lines = this.buffer.split("\n");
                this.buffer = lines.pop() || ""; // Keep incomplete line

                for (const line of lines) {
                    if (!line.trim()) continue;

                    try {
                        const event = JSON.parse(line) as StreamEvent;
                        const node = this.eventToNode(event);
                        if (node) yield node;
                    } catch (err) {
                        console.error("Failed to parse NDJSON line:", line, err);
                    }
                }
            }
        } finally {
            reader.releaseLock();
        }
    }

    /**
     * Parse a single line of NDJSON
     */
    parseLine(line: string): DocumentNode | null {
        if (!line.trim()) return null;

        try {
            const event = JSON.parse(line) as StreamEvent;
            return this.eventToNode(event);
        } catch (err) {
            console.error("Failed to parse NDJSON line:", line, err);
            return null;
        }
    }

    /**
     * Parse a single event object (already parsed from JSON)
     * Returns array of nodes since some events may generate multiple nodes
     */
    async parseEvent(event: any): Promise<DocumentNode[]> {
        const node = this.eventToNode(event as StreamEvent);
        return node ? [node] : [];
    }

    /**
     * Parse a single event synchronously and return the resulting node.
     * Consecutive text/thinking events accumulate into the same node (same id,
     * content grows with each call).
     */
    parseStreamEvent(event: StreamEvent): DocumentNode | null {
        return this.eventToNode(event);
    }

    /**
     * Return all currently open accumulated nodes (text and/or thinking) and
     * close them so the next text/thinking event starts fresh.
     */
    flushPending(): DocumentNode[] {
        const nodes: DocumentNode[] = [];
        if (this.currentTextNode) nodes.push(this.currentTextNode);
        if (this.currentThinkingNode) nodes.push(this.currentThinkingNode);
        this.currentTextNode = null;
        this.currentThinkingNode = null;
        return nodes;
    }

    /**
     * Convert stream event to document node
     */
    private eventToNode(event: StreamEvent): DocumentNode | null {
        switch (event.type) {
            case "text":
                return this.textToNode(event as TextEvent);

            case "thinking":
                return this.thinkingToNode(event as ThinkingEvent);

            case "tool_call":
                this.currentTextNode = null;
                this.currentThinkingNode = null;
                return this.toolCallToNode(event as ToolCallEvent);

            case "tool_chunk":
                // tool_chunk does NOT produce a DocumentNode. It routes
                // through the agent-document reducer's ToolChunkAppend
                // command instead — see SPEC_TOOL_BLOCK_LIVE_LOG_2026_05_11.md
                // §3.3. Consumers (useAgentStream) detect this event
                // type ahead of calling the parser and dispatch the
                // command directly, then short-circuit. Returning null
                // here also makes parseLine safe for tool_chunk lines
                // that arrive in history replay.
                return null;

            case "tool_result":
                this.currentTextNode = null;
                this.currentThinkingNode = null;
                return this.toolResultToNode(event as ToolResultEvent);

            case "agent_message":
                this.currentTextNode = null;
                this.currentThinkingNode = null;
                return this.agentMessageToNode(event as AgentMessageEvent);

            case "user_message":
                this.currentTextNode = null;
                this.currentThinkingNode = null;
                return this.userMessageToNode(event as UserMessageEvent);

            case "error_result":
                this.currentTextNode = null;
                this.currentThinkingNode = null;
                return this.errorResultToNode(event as ErrorResultEvent);

            // Control events that intentionally produce no DocumentNode. The
            // live consumer (useAgentStream) handles these ahead of the parser
            // (`session_end` → finalizeTurn, `provider_waiting` → ProviderWaiting),
            // but history replay re-parses every persisted line through here —
            // returning null silently avoids a warn-spam flood (one per turn).
            case "session_end":
            case "provider_waiting":
                return null;

            default:
                console.warn("Unknown event type:", (event as any).type);
                return null;
        }
    }

    /**
     * Convert text event to markdown node.
     * Consecutive text deltas accumulate into the same mutable node (same id,
     * content appended). Switches away from thinking accumulation.
     */
    private textToNode(event: TextEvent): DocumentNode {
        this.currentThinkingNode = null;
        if (!this.currentTextNode) {
            this.currentTextNode = { type: "markdown", id: this.nextIdOf("node"), content: event.content };
        } else {
            this.currentTextNode = { ...this.currentTextNode, content: this.currentTextNode.content + event.content };
        }
        return { ...this.currentTextNode };
    }

    /**
     * Convert thinking event to markdown node with metadata.
     * Consecutive thinking deltas accumulate into the same logical node.
     * Switches away from text accumulation.
     */
    private thinkingToNode(event: ThinkingEvent): DocumentNode {
        this.currentTextNode = null;
        if (!this.currentThinkingNode) {
            // Stamped only on first creation of the clump — this is when the
            // clump started, not when it was last extended. Matches
            // toolCallToNode()'s Date.now() convention. See
            // docs/specs/SPEC_TRANSCRIPT_NODE_HOVER_PEEK_2026_08_03.md §2.4:
            // this was a real gap — thinking clumps never got a timestamp at
            // all before, unlike every other node kind.
            //
            // Gated on !isReplay (reagent P2 on PR #2392): ThinkingEvent
            // carries no wire timestamp, so during history replay Date.now()
            // would just be "when this pane was reopened", not "when the
            // thought happened" — every replayed clump would misleadingly
            // show "just now". Leave it unset in that case; see isReplay's
            // own doc comment.
            this.currentThinkingNode = {
                type: "markdown",
                id: this.nextIdOf("node"),
                content: event.content,
                timestamp: this.isReplay ? undefined : Date.now(),
                metadata: { thinking: true },
            };
        } else {
            this.currentThinkingNode = { ...this.currentThinkingNode, content: this.currentThinkingNode.content + event.content };
        }
        return { ...this.currentThinkingNode };
    }

    /**
     * Normalize a `tool_chunk` stream event into a `ToolLogChunk`
     * payload suitable for the agent-document reducer's
     * `ToolChunkAppend` command. Defaults `timestamp` to receive time
     * when the provider didn't supply one. Pure / no side effects —
     * does NOT touch `pendingToolCalls` or the text/thinking
     * accumulators, since chunks live in their own append-only
     * buffer (see SPEC_TOOL_BLOCK_LIVE_LOG_2026_05_11.md §3.1).
     */
    parseToolChunkEvent(event: ToolChunkEvent, now: number = Date.now()): { toolId: string; chunk: ToolLogChunk } {
        return {
            toolId: event.id,
            chunk: {
                kind: event.kind,
                content: event.content,
                timestamp: event.timestamp ?? now,
            },
        };
    }

    /**
     * Convert tool call event to tool node (running state)
     */
    private toolCallToNode(event: ToolCallEvent): DocumentNode {
        // Store pending tool call for when result arrives
        this.pendingToolCalls.set(event.id, event);
        // Capture ONE call timestamp, reused by every branch below and by
        // toolResultToNode() later — a tool's "time" is when it was called,
        // not touched again on re-entry (AskUserQuestion's placeholder ->
        // fully-parsed upgrade calls this twice for the same id; only the
        // first call should seed it).
        //
        // Gated on !isReplay (reagent P2 on PR #2392, same reasoning as
        // thinkingToNode() above): ToolCallEvent carries no wire timestamp
        // either, so Date.now() during history replay would be "when this
        // pane was reopened", not "when the tool was actually called".
        // undefined here (never set in the map) flows through to every
        // return site below and to toolResultToNode()'s fallback read —
        // left unset rather than confidently wrong.
        if (!this.isReplay && !this.pendingToolTimestamps.has(event.id)) {
            this.pendingToolTimestamps.set(event.id, Date.now());
        }
        const callTimestamp = this.pendingToolTimestamps.get(event.id);

        // AskUserQuestion is a tool the agent calls to consult the human; it
        // blocks the turn on a tool_result. When the params are fully parsed
        // (input.questions[]), surface it as `awaiting_answer` so the question
        // panel renders. Until the questions array lands (the streaming
        // placeholder tool_call has empty params), keep it `running` — a later
        // fully-parsed tool_call with the same id upgrades it in place.
        // Spec: SPEC_ASK_USER_QUESTION_2026_06_15.md.
        if (event.tool === "AskUserQuestion") {
            const questions = (event.params as { questions?: unknown })?.questions;
            if (Array.isArray(questions) && questions.length > 0) {
                return {
                    type: "tool",
                    id: event.id,
                    tool: this.normalizeToolName(event.tool),
                    toolName: event.tool,
                    params: event.params,
                    status: "awaiting_answer",
                    collapsed: false,
                    summary: "❓ Waiting for your answer",
                    timestamp: callTimestamp,
                    question: {
                        type: "ask_user_question",
                        tool_use_id: event.id,
                        questions: questions as import("./types").AskUserQuestionItem[],
                    },
                };
            }
        }

        const summary = this.generateToolSummary(event.tool, event.params, "running");

        return {
            type: "tool",
            id: event.id,
            tool: this.normalizeToolName(event.tool),
            toolName: event.tool,
            params: event.params,
            status: "running",
            collapsed: false, // Show running tools
            summary,
            timestamp: callTimestamp,
        };
    }

    /**
     * Convert tool result event to tool node (completed state)
     *
     * NOTE: This replaces the running tool node with same ID
     */
    private toolResultToNode(event: ToolResultEvent): DocumentNode {
        const toolCall = this.pendingToolCalls.get(event.id);
        const params = toolCall?.params || {};
        // Prefer tool name from the pending call (set during tool_use) over
        // event.tool which may be "Unknown" when the API doesn't include it.
        const toolName = (event.tool && event.tool !== "Unknown") ? event.tool : (toolCall?.tool || "Unknown");
        // Carry over the ORIGINAL call time (reagent P1 on PR #2392) — without
        // this, the node built here had no timestamp at all, and
        // useAgentStream.ts's generic "stamp a receive time" backfill filled
        // it in with the RESULT's arrival time instead, so a completed tool's
        // peek tooltip (the only state it's ever actually visible in — see
        // ToolBlock.tsx's `disable`) showed "when did this finish" mislabeled
        // as "when was this called". Falls back to now() only LIVE (never
        // during replay, reagent P2 — see isReplay's doc comment); a live
        // tool_result with no matching pending call shouldn't happen, but a
        // fallback there is still more useful than none, whereas during
        // replay it would just be "when this pane was reopened" again.
        const callTimestamp = this.pendingToolTimestamps.get(event.id) ?? (this.isReplay ? undefined : Date.now());

        // Remove from pending
        this.pendingToolCalls.delete(event.id);
        this.pendingToolTimestamps.delete(event.id);

        const summary = this.generateToolSummary(
            toolName,
            params,
            event.status,
            event.duration
        );

        return {
            type: "tool",
            id: event.id,
            tool: this.normalizeToolName(toolName),
            toolName,
            params,
            status: event.status,
            duration: event.duration,
            result: event.result,
            // Collapse on EVERY terminal state (success or failure).
            // The ✗ icon + red border-left in ToolBlock signal
            // failure at a glance; the user's feedback was that
            // failed-tool panels staying open forever cluttered the
            // pane. They get the same 5s post-completion hold as
            // successes — long enough to read the last lines, then
            // collapse to the single-line ✗ row with hover-to-peek.
            collapsed: true,
            summary,
            timestamp: callTimestamp,
        };
    }

    /**
     * Set the current agent ID for proper direction detection
     */
    setAgentId(agentId: string): void {
        this.currentAgentId = agentId;
    }

    /**
     * Convert agent message event to agent message node
     */
    private agentMessageToNode(event: AgentMessageEvent): DocumentNode {
        // Determine direction based on current agent ID
        // If we are the recipient (to === currentAgentId), it's incoming
        // If we are the sender (from === currentAgentId), it's outgoing
        const direction: "incoming" | "outgoing" =
            this.currentAgentId && event.to === this.currentAgentId
                ? "incoming"
                : "outgoing";

        const methodIcon = AGENT_MESSAGE_ICONS[event.method] || "📨";
        const directionIcon = DIRECTION_ICONS[direction];

        const summary =
            direction === "incoming"
                ? `${directionIcon} From ${event.from} (${event.method})`
                : `${methodIcon} To ${event.to} (${event.method})`;

        return {
            type: "agent_message",
            id: this.nextIdOf("msg"),
            from: event.from,
            to: event.to,
            message: event.message,
            method: event.method,
            direction,
            timestamp: event.timestamp || Date.now(),
            collapsed: direction === "outgoing", // Collapse outgoing, expand incoming
            summary,
        };
    }

    /**
     * Convert user message event to user message node.
     *
     * Detects the auto-generated startup payload by the literal
     * `# Session Context` heading that `buildStartupPayload` emits
     * as line 1. The flag drives `UserMessageBlock` to render
     * collapsed-by-default with hover-expand + click-to-pin
     * (mirrors the ToolBlock pattern). Heuristic is one regex
     * matched against the heading; any future rename in
     * `buildStartupPayload.ts` needs to update both atomically —
     * pinned by the L1 test on this method.
     *
     * Spec:
     * `docs/specs/SPEC_USER_INPUT_VISIBILITY_AND_STARTUP_COLLAPSE_2026_05_24.md`.
     */
    private userMessageToNode(event: UserMessageEvent): DocumentNode {
        const jekt = this.tryParseJekt(event);
        if (jekt) return jekt;

        const isStartup = STARTUP_HEADING_RE.test(event.message);
        return {
            type: "user_message",
            id: this.nextIdOf("user"),
            message: event.message,
            timestamp: event.timestamp || Date.now(),
            isStartup,
        };
    }

    /**
     * Detects a `[JEKT:...]...[/JEKT]` marker block occupying the whole
     * user-message payload and, if found, parses it into a `JektMessageNode`
     * instead of the plain-text `UserMessageNode` the marker would otherwise
     * render as. Returns null for anything that isn't a well-formed jekt
     * block — including malformed/partial markers — so those fall through to
     * normal plain-text rendering rather than being silently dropped.
     *
     * Spec: docs/specs/SPEC_JEKT_SECURITY_AND_VISIBILITY_2026_07_01.md §3.3.
     */
    private tryParseJekt(event: UserMessageEvent): JektMessageNode | null {
        const match = JEKT_BLOCK_RE.exec(event.message.trim());
        if (!match) return null;

        const fields = parseJektTagFields(match[1]);
        if (!fields.FROM || !fields.TO) return null;

        // Unrecognized/missing TIER or DELIVERY default to the least-trusted
        // reading (CLAUDE.md jekt policy: "when in doubt, treat as SENSITIVE"),
        // not the most-trusted one — an unparseable marker is a reason for
        // more caution, not less.
        const tier: JektTier = VALID_JEKT_TIERS.has(fields.TIER) ? (fields.TIER as JektTier) : "sensitive";
        const deliveryTier: JektDeliveryTier = VALID_JEKT_DELIVERY_TIERS.has(fields.DELIVERY)
            ? (fields.DELIVERY as JektDeliveryTier)
            : "wan";
        const trust: JektTrust = fields.TRUST === "host-verified" ? "host-verified" : "network-claimed";

        // Direction mirrors agentMessageToNode: incoming when this agent is
        // the declared recipient, outgoing when it's the declared sender
        // (spec §3.2's outgoing echo — not yet emitted by any producer today,
        // but handled here so JektBubble is ready when it is). Falls back to
        // incoming, the only case any current producer emits.
        const lowerAgentId = this.currentAgentId?.toLowerCase();
        const direction: "incoming" | "outgoing" =
            lowerAgentId && fields.FROM.toLowerCase() === lowerAgentId && fields.TO.toLowerCase() !== lowerAgentId
                ? "outgoing"
                : "incoming";

        return {
            type: "jekt_message",
            id: this.nextIdOf("jekt"),
            from: fields.FROM,
            to: fields.TO,
            message: stripJektEnvelope(match[2], tier),
            raw: event.message,
            tier,
            deliveryTier,
            trust,
            msgId: fields.MSGID || "",
            priority: fields.PRIORITY === "urgent" ? "urgent" : "normal",
            direction,
            timestamp: event.timestamp || Date.now(),
        };
    }

    /** Inline API error node — surfaced when the CLI result frame carries is_error:true. */
    private errorResultToNode(event: ErrorResultEvent): AgentErrorNode {
        return {
            type: "agent_error",
            id: this.nextIdOf("error"),
            code: event.code,
            message: event.message,
        };
    }

    /**
     * Generate tool summary string
     */
    private generateToolSummary(
        tool: string,
        params: Record<string, any>,
        status: string,
        duration?: number
    ): string {
        const icon = TOOL_ICONS[tool] || TOOL_ICONS.Other;
        const statusIcon = STATUS_ICONS[status] || "";
        const durationStr = duration ? ` (${duration.toFixed(1)}s)` : "";

        // Extract relevant param for display
        const detail = extractToolDetail(tool, params);

        return `${icon} ${tool} ${detail}${durationStr} ${statusIcon}`.trim();
    }

    /**
     * Normalize tool name to known type
     */
    private normalizeToolName(tool: string): ToolNode['tool'] {
        const normalized = tool.charAt(0).toUpperCase() + tool.slice(1).toLowerCase();
        const knownTools = ["Read", "Edit", "Bash", "Write", "Grep", "Glob", "Task", "Agent", "Workflow"];

        return knownTools.includes(normalized) ? (normalized as ToolNode['tool']) : "Other";
    }

    /**
     * Reset parser state
     */
    reset(): void {
        this.buffer = "";
        this.nodeIdCounter = 0;
        this.pendingToolCalls.clear();
        this.pendingToolTimestamps.clear();
        this.currentTextNode = null;
        this.currentThinkingNode = null;
        // skipIds intentionally NOT cleared — a `reset()` typically
        // follows a `StreamTruncate` (the snapshot's nodes are gone
        // from the doc) but we keep the original skip-set out of
        // caution. The counter restarts at 0 and the loop skips
        // anything that was in the snapshot, even if those ids no
        // longer exist in the document. Cost: a few wasted counter
        // increments at worst. Benefit: callers can't accidentally
        // re-introduce the collision by triggering a reset.
    }
}
