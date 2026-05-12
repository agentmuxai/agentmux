// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Agent pane session-replay fixture types.
 *
 * Shape and rationale: `docs/specs/SPEC_AGENT_PANE_SESSION_REPLAY_2026_05_12.md`.
 *
 * One `.session.ndjson` file per recorded session. Each line is a
 * single JSON object — header first, then events tagged by `src`,
 * optional trailer last. Events carry a relative `t_ms` and a stable
 * `seq` so replay schedulers can honor (or ignore) timing.
 */

import type { AgentDocumentCommand } from "@/app/store/agent-document/types";
import type { AgentPaneCommand } from "@/app/store/agent-pane-state/types";

/** Fixture metadata. Line 1 of every `.session.ndjson`. */
export interface FixtureHeader {
    kind: "header";
    /** Bump on incompatible format changes. */
    version: 1;
    /** AgentMux version the session was recorded against. */
    agentmux_version: string;
    /** v8 schema or whatever's active at record time. */
    schema_version: number;
    recorded_at: string;
    provider: "claude" | "codex" | "gemini" | string;
    block_id: string;
    instance_name: string;
    /** Which fields were redacted to placeholders at record time. */
    redactions: string[];
}

/** A single Claude Code stream-json line, opaque to the replay
 *  driver — handed verbatim to the real parser. */
export interface FixtureStreamEvent {
    seq: number;
    t_ms: number;
    src: "stream-json";
    /** Raw JSON text of one stream-json line. */
    line: string;
}

/** A WPS broker event. The replay driver matches `event` + `data.op`
 *  to translate into the right reducer command. */
export interface FixtureWpsEvent {
    seq: number;
    t_ms: number;
    src: "wps";
    /** Broker event name, e.g. `"tool_chunk"`, `"controllerstatus"`. */
    event: string;
    /** Scope filters (`["block:<id>"]`). */
    scopes: string[];
    /** Free-form payload. Shape depends on `event`. */
    data: unknown;
}

/** A reducer command dispatched by the frontend (not derived from
 *  the upstream channels). Captures user input + frontend-local
 *  state transitions during recording. */
export interface FixtureDispatchEvent {
    seq: number;
    t_ms: number;
    src: "dispatch";
    blockId: string;
    /** Either an agent-document or an agent-pane-state command. */
    action: AgentDocumentCommand | AgentPaneStateCommand;
    /** Which slot store the action targets. */
    store: "doc" | "pane";
}

/** Final-state assertions + recording stats. Optional last line. */
export interface FixtureTrailer {
    kind: "trailer";
    final_doc_node_count?: number;
    final_status?: string;
    wall_time_ms?: number;
    /** Free-form expectations for replay tests to assert. */
    expect?: Record<string, unknown>;
}

export type FixtureEvent =
    | FixtureStreamEvent
    | FixtureWpsEvent
    | FixtureDispatchEvent;

export type FixtureLine = FixtureHeader | FixtureEvent | FixtureTrailer;

/** Parsed fixture file. */
export interface Fixture {
    header: FixtureHeader;
    events: FixtureEvent[];
    trailer: FixtureTrailer | null;
}
