// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { StreamEvent } from "../types";

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
}
