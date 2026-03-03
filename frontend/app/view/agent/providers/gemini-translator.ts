// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { StreamEvent } from "../types";
import type { OutputTranslator } from "./translator";

/**
 * Stub translator for Gemini CLI JSON output.
 * Passes through text content as TextEvents.
 */
export class GeminiTranslator implements OutputTranslator {
    translate(rawEvent: any): StreamEvent[] {
        if (!rawEvent || typeof rawEvent !== "object") return [];

        // Pass through events that already match StreamEvent format
        if (rawEvent.type === "text" && rawEvent.content) {
            return [{ type: "text", content: rawEvent.content }];
        }

        // Basic Gemini response handling: extract text from candidates
        if (rawEvent.candidates && Array.isArray(rawEvent.candidates)) {
            const events: StreamEvent[] = [];
            for (const candidate of rawEvent.candidates) {
                const parts = candidate.content?.parts;
                if (Array.isArray(parts)) {
                    for (const part of parts) {
                        if (part.text) {
                            events.push({ type: "text", content: part.text });
                        }
                    }
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
