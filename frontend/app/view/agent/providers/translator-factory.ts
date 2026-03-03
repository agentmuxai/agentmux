// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { OutputTranslator } from "./translator";
import { ClaudeTranslator } from "./claude-translator";
import { GeminiTranslator } from "./gemini-translator";
import { CodexTranslator } from "./codex-translator";

/**
 * Create an OutputTranslator for the given output format.
 */
export function createTranslator(outputFormat: string): OutputTranslator {
    switch (outputFormat) {
        case "claude-stream-json":
            return new ClaudeTranslator();
        case "gemini-json":
            return new GeminiTranslator();
        case "codex-json":
            return new CodexTranslator();
        default:
            console.warn(`[translator-factory] Unknown output format "${outputFormat}", falling back to Claude translator`);
            return new ClaudeTranslator();
    }
}
