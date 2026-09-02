// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Install hints per LSP-supported language. Backs the "Install <server>"
// banner shown when the server binary isn't on PATH.
// Spec: docs/specs/SPEC_EDITOR_LSP_AND_THEMES_2026-05-26.md § Install hints per language

export interface InstallHint {
    /** Human label for the server, e.g. "TypeScript language server" */
    serverName: string;
    /** The binary the supervisor tries to spawn */
    binary: string;
    /** Copy-paste install command shown in the banner */
    install: string;
    /** Docs link for advanced installs (homebrew, apt, etc.) */
    docs: string;
}

const HINTS: Record<string, InstallHint> = {
    typescript: {
        serverName: "TypeScript language server",
        binary: "typescript-language-server",
        install: "npm install -g typescript-language-server typescript",
        docs: "https://github.com/typescript-language-server/typescript-language-server",
    },
    javascript: {
        serverName: "TypeScript language server",
        binary: "typescript-language-server",
        install: "npm install -g typescript-language-server typescript",
        docs: "https://github.com/typescript-language-server/typescript-language-server",
    },
    rust: {
        serverName: "rust-analyzer",
        binary: "rust-analyzer",
        install: "rustup component add rust-analyzer",
        docs: "https://rust-analyzer.github.io/manual.html#installation",
    },
    python: {
        serverName: "pyright",
        binary: "pyright-langserver",
        install: "npm install -g pyright",
        docs: "https://microsoft.github.io/pyright/",
    },
    go: {
        serverName: "gopls",
        binary: "gopls",
        install: "go install golang.org/x/tools/gopls@latest",
        docs: "https://pkg.go.dev/golang.org/x/tools/gopls",
    },
    c: {
        serverName: "clangd",
        binary: "clangd",
        install: "(distro-packaged: `brew install llvm` / `apt install clangd` / Windows: LLVM installer)",
        docs: "https://clangd.llvm.org/installation",
    },
    cpp: {
        serverName: "clangd",
        binary: "clangd",
        install: "(distro-packaged: `brew install llvm` / `apt install clangd` / Windows: LLVM installer)",
        docs: "https://clangd.llvm.org/installation",
    },
};

export function installHintFor(language: string): InstallHint | null {
    return HINTS[language] ?? null;
}

/** Phase 1 ships LSP for TS/JS only. Adding to this set lights up additional
 *  languages once their server discovery + capabilities are validated. */
const LSP_SUPPORTED_LANGUAGES = new Set(["typescript", "javascript"]);

export function isLspSupportedLanguage(language: string): boolean {
    return LSP_SUPPORTED_LANGUAGES.has(language);
}
