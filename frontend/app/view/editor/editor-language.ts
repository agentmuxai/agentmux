// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// ── Language detection ──────────────────────────────────────────────────────
// Maps file extensions to the canonical names the rest of the editor expects:
// `loadLanguage` in editor-view.tsx, LSP_SUPPORTED_LANGUAGES in install-hints.ts.
// The slice's `deriveLanguage` returns raw extensions ("ts", "py") — fine as a
// fallback label, but downstream lookups need the mapped form ("typescript",
// "python"). Keep this map in sync with `loadLanguage`'s switch.

export const EXTENSION_MAP: Record<string, string> = {
    ".ts": "typescript",
    ".tsx": "typescript",
    ".js": "javascript",
    ".jsx": "javascript",
    ".mjs": "javascript",
    ".cjs": "javascript",
    ".py": "python",
    ".rs": "rust",
    ".html": "html",
    ".htm": "html",
    ".css": "css",
    ".scss": "css",
    ".json": "json",
    ".md": "markdown",
    ".markdown": "markdown",
    ".yaml": "yaml",
    ".yml": "yaml",
    ".toml": "toml",
    ".sh": "shell",
    ".bash": "shell",
    ".zsh": "shell",
    ".ps1": "powershell",
    ".sql": "sql",
    ".go": "go",
    ".java": "java",
    ".c": "c",
    ".cpp": "cpp",
    ".h": "c",
    ".hpp": "cpp",
    ".xml": "html",
    ".svg": "html",
};

export function detectLanguage(filePath: string): string {
    const lower = filePath.toLowerCase();
    // Extension lookup. Use endsWith so multi-segment extensions like
    // `.test.tsx` still resolve via the trailing component.
    for (const [ext, lang] of Object.entries(EXTENSION_MAP)) {
        if (lower.endsWith(ext)) return lang;
    }
    // Special bareword filenames (no extension or unusual conventions).
    const name = lower.split(/[\\/]/).pop() ?? "";
    if (name === "dockerfile") return "shell";
    if (name === "makefile") return "shell";
    if (name === "cargo.toml" || name === "cargo.lock") return "toml";
    return "text";
}
