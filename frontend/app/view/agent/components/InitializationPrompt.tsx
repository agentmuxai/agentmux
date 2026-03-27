// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * InitializationPrompt - Display and capture responses to Claude Code init questions
 *
 * Handles theme selection, login prompts, and generic text input.
 */

import { createSignal, Show, type JSX } from "solid-js";
import type { InitQuestion } from "../types";
import "./InitializationPrompt.scss";

interface InitializationPromptProps {
    question: InitQuestion;
    onResponse: (answer: string) => void;
}

export const InitializationPrompt = ({ question, onResponse }: InitializationPromptProps): JSX.Element => {
    const [textInput, setTextInput] = createSignal("");

    const handleSubmit = () => {
        if (textInput().trim()) {
            onResponse(textInput());
            setTextInput("");
        }
    };

    // Theme selection UI
    if (question.type === "theme") {
        return (
            <div class="init-prompt theme-prompt">
                <div class="init-prompt-header">
                    <div class="init-prompt-icon">🎨</div>
                    <div class="init-prompt-title">Choose Your Theme</div>
                </div>
                <div class="init-prompt-message">{question.prompt}</div>
                <div class="init-prompt-options">
                    <button
                        class="init-option-btn light"
                        onClick={() => onResponse("light")}
                        title="Use light theme"
                    >
                        ☀️ Light
                    </button>
                    <button
                        class="init-option-btn dark"
                        onClick={() => onResponse("dark")}
                        title="Use dark theme"
                    >
                        🌙 Dark
                    </button>
                </div>
            </div>
        );
    }

    // Login prompt UI
    if (question.type === "login") {
        return (
            <div class="init-prompt login-prompt">
                <div class="init-prompt-header">
                    <div class="init-prompt-icon">🔐</div>
                    <div class="init-prompt-title">Claude Code Login</div>
                </div>
                <div class="init-prompt-message">{question.prompt}</div>
                <div class="init-prompt-options">
                    <button
                        class="init-option-btn yes"
                        onClick={() => onResponse("y")}
                        title="Log in to Claude Code"
                    >
                        ✓ Yes
                    </button>
                    <button
                        class="init-option-btn no"
                        onClick={() => onResponse("n")}
                        title="Continue without logging in"
                    >
                        ✗ No
                    </button>
                </div>
            </div>
        );
    }

    // Generic text input fallback
    return (
        <div class="init-prompt generic-prompt">
            <div class="init-prompt-header">
                <div class="init-prompt-icon">❓</div>
                <div class="init-prompt-title">Input Required</div>
            </div>
            <div class="init-prompt-message">{question.prompt}</div>
            <div class="init-prompt-input-row">
                <input
                    type="text"
                    class="init-prompt-input"
                    placeholder="Enter response..."
                    value={textInput()}
                    onInput={(e) => setTextInput((e.target as HTMLInputElement).value)}
                    onKeyDown={(e) => {
                        if (e.key === "Enter") {
                            handleSubmit();
                        }
                    }}
                    autofocus
                />
                <button class="init-submit-btn" onClick={handleSubmit} disabled={!textInput().trim()}>
                    Submit
                </button>
            </div>
        </div>
    );
};

InitializationPrompt.displayName = "InitializationPrompt";
