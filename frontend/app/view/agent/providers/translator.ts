// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { PermissionRequestEvent, StreamEvent } from "../types";

/**
 * Translates raw CLI output events into the internal StreamEvent format.
 * Each provider has its own translator implementation.
 */
export interface OutputTranslator {
    /**
     * Translate a raw event object (parsed from JSON) into zero or more StreamEvents.
     * Returns empty array if the event should be discarded (e.g., metadata events).
     */
    translate(rawEvent: any): StreamEvent[];

    /**
     * Reset any internal state (e.g., between sessions).
     */
    reset(): void;

    /**
     * Detect a per-tool-call permission request in the CLI's output and
     * synthesise a `PermissionRequestEvent`. Returns null if the input
     * isn't a permission gate. Today every implementation returns null
     * — the type hook lands in v1 PR-1; the actual detection arrives
     * per-provider in later PRs.
     *
     * `raw` is whatever shape the provider's stdout produced — a
     * stream-json event, a parsed Anthropic message, or a raw text
     * line for CLIs that prompt on tty. The translator decides.
     *
     * Spec: docs/specs/SPEC_DECISION_PROMPT_2026_04_24.md §9.
     */
    parsePermissionRequest?(raw: unknown): PermissionRequestEvent | null;
}
