// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Vite configuration for AgentMux frontend.
// Builds the SolidJS frontend for both dev mode (Vite HMR) and
// production (bundled into the CEF portable package).

import tailwindcss from "@tailwindcss/vite";
import * as fs from "fs";
import * as path from "path";
import solid from "vite-plugin-solid";
import { defineConfig, type Plugin } from "vite";
import svgr from "vite-plugin-svgr";
import tsconfigPaths from "vite-tsconfig-paths";

/**
 * Maps Taskfile {{OS}} values to Node.js process.platform equivalents.
 * Taskfile: "windows" | "darwin" | "linux"
 * Node/Tauri: "win32" | "darwin" | "linux"
 */
const TASKFILE_OS_MAP: Record<string, string> = {
    windows: "win32",
    darwin: "darwin",
    linux: "linux",
};

/**
 * Returns the target platform for the build. Checks VITE_PLATFORM first (set
 * by Taskfile), falls back to the current OS via process.platform.
 */
function getTargetPlatform(): string {
    const env = process.env.VITE_PLATFORM;
    if (env) {
        return TASKFILE_OS_MAP[env] ?? env;
    }
    return process.platform;
}

/**
 * Vite plugin that resolves `.platform.{ts,tsx,scss,css}` imports to the
 * platform-specific file at build time.
 *
 * Example: `import "./foo.platform.scss"` resolves to `./foo.win32.scss`
 * when building for Windows.
 *
 * Files must exist as `foo.win32.ts`, `foo.darwin.ts`, `foo.linux.ts`.
 * If the platform file does not exist, the original import is left unchanged
 * (Vite will error naturally).
 */
function platformResolve(): Plugin {
    const platform = getTargetPlatform();
    console.log(`[platformResolve] Target platform: ${platform}`);
    return {
        name: "platform-resolve",
        enforce: "pre",
        resolveId(source, importer) {
            if (!source.includes(".platform")) return null;
            const resolved = source.replace(/\.platform(\.(ts|tsx|scss|css))?$/, (_, ext) => {
                return ext ? `.${platform}${ext}` : `.${platform}`;
            });
            if (resolved === source) return null;
            return this.resolve(resolved, importer, { skipSelf: true });
        },
    };
}

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
        cssCodeSplit: false,
        outDir: "dist/frontend",
        rollupOptions: {
            input: {
                index: "index.html",
            },
            output: {
                // DISABLED: manualChunks creates static inter-chunk imports that
                // caused loading issues in the old WebKitGTK host.
                // All code goes in one bundle. Dynamic imports (mermaid, katex, shiki) are
                // still lazy-loaded but as inlined chunks, not separate files.
            },
        },
    },
    server: {
        port: 5173,
        strictPort: true, // Fail if port 5173 is already in use (required for Tauri)
        open: false,
        watch: {
            ignored: ["dist/**", "**/*.md", "**/*.json"],
        },
    },
    css: {
        preprocessorOptions: {
            scss: {},
        },
    },
    plugins: [
        platformResolve(),
        tsconfigPaths(),
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
