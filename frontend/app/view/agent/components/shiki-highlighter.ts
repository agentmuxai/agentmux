// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Shiki's web bundle plus the grammars it leaves out that agents read and edit
 * often: Rust, Go, TOML, PowerShell, Dockerfile and so on. Without them those
 * files render as plain text in the Read, Write and Edit previews.
 *
 * Each grammar is its own chunk, fetched the first time a file in that
 * language is highlighted, so nothing here costs anything at startup. All 24
 * together are about 600 KB raw, 84 KB gzipped
 * (docs/analysis/ANALYSIS_READ_TOOL_PREVIEW_2026_10_01.md §5).
 *
 * Import this module dynamically: it pulls in Shiki itself.
 */

import type { DynamicImportLanguageRegistration } from "shiki/core";
import {
    bundledLanguages as webLanguages,
    createBundledHighlighter,
    createSingletonShorthands,
    guessEmbeddedLanguages,
} from "shiki/bundle/web";
import { createOnigurumaEngine } from "shiki/engine/oniguruma";

/** Grammar id, the extra names it answers to, and its loader. */
const EXTRA: [string, string[], DynamicImportLanguageRegistration][] = [
    ["rust", ["rs"], () => import("shiki/langs/rust.mjs")],
    ["go", [], () => import("shiki/langs/go.mjs")],
    ["toml", [], () => import("shiki/langs/toml.mjs")],
    ["powershell", ["ps", "ps1"], () => import("shiki/langs/powershell.mjs")],
    ["docker", ["dockerfile"], () => import("shiki/langs/docker.mjs")],
    ["make", ["makefile"], () => import("shiki/langs/make.mjs")],
    ["ini", ["properties"], () => import("shiki/langs/ini.mjs")],
    ["dotenv", [], () => import("shiki/langs/dotenv.mjs")],
    ["fish", [], () => import("shiki/langs/fish.mjs")],
    ["kotlin", ["kt", "kts"], () => import("shiki/langs/kotlin.mjs")],
    ["swift", [], () => import("shiki/langs/swift.mjs")],
    ["csharp", ["c#", "cs"], () => import("shiki/langs/csharp.mjs")],
    ["lua", [], () => import("shiki/langs/lua.mjs")],
    ["ruby", ["rb"], () => import("shiki/langs/ruby.mjs")],
    ["terraform", ["tf", "tfvars"], () => import("shiki/langs/terraform.mjs")],
    ["hcl", [], () => import("shiki/langs/hcl.mjs")],
    ["dart", [], () => import("shiki/langs/dart.mjs")],
    ["elixir", [], () => import("shiki/langs/elixir.mjs")],
    ["elm", [], () => import("shiki/langs/elm.mjs")],
    ["haskell", ["hs"], () => import("shiki/langs/haskell.mjs")],
    ["clojure", ["clj"], () => import("shiki/langs/clojure.mjs")],
    ["scala", [], () => import("shiki/langs/scala.mjs")],
    ["groovy", [], () => import("shiki/langs/groovy.mjs")],
    ["perl", [], () => import("shiki/langs/perl.mjs")],
];

export const extraLanguages: Record<string, DynamicImportLanguageRegistration> = Object.fromEntries(
    EXTRA.flatMap(([id, aliases, load]) => [id, ...aliases].map((name) => [name, load]))
);

/**
 * Only the themes something renders with: HighlightedCode and the preview
 * text use the dark one, and the shell highlighter colours both so a theme
 * flip needs no re-highlight (embedded-highlight.ts). Registering Shiki's
 * `bundledThemes` instead shipped all ~65 as chunks, about 1.3 MB of JS for
 * two of them (#4207 F1). A new theme name must be added here, or Shiki
 * rejects it at highlight time.
 */
const themes = {
    "github-dark-high-contrast": () => import("shiki/themes/github-dark-high-contrast.mjs"),
    "github-light-high-contrast": () => import("shiki/themes/github-light-high-contrast.mjs"),
};

const createHighlighter = createBundledHighlighter({
    langs: { ...webLanguages, ...extraLanguages },
    themes,
    engine: () => createOnigurumaEngine(import("shiki/wasm")),
});

export const { codeToHtml, codeToTokens } = createSingletonShorthands(createHighlighter, { guessEmbeddedLanguages });
