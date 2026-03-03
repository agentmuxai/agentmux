// Copyright 2024, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * InitializationPrompt - Display and capture responses to Claude Code init questions
 *
 * Handles theme selection, login prompts, and generic text input.
 */

import React, { memo, useState } from "react";
import type { InitQuestion } from "../types";
import "./InitializationPrompt.scss";

interface InitializationPromptProps {
    question: InitQuestion;
    onResponse: (answer: string) => void;
}

export const InitializationPrompt: React.FC<InitializationPromptProps> = memo(
    ({ question, onResponse }) => {
        const [textInput, setTextInput] = useState("");

        const handleSubmit = () => {
            if (textInput.trim()) {
                onResponse(textInput);
                setTextInput("");
            }
        };

        // Theme selection UI
        if (question.type === "theme") {
            return (
                <div className="init-prompt theme-prompt">
                    <div className="init-prompt-header">
                        <div className="init-prompt-icon">🎨</div>
                        <div className="init-prompt-title">Choose Your Theme</div>
                    </div>
                    <div className="init-prompt-message">{question.prompt}</div>
                    <div className="init-prompt-options">
                        <button
                            className="init-option-btn light"
                            onClick={() => onResponse("light")}
                            title="Use light theme"
                        >
                            ☀️ Light
                        </button>
                        <button
                            className="init-option-btn dark"
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
                <div className="init-prompt login-prompt">
                    <div className="init-prompt-header">
                        <div className="init-prompt-icon">🔐</div>
                        <div className="init-prompt-title">Claude Code Login</div>
                    </div>
                    <div className="init-prompt-message">{question.prompt}</div>
                    <div className="init-prompt-options">
                        <button
                            className="init-option-btn yes"
                            onClick={() => onResponse("y")}
                            title="Log in to Claude Code"
                        >
                            ✓ Yes
                        </button>
                        <button
                            className="init-option-btn no"
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
            <div className="init-prompt generic-prompt">
                <div className="init-prompt-header">
                    <div className="init-prompt-icon">❓</div>
                    <div className="init-prompt-title">Input Required</div>
                </div>
                <div className="init-prompt-message">{question.prompt}</div>
                <div className="init-prompt-input-row">
                    <input
                        type="text"
                        className="init-prompt-input"
                        placeholder="Enter response..."
                        value={textInput}
                        onChange={(e) => setTextInput(e.target.value)}
                        onKeyDown={(e) => {
                            if (e.key === "Enter") {
                                handleSubmit();
                            }
                        }}
                        autoFocus
                    />
                    <button className="init-submit-btn" onClick={handleSubmit} disabled={!textInput.trim()}>
                        Submit
                    </button>
                </div>
            </div>
        );
    }
);

InitializationPrompt.displayName = "InitializationPrompt";
