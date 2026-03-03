// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { StreamEvent } from "../types";
import type { OutputTranslator } from "./translator";

/**
 * Stub translator for Codex CLI JSON output.
 * Passes through text content as TextEvents.
 */
export class CodexTranslator implements OutputTranslator {
    translate(rawEvent: any): StreamEvent[] {
        if (!rawEvent || typeof rawEvent !== "object") return [];

        // Pass through events that already match StreamEvent format
        if (rawEvent.type === "text" && rawEvent.content) {
            return [{ type: "text", content: rawEvent.content }];
        }

        // Basic Codex response handling
        if (rawEvent.choices && Array.isArray(rawEvent.choices)) {
            const events: StreamEvent[] = [];
            for (const choice of rawEvent.choices) {
                const delta = choice.delta;
                if (delta?.content) {
                    events.push({ type: "text", content: delta.content });
                }
            }
            return events;
        }

        // Fallback: if there's a text field, use it
        if (rawEvent.text) {
            return [{ type: "text", content: rawEvent.text }];
        }

        return [];
    }

    reset(): void {
        // No state to reset
    }
}
