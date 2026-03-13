// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Vite configuration for Tauri development mode.
// This replaces electron-vite for the Tauri build.
// Only the renderer (frontend) configuration is needed —
// Tauri handles the "main process" in Rust.

import tailwindcss from "@tailwindcss/vite";
import * as fs from "fs";
import * as path from "path";
import solid from "vite-plugin-solid";
import { defineConfig, type Plugin } from "vite";
import { ViteImageOptimizer } from "vite-plugin-image-optimizer";
import svgr from "vite-plugin-svgr";
import tsconfigPaths from "vite-tsconfig-paths";

/**
 * Strips redundant KaTeX font formats (TTF, WOFF) from the build output.
 * KaTeX CSS lists woff2/woff/ttf as @font-face fallbacks; Tauri's Chromium
 * webview only needs woff2, so the others are dead weight (~876 KB).
 */
function stripKatexLegacyFonts(): Plugin {
    return {
        name: "strip-katex-legacy-fonts",
        apply: "build",
        closeBundle() {
            const assetsDir = path.resolve(__dirname, "dist/frontend/assets");
            if (!fs.existsSync(assetsDir)) return;
            const files = fs.readdirSync(assetsDir);
            let removed = 0;
            for (const file of files) {
                if (/^KaTeX_.*\.(ttf|woff)$/i.test(file) && !file.endsWith(".woff2")) {
                    fs.unlinkSync(path.join(assetsDir, file));
                    removed++;
                }
            }
            if (removed > 0) {
                console.log(`[strip-katex-legacy-fonts] Removed ${removed} redundant KaTeX font files (TTF/WOFF)`);
            }
        },
    };
}

export default defineConfig({
    root: ".",
    build: {
        target: ["es2021", "chrome97", "safari13"],
        sourcemap: process.env.NODE_ENV === "development",
        outDir: "dist/frontend",
        rollupOptions: {
            input: {
                index: "index.html",
            },
            output: {
                manualChunks(id) {
                    const p = id.replace(/\\/g, "/");
                    if (p.includes("node_modules/mermaid") || p.includes("node_modules/@mermaid")) return "mermaid";
                    if (p.includes("node_modules/katex") || p.includes("node_modules/@katex")) return "katex";
                    if (p.includes("node_modules/shiki") || p.includes("node_modules/@shiki")) return "shiki";
                    return undefined;
                },
            },
        },
    },
    server: {
        port: 5173,
        strictPort: true, // Fail if port 5173 is already in use (required for Tauri)
        open: false,
        watch: {
            ignored: ["dist/**", "**/*.md", "**/*.json", "src-tauri/**"],
        },
    },
    css: {
        preprocessorOptions: {
            scss: {},
        },
    },
    plugins: [
        tsconfigPaths(),
        { ...ViteImageOptimizer(), apply: "build" },
        svgr({
            svgrOptions: { exportType: "default", ref: true, svgo: false, titleProp: true },
            include: "**/*.svg",
        }),
        solid(),
        tailwindcss(),
        stripKatexLegacyFonts(),
    ],

    // Environment variable prefix for Tauri
    envPrefix: ["VITE_", "TAURI_"],
});
