// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Map a file path (absolute or relative) or a raw first-line shebang to a
 * Shiki language id. Returns "text" for unknown inputs so callers can always
 * route through HighlightedCode without branching.
 *
 * Detection order (first hit wins):
 *   1. Extension map  — covers the common programmer file types
 *   2. Basename match — Dockerfile, Makefile, .gitignore, .env*
 *   3. Shebang scan   — #!/usr/bin/env python3, #!/bin/bash, etc.
 *   4. Fallback       — "text"
 *
 * Spec: docs/specs/SPEC_TOOL_OVERLAY_CODE_HIGHLIGHTING_2026_04_14.md §4.2
 */

const EXT_MAP: Record<string, string> = {
    // TypeScript / JavaScript
    ts: "typescript",
    tsx: "tsx",
    js: "javascript",
    jsx: "jsx",
    mjs: "javascript",
    cjs: "javascript",
    // Python
    py: "python",
    pyw: "python",
    // Rust
    rs: "rust",
    // Go
    go: "go",
    // Shell
    sh: "bash",
    bash: "bash",
    zsh: "bash",
    fish: "fish",
    ps1: "powershell",
    psm1: "powershell",
    // Markup / config
    md: "markdown",
    mdx: "markdown",
    json: "json",
    jsonc: "jsonc",
    yaml: "yaml",
    yml: "yaml",
    toml: "toml",
    xml: "xml",
    html: "html",
    htm: "html",
    // CSS
    css: "css",
    scss: "scss",
    sass: "scss",
    less: "less",
    // SQL
    sql: "sql",
    // Ruby
    rb: "ruby",
    // Java / JVM
    java: "java",
    kt: "kotlin",
    kts: "kotlin",
    // Swift
    swift: "swift",
    // C / C++
    c: "c",
    h: "c",
    cpp: "cpp",
    cxx: "cpp",
    cc: "cpp",
    hpp: "cpp",
    hxx: "cpp",
    // C#
    cs: "csharp",
    // PHP
    php: "php",
    // Lua
    lua: "lua",
    // Web frameworks
    vue: "vue",
    svelte: "svelte",
    // Infrastructure
    tf: "terraform",
    hcl: "hcl",
    graphql: "graphql",
    gql: "graphql",
    // Other
    r: "r",
    dart: "dart",
    ex: "elixir",
    exs: "elixir",
    elm: "elm",
    hs: "haskell",
    clj: "clojure",
    cljs: "clojure",
    scala: "scala",
    groovy: "groovy",
    pl: "perl",
    dockerfile: "dockerfile",
    lock: "text",
    log: "text",
};

/** Exact basename matches (case-insensitive). */
const BASENAME_MAP: Record<string, string> = {
    dockerfile: "dockerfile",
    makefile: "makefile",
    gnumakefile: "makefile",
    rakefile: "ruby",
    gemfile: "ruby",
    podfile: "ruby",
    vagrantfile: "ruby",
    ".gitignore": "ignore",
    ".npmignore": "ignore",
    ".dockerignore": "ignore",
    ".gitattributes": "ini",
    ".editorconfig": "ini",
    ".babelrc": "json",
    ".eslintrc": "json",
    ".prettierrc": "json",
};

/** Map shebang interpreter names to Shiki lang ids. */
const SHEBANG_MAP: Record<string, string> = {
    python: "python",
    python3: "python",
    python2: "python",
    node: "javascript",
    nodejs: "javascript",
    bash: "bash",
    sh: "bash",
    zsh: "bash",
    fish: "fish",
    ruby: "ruby",
    perl: "perl",
    php: "php",
    lua: "lua",
    "deno": "typescript",
};

/**
 * Detect language from a file path. Pass the full content to enable
 * shebang detection for extension-less files.
 */
export function detectLanguage(filePath: string, firstLine?: string): string {
    const base = filePath.split(/[\\/]/).pop() ?? filePath;

    // 1. Extension map
    const dotIdx = base.lastIndexOf(".");
    if (dotIdx > 0) {
        const ext = base.slice(dotIdx + 1).toLowerCase();
        if (EXT_MAP[ext]) return EXT_MAP[ext];
    }

    // 2. Basename match (case-insensitive)
    const baseLower = base.toLowerCase();
    if (BASENAME_MAP[baseLower]) return BASENAME_MAP[baseLower];

    // Handle .env, .env.local, .env.production etc.
    if (baseLower === ".env" || baseLower.startsWith(".env.")) return "bash";

    // 3. Shebang scan
    if (firstLine?.startsWith("#!")) {
        // #!/usr/bin/env python3  or  #!/bin/bash
        const parts = firstLine.replace(/^#!\s*/, "").split(/\s+/);
        // Last segment of the path is the interpreter name (e.g. "python3" from "/usr/bin/env python3")
        const interp = (parts[0].includes("/") ? parts[0].split("/").pop() : parts[0]) ?? "";
        const interpLower = interp.toLowerCase().replace(/\d+$/, ""); // strip trailing version
        if (SHEBANG_MAP[interp.toLowerCase()]) return SHEBANG_MAP[interp.toLowerCase()];
        if (SHEBANG_MAP[interpLower]) return SHEBANG_MAP[interpLower];
        // env-style: #!/usr/bin/env python3
        if (parts.length > 1) {
            const envInterp = parts[1].toLowerCase().replace(/\d+$/, "");
            if (SHEBANG_MAP[parts[1].toLowerCase()]) return SHEBANG_MAP[parts[1].toLowerCase()];
            if (SHEBANG_MAP[envInterp]) return SHEBANG_MAP[envInterp];
        }
    }

    return "text";
}
