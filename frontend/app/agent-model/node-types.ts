// Copyright 2024, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { ResumePreflightStep } from "@/app/store/rpc-api";
import type { MessageOrigin } from "@/types/rpc/MessageOrigin";
import type { ToolNode, ToolStreamingLog } from "./tool-types";

/**
 * Document node types that make up the agent's markdown document
 */
export type DocumentNode = (MarkdownNode | ToolNode | AgentMessageNode | UserMessageNode | ShellNode | AgentErrorNode | ContextCompactedNode | CompactionStartedNode | MemoryReinjectionNode | ContextDeliveryNode | JektMessageNode | SystemNoteNode | SessionOutcomeNode | CliNoticeNode | DayDividerNode | HistoryLinkNode | ResumePreflightNode | AmbientNarrationNode) & ViewModeTag | HiddenRunNode;

/**
 * How the pane's view (virtualization/verbosity.ts) asks a row to start: `collapsed` is a one-line
 * fold that opens only when pinned, `expanded` is open until folded. Set on a copy of the node by
 * the view's derivation, never on the stored node; absent means the row's own rule (`auto`).
 */
export type ViewMode = "expanded" | "collapsed";

export interface ViewModeTag {
    viewMode?: ViewMode;
}

/**
 * A run of consecutive rows the view hides, shown as one line ("12 tool calls, 3 thoughts").
 * A render-time synthetic like `DayDividerNode`: derived from the other nodes, never stored. Its id
 * comes from the first hidden row, so it stays the same when rows are appended.
 */
export interface HiddenRunNode {
    type: "hidden_run";
    id: string;
    /** Canonical row names to counts, in first-seen order. */
    counts: { kind: HiddenKind; count: number }[];
    /** How many of the hidden tool calls failed. */
    failed: number;
    /** The run is open: its rows follow it, in their full form. */
    open: boolean;
    /** The ids it hides, in order. */
    hiddenIds: string[];
}

export type HiddenKind = "tool" | "thinking" | "jekt" | "agent_message" | "note" | "setup";

/**
 * Raw markdown text block
 */
export interface MarkdownNode {
    type: "markdown";
    id: string;
    content: string; // Raw markdown text
    timestamp?: number; // Unix ms — when this node was first received
    metadata?: {
        thinking?: boolean; // Whether this is a thinking block
        /**
         * Set when the node was a thinking block whose stream
         * ended before completion (pane closed, app killed, etc.).
         * On the next session open, the orphan-scrub pass on
         * SessionEnd / HistoryLoaded flips this to `true` and
         * clears `thinking` — so the UI can render the partial
         * content as historical-canceled (collapsed by default,
         * "⏹ Canceled — partial thought" label) instead of
         * misleadingly showing a live "Thinking..." spinner on
         * something the agent will never finish.
         *
         * Spec: docs/specs/SPEC_ORPHAN_THINKING_NODES_2026_05_27.md.
         */
        canceled?: boolean;
        /** Unix ms when the orphan-scrub pass marked this node
         *  canceled. Optional; primarily for future audit/tooltip. */
        canceledAt?: number;
    };
}

/**
 * Agent-to-agent message (mux or ject)
 */
export interface AgentMessageNode {
    type: "agent_message";
    id: string;
    from: string; // Agent ID
    to: string; // Agent ID (this agent)
    message: string;
    method: "mux" | "ject"; // Mux = async mailbox, Ject = terminal injection
    direction: "incoming" | "outgoing";
    timestamp: number;
    collapsed: boolean;
    summary: string; // e.g., "📨 claude-1 → reviewer (mux)" or "📥 From claude-1 (mux)"
}

/** Sensitivity tier declared by the sender (or escalated on delivery). See
 *  docs/specs/SPEC_JEKT_SECURITY_AND_VISIBILITY_2026_07_01.md §4. */
export type JektTier = "info" | "coord" | "sensitive";

/** Network path the jekt crossed to reach this agent. */
/** `channel`: another AgentMux instance on the same machine. */
export type JektDeliveryTier = "host" | "channel" | "lan" | "wan";

/** Sender trust classification: how the sender's identity was proven, if at
 *  all. Only the `*-verified` labels mean a signature (or the caller's own
 *  identity token) proved it; see
 *  https://docs.agentmux.ai/internals/jekt-protocol/. */
export type JektTrust =
    | "host-verified"
    | "channel-verified"
    | "lan-verified"
    | "wan-verified"
    | "self-declared"
    | "unverified"
    | "network-claimed";

/**
 * A muxbus jekt message, parsed out of the `[JEKT:FROM=... TIER=... ...]` /
 * `[/JEKT]` marker block that `wrap_jekt_message` (agentmux-srv) and
 * `wrapJektMessage` (muxbus-cloud) wrap around injected text before it lands
 * in the agent's input stream. Rendered as an `OriginContainer` (its agent variant) instead of the raw
 * marker text.
 *
 * Spec: docs/specs/SPEC_JEKT_SECURITY_AND_VISIBILITY_2026_07_01.md §3.3.
 */
export interface JektMessageNode {
    type: "jekt_message";
    id: string;
    from: string; // Agent ID (sender)
    to: string; // Agent ID (recipient)
    /** Inner message text with the marker's presentational scaffolding
     *  (divider lines, From/To header, sensitive warning, reply hint)
     *  stripped — what the sender actually typed/sent. */
    message: string;
    /** Full original `[JEKT:...]...[/JEKT]` block, unmodified. Shown in the
     *  expanded metadata view (spec §3.3: "raw payload"). */
    raw: string;
    tier: JektTier;
    deliveryTier: JektDeliveryTier;
    trust: JektTrust;
    msgId: string;
    priority: "normal" | "urgent";
    direction: "incoming" | "outgoing";
    /** When it was sent: for a held message, the original send time from
     *  the marker's `TS`, not the time it was finally delivered. */
    timestamp: number;
    /** Seconds the message waited for an absent recipient (`HELD_FOR`,
     *  SPEC_DURABLE_JEKT_DELIVERY_2026_09_24.md §2.4); absent when it was
     *  delivered live. */
    heldForSecs?: number;
    /** Files the sender sent with it, read back from srv's `<attached_files>`
     *  list (marker `FILES=`; SPEC_JEKT_FILE_TRANSFER_2026_10_05.md §6.2).
     *  Absent when none came. */
    attachments?: { id?: string; name: string }[];
}

/**
 * One of AgentMux's own notes (`[AgentMux] ...`: an upgrade, cloud messages
 * paused, an overlap) that srv delivered to the agent, alone or after another
 * message. Rendered as an `OriginContainer`, never as the human's turn
 * (SPEC_MESSAGE_ORIGIN_AND_ONE_CONTAINER_2026_10_25.md, P0; P2 makes it the
 * `system` variant of the one origin container).
 */
export interface SystemNoteNode {
    type: "system_note";
    id: string;
    /** The note without its `[AgentMux]` marker. */
    text: string;
    /** "sent 14:30, delivered 14:41" when it arrived late. */
    delay?: string;
    timestamp?: number;
    /**
     * `cli`: a turn the CLI wrote itself (a task notification, a system reminder, a hook's output),
     * shown as a "CLI message" and never in the human's style. `earlier`: a record with no origin srv
     * signed (written before origins, or one that failed the check), shown as an "Earlier message".
     * Absent: a note from AgentMux, or a message whose `origin` says what it is.
     */
    source?: "cli" | "earlier";
    /** For a `cli` note: the wrapper text as the CLI wrote it (the dock reads task notifications from it). */
    raw?: string;
    /**
     * The origin srv signed for this message (`amx.origin`, read back only when it verified). Set on
     * the container's non-jekt variants (a notice, a schedule, a task, a supervisor turn...); the
     * container's label and badges come from it, never from the text.
     */
    origin?: MessageOrigin;
    /** Shown beside the label: a supervisor turn's "steady 2 of 5". */
    detail?: string;
    /** Files or images sent with an "Earlier message", read back from the record's attachment list. */
    attachments?: { id?: string; name: string }[];
}

/**
 * User message to agent.
 *
 * `collapsed` + `summary` fields were retired in PR #1020 — the
 * renderer (`UserMessageBlock`) keys its render shape off
 * `isStartup` + `documentState.pinnedNodes`, not per-node mutable
 * state. The agent-message variant still carries `collapsed` /
 * `summary` because its toggle UI lives on the document-level
 * `collapsedNodes` set; user messages have their own pin model.
 */
export interface UserMessageNode {
    type: "user_message";
    id: string;
    message: string;
    /** Unix ms. Absent when unknown — a replayed message whose line has no
     *  stored receive time; never an invented "now" (ReAgent P1, #3620).
     *  Renderers already treat a missing value as unknown. */
    timestamp?: number;
    /** True when this row is the auto-generated startup context
     * payload (see `buildStartupPayload.ts` + onReadyFn in
     * `agent-view.tsx`). The renderer surfaces these
     * collapsed-by-default with hover-expand + click-to-pin,
     * mirroring the ToolBlock pattern — they otherwise dominate
     * the pane on every fresh agent launch.
     *
     * Detected by stream-parser via the literal `# Session Context`
     * heading the builder emits. Frontend-only marker; never
     * round-trips through the agent CLI.
     *
     * Spec:
     * `docs/specs/SPEC_USER_INPUT_VISIBILITY_AND_STARTUP_COLLAPSE_2026_05_24.md`. */
    isStartup?: boolean;
    /** The pane sent it on AgentMux's behalf (the start-up document), so a retry sends it so again. */
    system?: boolean;
    /** Images sent with the message, in the order the agent numbered them.
     *  An entry without `id` was no longer available when it was sent.
     *  SPEC_AGENT_PANE_IMAGE_ATTACHMENTS_2026_09_26.md §5.8. */
    attachments?: { id?: string; name: string }[];
    /** Set when this turn was one copy of a Swarm broadcast: the user typed it
     *  once and sent it to several agents. `message` is then the body, with the
     *  `[BROADCAST:...]` header srv prepended already removed.
     *  SPEC_SWARM_BROADCAST_AS_USER_MESSAGE_2026_10_01.md §4.5. */
    broadcast?: { recipients?: number; msgId?: string };
    /** Set when the Swarm Supervisor sent this turn to keep the agent going:
     *  `message` is then the body, with srv's `[SUPERVISOR:...]` header removed.
     *  SPEC_SWARM_SUPERVISOR_2026_10_16.md §3.8. */
    supervisor?: { mode: "steady" | "drive"; n: number; of: number };
    /** The paired device the user sent this from (their phone), when they did:
     *  shown as "From <device>". From the stored record's `amx`, which only srv
     *  writes, never from the text. agentmux-mobile's
     *  SPEC_SEND_FROM_THE_PHONE_2026_10_13 §5.3. */
    fromDevice?: string;
}

/**
 * Persistent shell — a long-running process launched by the agent via the
 * Shell tool (agentmux-mcp). Lives in the document as a single compact row
 * that expands inline to show the live-log.
 *
 * Unlike ToolNode, a ShellNode is not tied to a tool-call lifecycle —
 * the shell runs independently after the Shell tool returns.
 *
 * Spec: docs/specs/SPEC_PERSISTENT_SHELL_NODE_2026_06_11.md
 */
export interface ShellNode {
    type: "shell";
    id: string;
    cmd: string;
    title: string;
    cwd?: string;
    status: "running" | "exited-ok" | "exited-err" | "stopped";
    exitCode?: number;
    spawnedAt: number;   // Unix ms
    exitedAt?: number;   // Unix ms
    log: ToolStreamingLog;
}

/**
 * Inline API error node rendered in the transcript when a session ends with
 * is_error:true (e.g. 401 auth failure, 429 rate-limit). Not collapsible.
 * Spec: SPEC_AGENT_ERROR_FRAMEWORK_2026_06_20.md §P1.3
 */
export interface AgentErrorNode {
    type: "agent_error";
    id: string;
    code: number;   // HTTP status code (e.g. 401, 429)
    message: string; // Full error text from the CLI result event
    /** A sign-in has answered this auth error: the row is muted and offers no "Login Again". Set at display time. */
    resolved?: boolean;
}

/**
 * Transcript boundary marker inserted when Claude compacts its context.
 *
 * `source: "real"` — sourced from the backend's authoritative
 * `CompactionBoundary` event (Claude Code's `compact_boundary` frame,
 * exact data). `trigger`/`durationMs` are present.
 * `source: "heuristic"` — the pre-existing cross-provider fallback,
 * inferred from a ≥50%-drop in observed input tokens. `trigger`/
 * `durationMs` are absent (not knowable from the heuristic alone).
 * See docs/specs/SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md §4.3.
 */
export interface ContextCompactedNode {
    type: "context_compacted";
    id: string;
    tokensBefore: number;
    /**
     * For a real boundary, the CLI's `post_tokens`: the SUMMARY's size only, not
     * the system prompt and tools every call carries (CLI 2.1.288: 1,417, with
     * the next call's prompt at 39,490). For the heuristic, the next call's
     * whole prompt.
     */
    tokensAfter: number;
    /**
     * The context's real size after a real compaction: the next main-agent
     * call's whole prompt. Filled in when that call reports (live and in
     * history replay); absent until then.
     */
    contextAfter?: number;
    timestamp: number;
    source: "real" | "heuristic";
    trigger?: "manual" | "auto";
    /** Real compaction duration in ms — only present when `source === "real"`. */
    durationMs?: number;
}

/**
 * Live "compaction is happening now" announcement — pushed the instant
 * the `PreCompact` hook's `compaction_started` signal lands (Tier 1 of
 * docs/specs/SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md).
 * Deliberately a SEPARATE node type from `ContextCompactedNode`: this
 * one represents an in-progress announcement with no outcome data yet,
 * while `ContextCompactedNode` is the completed record (real or
 * heuristic) — conflating the two would make a still-running
 * compaction look like a finished one in the transcript.
 */
export interface CompactionStartedNode {
    type: "compaction_started";
    id: string;
    trigger: "manual" | "auto";
    startedAt: number;
}

/**
 * Label-only record of a hidden Global Memory + Personal Memory reinjection,
 * fired once per real `CompactionBoundary` event. See
 * docs/specs/SPEC_HIDDEN_MEMORY_REINJECTION_AFTER_COMPACTION_2026_09_22.md
 * §3.2. Deliberately carries NO content field — the actual memory bodies are
 * sent to the model in a real message this node never stores or renders;
 * that is the hiding mechanism itself (nothing to leak, not something hidden
 * via CSS/a collapsed-by-default flag on real content).
 */
export interface MemoryReinjectionNode {
    type: "memory_reinjection";
    id: string;
    globalMemoryCount: number;
    personalMemoryCount: number;
    /** Sum of estimateTokens() over every entry sent — labeled "(est.)" wherever shown, per §2/§4. */
    estimatedTokens: number;
    perEntryTokens: Array<{
        label: string;
        source: "global" | "personal";
        tokens: number;
        sizeBytes: number;
    }>;
    /** Real on-disk byte totals per source — §3.4.1. Not an estimate, unlike estimatedTokens. */
    totalSizeBytes: { global: number; personal: number };
    /** Driven by Personal-memory bytes alone, never the combined total — §3.4.2. */
    sizeBand: "low" | "mid" | "high" | "critical";
    at: number;
}

/**
 * One piece of content the agent was given without the user typing it.
 * docs/specs/SPEC_CONTEXT_DELIVERY_2026_09_30.md §3.1.
 */
export interface ContextItem {
    kind:
        | "global_memory"
        | "personal_memory"
        | "running_summary"
        | "compaction_summary"
        | "command_output"
        | "continuation_packet"
        | "startup_file";
    /** Entry name, file name, or a fixed label for the kind. */
    name: string;
    /** Global Memory only: AgentMux's own system entry, or the workspace's. */
    tier?: "system" | "workspace";
    /** Personal Memory only: the file it was read from. */
    path?: string;
    /** Global Memory only: the entry's id. */
    bundleId?: string;
    /** How much of it went out when a delivery was cut (spec §3.4 step 3). Absent = full. */
    delivered?: "full" | "partial" | "omitted";
    /** Of what was delivered: a partial item's slice, 0 when omitted. */
    sizeBytes: number;
    /** The whole entry's estimated tokens, whatever was delivered (memory items). */
    sourceTokens?: number;
    /** Estimated — labelled "(est.)" wherever shown. */
    tokens: number;
    /** ≤160 chars, shown while the card is collapsed. */
    excerpt?: string;
    /** The full text, when the transcript already holds it (the compaction summary). */
    body?: string;
    /**
     * `startup_file` only: a file the CLI read by itself at session start
     * (SPEC_LAUNCH_CONTEXT_WORKSPACE_RULE_AND_STARTUP_FILES_2026_09_30.md §4).
     * `role`: `user_instructions`, `instructions`, `instructions_import`,
     * `skills` or `mcp_servers`.
     */
    role?: string;
    /** `startup_file` only: who writes it. `external` = outside the workspace, not AgentMux's. */
    owner?: "agentmux" | "user" | "external";
    /** `startup_file` skills / MCP servers: how many are listed (they carry no size). */
    count?: number;
    /** `startup_file` only: AgentMux sections it carries, e.g. `global_memory`. */
    contains?: string[];
    /**
     * `startup_file`: a memory entry the agent got through its startup file
     * rather than the hook, so it has no size of its own (the file's row
     * counts it). SPEC_LAUNCH_CONTEXT_WORKSPACE_RULE_AND_STARTUP_FILES_2026_09_30.md §4.4.
     */
    via?: "startup_file";
}

/**
 * A context delivery: everything the agent was given at one moment, shown as
 * one card with a row per item. The compaction summary is the first kind
 * (CD1): Claude Code writes it after `/compact` and echoes it as a user frame,
 * which used to render as if the user had typed it.
 * docs/specs/SPEC_CONTEXT_DELIVERY_2026_09_30.md §3.
 */
export interface ContextDeliveryNode {
    type: "context_delivery";
    /** Deterministic, so live and replay land on the same node. */
    id: string;
    /** `command`: a local slash command's output the CLI put in the conversation. */
    reason: "startup" | "clear" | "compaction" | "resume_fresh" | "command";
    /** Compaction only. */
    trigger?: "manual" | "auto";
    items: ContextItem[];
    timestamp: number;
    /** Memory deliveries: how large Personal Memory is against the context window. */
    sizeBand?: "low" | "mid" | "high" | "critical";
    /** A fallback re-delivery's card (CD2b): it stands for its hidden message. */
    fallback?: boolean;
}

/**
 * Transcript boundary marker for a `--resume` attempt's outcome — sourced
 * from AgentMux's own `agentmux_session_outcome` frame (the backend's
 * `persistent_resume` state machine, not the provider CLI). `"resumed"`
 * means the CLI genuinely continued the prior session; `"fresh"` means it
 * could not, and the model has none of the turns before this marker even
 * though the pane's scrollback above it is still fully visible. See
 * docs/specs/SPEC_AGENT_PANE_HISTORY_ALIGNMENT_2026_08_05.md §2.2.
 */
export interface SessionOutcomeNode {
    type: "session_outcome";
    id: string;
    outcome: "resumed" | "fresh";
    attemptedSid: string;
    actualSid: string | null;
    /** A `fresh` session given AgentMux's record of the conversation (see `SessionOutcomeData.continued`). */
    continued?: boolean;
    timestamp: number;
}

/**
 * A notice about the agent's CLI, from srv's own `agentmux_cli_install` /
 * `agentmux_cli_version_changed` frames (`cli-notice.ts`;
 * SPEC_LAUNCH_CONTEXT_WORKSPACE_RULE_AND_STARTUP_FILES_2026_09_30.md §6.3), or
 * the CLI's own `system/status` frame reporting a failed compaction
 * (SPEC_COMPACTION_ESTIMATED_PROGRESS_AND_STREAM_FRAMES_2026_10_01.md §10).
 * For the user only: the model never sees it.
 */
export interface CliNoticeNode {
    type: "cli_notice";
    id: string;
    kind: "install" | "version_changed" | "compaction_failed" | "mcp_health" | "container_start" | "container_image" | "container_limit" | "container_caches" | "agent_ready";
    /** Provider id, e.g. `"claude"`. */
    provider: string;
    /** `install`: the version being installed. */
    version?: string;
    state?: "installing" | "installed" | "failed";
    /** `install`, once installed, and `agent_ready`: how long it took. */
    seconds?: number;
    /** `install` or `compaction_failed`: why it failed, when known. */
    error?: string;
    /** `version_changed`: the version the agent last ran, and now runs. */
    from?: string;
    to?: string;
    /** `version_changed`: the version AgentMux pins, when known. */
    pinned?: string | null;
    /** `container_start`: where a container agent's start-up is. */
    startState?: "pulling" | "starting" | "ready" | "failed";
    /** `container_start`, while `pulling`: how much of the image has arrived, once the daemon has said. */
    percent?: number;
    /** `container_start`: the image being started. */
    image?: string;
    /** `container_image`: the container runs an older agent image (`outdated`), has been removed so
     *  the next message starts it from the current one (`updated`), or was moved to the current one by AgentMux
     *  while the agent was idle (`auto_updated`). `running` and `current` are the
     *  image versions; `blockId` is the pane the update acts on. */
    imageState?: "outdated" | "updated" | "auto_updated";
    running?: string;
    current?: string;
    blockId?: string;
    /** `container_limit`: a container's limit changed by someone (`changed`), a process in it was stopped for going
     *  over its memory limit (`oom`), or Docker refused a new limit (`failed`). `memoryMb` and `cpus` are the limits
     *  in force, `by` who changed them, `agentId` and `agentName` whose container it is. */
    limitState?: "changed" | "oom" | "failed";
    memoryMb?: number;
    cpus?: number;
    by?: string;
    agentId?: string;
    agentName?: string;
    /** `container_caches`: how much the agent's cache volumes hold, in bytes (`agentId` and `agentName` as for
     *  `container_limit`). */
    cacheBytes?: number;
    /** `mcp_health`: where the repair of the agent's AgentMux MCP server is. */
    mcpState?: "healing" | "restarting" | "ok" | "gave_up";
    /** `mcp_health`, while `healing`: which reconnect. */
    step?: number;
    /** `mcp_health`: the agent runs without the shell prefix (the safe profile). */
    safeProfile?: boolean;
    timestamp: number;
}

/**
 * Calendar-day separator in the Agent History view. A render-time synthetic
 * (injected by `injectDayDividers`, never persisted, never part of the live
 * document store) — id `day-<YYYY-MM-DD>` is content-derived so pagination
 * prepends recompute to the identical node instead of duplicating.
 * Spec: SPEC_AGENT_PANE_SESSION_SCOPED_SCROLLBACK_AND_AGENT_HISTORY_VIEW_2026_08_09.md §4.4.
 */
export interface DayDividerNode {
    type: "day_divider";
    id: string;
    /** Human label, e.g. "Sun, Aug 10 2026". */
    dayLabel: string;
    /** Local midnight of the day, unix ms. */
    timestamp: number;
}

/**
 * "Open Agent History" link row in the LIVE working document. A render-time
 * synthetic (injected by `injectHistoryLink`, never persisted, never part of
 * the reducer store) — scrolls with the transcript instead of being pinned
 * above it, since opening it just switches/opens a pane tab (§3.2 of
 * SPEC_AGENT_HISTORY_AS_TAB_AND_DRAFT_PRESERVATION_2026_08_11.md). Never
 * injected into the Agent History reader itself — that view has no need to
 * link to itself. Fixed id: there is at most one per live document.
 */
export interface HistoryLinkNode {
    type: "history_link";
    /** `history-link` at the top of the feed; `history-gap:<next id>` between
     *  turns kept around a blocked one (spec §6.9). */
    id: string;
    /** Turns rolled off above this row, when known. */
    earlierTurns?: number;
    /** A row between kept turns rather than at the top. */
    gap?: boolean;
}

/**
 * Pane-open continuity notice — "the conversation below won't be continued",
 * known before anything is spawned (`useResumePreflight`, backed by
 * `crates/srv/src/backend/resume_preflight.rs`).
 *
 * A render-time synthetic like `HistoryLinkNode`: injected by
 * `injectResumePreflight`, never persisted, never in the reducer store. It is
 * a document node rather than a banner because it belongs in the transcript
 * it is making a claim about — and because when this was written the pane's
 * only banner host (`AgentControlBar`) rendered solely while the Shell drawer
 * was open, which would have let the notice be missed entirely. That host is
 * gone: `AgentSessionNotices` now renders above the composer unconditionally
 * (SPEC_AGENT_SHELL_DRAWER_INFO_PANEL_2026_09_19.md §3), so visibility alone
 * no longer rules a banner out.
 *
 * Distinct from `SessionOutcomeNode`, which reports what a resume attempt
 * *did*. This reports what the next one *will* do, and disappears once the
 * real outcome is known — the two never claim to describe the same event.
 *
 * `pending` is the progress-list state: the preflight is still running and has
 * been for long enough to be worth showing (`STEPS_VISIBLE_AFTER_MS`).
 */
export interface ResumePreflightNode {
    type: "resume_preflight";
    id: "resume-preflight";
    /** Omitted while `pending` — there's no verdict yet. */
    verdict?: "fresh" | "recover";
    /** A real session on disk that the next spawn won't reach for. */
    recoverableSessionId?: string;
    pending: boolean;
    steps: ResumePreflightStep[];
}

/**
 * A short line AgentMux generated about something it did on its own (first
 * consumer: a tool call the harness detached to the background), rendered
 * in-flow in the agent's own prose styling with a small trailing `ambient` tag.
 *
 * Deliberately NOT a `MarkdownNode`: markdown means "model output", and code
 * that walks the document on that assumption must not start treating AgentMux's
 * own sentence as something the model said. The model did not write these and
 * the CLI transcript does not contain them — the tag exists to keep that true
 * when the line is read back later.
 *
 * View-only: never sent to the CLI, so it cannot alter the model's next turn.
 * Spec: docs/specs/SPEC_AMBIENT_NARRATION_INLINE_AGENT_VOICE_2026_09_24.md.
 */
export interface AmbientNarrationNode {
    type: "ambient_narration";
    id: string;
    /** The backend's prompt selector, e.g. `"background_task"`. */
    kind: string;
    text: string;
    timestamp: number;
}

/** The kind of a document node: the `type` of one of the `DocumentNode` shapes. */
export type NodeKind = DocumentNode["type"];
