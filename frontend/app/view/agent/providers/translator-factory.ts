// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { OutputTranslator } from "./translator";
import { ClaudeTranslator } from "./claude-translator";
import { GeminiTranslator } from "./gemini-translator";
import { CodexTranslator } from "./codex-translator";
import { AcpTranslator } from "./acp-translator";
import { KimiTranslator } from "./kimi-translator";

/**
 * Create an OutputTranslator for the given output format.
 */
/**
 * `replay`: the translator reads stored history (parseHistoryLines) rather
 * than a live stream, where some providers' records mean something different
 * (e.g. a Gemini user-prompt echo that live is already shown optimistically).
 */
export function createTranslator(outputFormat: string, opts: { replay?: boolean } = {}): OutputTranslator {
    switch (outputFormat) {
        case "claude-stream-json":
            return new ClaudeTranslator({ replay: opts.replay });
        case "gemini-json":
            return new GeminiTranslator({ replay: opts.replay });
        case "codex-json":
            return new CodexTranslator({ replay: opts.replay });
        case "kimi-stream-json":
            return new KimiTranslator({ replay: opts.replay });
        case "acp":
            return new AcpTranslator();
        default:
            console.warn(`[translator-factory] Unknown output format "${outputFormat}", falling back to Claude translator`);
            return new ClaudeTranslator({ replay: opts.replay });
    }
}
