// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

export const PROVIDERS = [
    { id: "claude", label: "Claude Code", cmd: "claude --output-format stream-json" },
    { id: "codex", label: "Codex CLI", cmd: "codex --full-auto" },
    { id: "gemini", label: "Gemini CLI", cmd: "gemini --yolo" },
] as const;
