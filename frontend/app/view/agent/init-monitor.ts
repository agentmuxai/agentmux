// Copyright 2024, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * InitializationMonitor - Detects and handles Claude Code initialization prompts
 *
 * Monitors raw subprocess output for patterns like:
 * - "Choose theme (light/dark):"
 * - "Log in? (y/n):"
 *
 * When detected, updates initStateAtom to trigger UI prompt display.
 */

import type { Setter } from "solid-js";
import { globalStore, getApi } from "@/app/store/global";
import type { InitQuestion, InitState } from "./types";

/**
 * Pattern matching for initialization questions
 */
const INIT_PATTERNS = {
    theme: /(?:Choose|Select|Pick)\s+(?:your\s+)?theme.*?\(light\/dark\)|theme.*?\(light\/dark\)/i,
    login: /(?:Log\s+in|Login|Authenticate|Sign\s+in).*?\?.*?\(y\/n\)|Login\?\s+\(y\/n\)/i,
    generic: /\[.*\]\s*:/, // Catch generic prompts like "[Name]:"
} as const;

/**
 * Delay before marking initialization as complete (ms)
 * If no new questions appear within this time, assume init is done
 */
const COMPLETION_TIMEOUT = 3000;

export class InitializationMonitor {
    private buffer: string = "";
    private initStateAtom: Setter<InitState>;
    private completionTimer: NodeJS.Timeout | null = null;
    private active: boolean = false;

    constructor(initStateAtom: Setter<InitState>) {
        this.initStateAtom = initStateAtom;
    }

    /**
     * Start monitoring for initialization questions
     */
    start(): void {
        const log = (msg: string) => {
            console.log(msg);
            getApi().sendLog(msg);
        };

        log("[InitMonitor] ===== STARTING INITIALIZATION MONITORING =====");
        log(`[InitMonitor] Monitoring enabled: ${this.active}`);
        this.active = true;
        this.buffer = "";

        const newState = {
            phase: "spawning" as const,
            message: "Starting Claude Code...",
        };
        log(`[InitMonitor] Setting initial state: ${JSON.stringify(newState)}`);
        this.initStateAtom(newState);
        log(`[InitMonitor] Current state after set: ${JSON.stringify(newState)}`);
    }

    /**
     * Stop monitoring
     */
    stop(): void {
        console.log("[InitMonitor] Stopping initialization monitoring");
        this.active = false;
        this.buffer = "";
        this.clearCompletionTimer();
    }

    /**
     * Handle raw output chunk from subprocess
     *
     * @param chunk - Raw text from process output
     */
    handleRawOutput(chunk: string): void {
        const log = (msg: string) => {
            console.log(msg);
            getApi().sendLog(msg);
        };

        if (!this.active) {
            log("[InitMonitor] SKIPPING - monitor not active");
            return;
        }

        log("[InitMonitor] ===== RAW OUTPUT RECEIVED =====");
        log(`[InitMonitor] Chunk length: ${chunk.length}`);
        log(`[InitMonitor] Chunk content: ${JSON.stringify(chunk.slice(0, 200))}`);

        // Accumulate output in buffer
        this.buffer += chunk;

        // Keep only last 1000 characters to prevent unbounded growth
        if (this.buffer.length > 1000) {
            this.buffer = this.buffer.slice(-1000);
        }

        log(`[InitMonitor] Buffer (last 200 chars): ${this.buffer.slice(-200)}`);

        // Check for theme question
        if (INIT_PATTERNS.theme.test(this.buffer)) {
            log("[InitMonitor] ✅ DETECTED THEME QUESTION");
            this.emitQuestion({
                type: "theme",
                prompt: "Choose your theme preference for Claude Code",
                options: ["light", "dark"],
                expectsInput: true,
            });
            return;
        }

        // Check for login question
        if (INIT_PATTERNS.login.test(this.buffer)) {
            log("[InitMonitor] ✅ DETECTED LOGIN QUESTION");
            this.emitQuestion({
                type: "login",
                prompt: "Would you like to log in to Claude Code?",
                options: ["yes", "no"],
                expectsInput: true,
            });
            return;
        }

        // Check for generic prompts
        if (INIT_PATTERNS.generic.test(this.buffer)) {
            log("[InitMonitor] ✅ DETECTED GENERIC QUESTION");
            const match = this.buffer.match(/\[(.*?)\]\s*:/);
            const promptText = match ? match[1] : "Input required";
            this.emitQuestion({
                type: "other",
                prompt: promptText,
                expectsInput: true,
            });
            return;
        }

        // If no question detected, check for completion indicators
        this.checkForCompletion(chunk);
    }

    /**
     * Emit a question to be displayed in UI
     */
    private emitQuestion(question: InitQuestion): void {
        const log = (msg: string) => {
            console.log(msg);
            getApi().sendLog(msg);
        };

        log(`[InitMonitor] Emitting question: ${JSON.stringify(question)}`);

        // Clear buffer after detecting question to avoid re-detection
        this.buffer = "";

        // Clear any pending completion timer
        this.clearCompletionTimer();

        // Update state to show question
        const newState = {
            phase: "awaiting_response" as const,
            question: question,
        };
        this.initStateAtom(newState);
        log(`[InitMonitor] State updated to: ${JSON.stringify(newState)}`);
    }

    /**
     * Called after user responds to a question
     * Transitions to "processing" state and waits for next question or completion
     */
    responseProcessed(): void {
        console.log("[InitMonitor] Response processed, waiting for next question or completion");

        this.initStateAtom({
            phase: "processing",
            message: "Processing response...",
        });

        // Clear buffer to avoid re-detecting the same question
        this.buffer = "";

        // Start completion timer - if no new questions appear, assume init is done
        this.startCompletionTimer();
    }

    /**
     * Check if output contains indicators that initialization is complete
     */
    private checkForCompletion(chunk: string): void {
        // Look for ready indicators in the output
        const readyIndicators = [
            /ready/i,
            /initialized/i,
            /connected/i,
            /^>/, // Command prompt
            /\$/, // Shell prompt
        ];

        for (const indicator of readyIndicators) {
            if (indicator.test(chunk)) {
                console.log("[InitMonitor] Detected completion indicator:", indicator);
                this.startCompletionTimer();
                return;
            }
        }
    }

    /**
     * Start timer to mark initialization as complete
     */
    private startCompletionTimer(): void {
        this.clearCompletionTimer();

        this.completionTimer = setTimeout(() => {
            console.log("[InitMonitor] Completion timeout reached, marking as ready");
            this.completeInitialization();
        }, COMPLETION_TIMEOUT);
    }

    /**
     * Clear the completion timer
     */
    private clearCompletionTimer(): void {
        if (this.completionTimer) {
            clearTimeout(this.completionTimer);
            this.completionTimer = null;
        }
    }

    /**
     * Mark initialization as complete
     */
    private completeInitialization(): void {
        if (!this.active) return;

        console.log("[InitMonitor] Initialization complete");

        this.initStateAtom({
            phase: "ready",
            message: "Connected to Claude Code",
        });

        this.stop();
    }

    /**
     * Handle error during initialization
     */
    handleError(error: string): void {
        console.error("[InitMonitor] Initialization error:", error);

        this.initStateAtom({ phase: "error", error } as InitState);

        this.stop();
    }
}
