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
 * process.platform: "win32" | "darwin" | "linux"
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
 * Fails a production build outright when VITE_MUXBUS_CLIENT_ID resolves
 * empty. The MuxBus Cloud UI (HostPopover / AgentMuxConnectPanel /
 * accounts-manager) is gated on `isConfigured()` — client ID baked in at
 * compile time — so a build that silently loses the env var ships with the
 * entire MuxBus section invisibly absent, indistinguishable from a good
 * build until someone notices missing UI. That exact failure shipped in the
 * 2026-08-05 portable (frontend/.env.production was present and correct,
 * but that one build run didn't load it — cause transient, never
 * reproduced). The env file is committed, so a correctly-functioning build
 * can never trip this; if it fires, the env pipeline itself is broken and
 * the build MUST not ship.
 *
 * Production mode only: `--mode dev` builds and the dev server are never
 * blocked (dev loads .env.development live, and a dev loop shouldn't die
 * over cloud-login config).
 */
function requireMuxBusClientId(): Plugin {
    return {
        name: "require-muxbus-client-id",
        apply: "build",
        configResolved(config) {
            if (config.mode !== "production") return;
            if (!config.env.VITE_MUXBUS_CLIENT_ID) {
                throw new Error(
                    "[require-muxbus-client-id] VITE_MUXBUS_CLIENT_ID is empty in a production build. " +
                        "It should have been loaded from frontend/.env.production (committed). " +
                        "An empty ID compiles isConfigured() to false and silently hides all MuxBus Cloud UI — " +
                        "refusing to build. Check envDir resolution and that nothing in the build " +
                        "environment shadows VITE_MUXBUS_CLIENT_ID with an empty value.",
                );
            }
        },
    };
}

/**
 * Strips redundant KaTeX font formats (TTF, WOFF) from the build output.
 * KaTeX CSS lists woff2/woff/ttf as @font-face fallbacks; CEF's bundled
 * Chromium only needs woff2, so the others are dead weight (~876 KB).
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
        // Always emit `.map` files so the runtime source-map resolver
        // (frontend/log/source-map-resolver.ts) can rewrite raw
        // `error.stack` positions into original-file frames before
        // piping to the host log. Adds ~30MB to a portable but the
        // pay-off — readable stacks for every crash without needing
        // DevTools — is worth it for an internal tool. Spec:
        // SPEC_FE_SOURCE_MAP_RESOLVER_2026_05_27.md §7.1.
        sourcemap: true,
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
        // Default 5173 preserves single-clone behavior. Set
        // AGENTMUX_VITE_PORT to run a second `task dev` from a parallel
        // clone — the Taskfile derives a per-clone port automatically
        // from the clone's workspace-root hash, so both clones can run
        // simultaneously without colliding. strictPort still fails fast
        // if the chosen port is taken (TOCTOU guard, see Taskfile).
        // Companion to RuntimeMode::Dev clone_id (PR #1053).
        port: Number(process.env.AGENTMUX_VITE_PORT) || 5173,
        strictPort: true,
        open: false,
        watch: {
            // target/** — the Rust/Cargo build output, including the CEF
            // binary distribution's extraction. `task dev` runs this watcher
            // concurrently with the Rust/CEF host build, and without this
            // exclusion the dev-server's own file watcher is one more
            // process touching the same files `download-cef`'s extraction
            // (or its repair-cef-extract.sh fallback) is racing to move into
            // place — see docs/retro/RETRO_CEF_EXTRACT_PARTIAL_REPAIR_GAP_AND_VITE_WATCHER_2026_08_05.md
            // and the original docs/retro/RETRO_CEF_BUILD_RACE_2026_04_24.md.
            // No functional loss: nothing under target/ is ever meant to
            // trigger a frontend HMR reload.
            //
            // ANCHORED TO THE REPO ROOT, ABSOLUTE, FORWARD-SLASHED — not the
            // relative "dist/**" form this originally used. Chokidar matches
            // ignore globs against ABSOLUTE paths, so a relative pattern
            // matches nothing, silently. Worse, even "**/dist/**" fails when
            // the repo lives under a dot-directory (every agent checkout is
            // under ~/.agentmux/...): picomatch's `**` refuses to traverse
            // dot-segments like ".agentmux" without dot:true, which chokidar
            // doesn't set. Empirically verified: with the relative patterns,
            // a `task dev` Vite process on such a checkout accumulated 75K+
            // Windows handles in ~1h (caught by mem_attribution's new
            // handle-count anomaly WARN, 2026-08-09) because it was watching
            // dist/cef-dev's entire extracted CEF runtime — meaning the
            // #2424 race fix above was also silently ineffective there. An
            // absolute pattern spells the dot-directories out literally, so
            // `**` never has to traverse them.
            ignored: [
                `${path.resolve(__dirname).replace(/\\/g, "/")}/dist/**`,
                `${path.resolve(__dirname).replace(/\\/g, "/")}/target/**`,
                `${path.resolve(__dirname).replace(/\\/g, "/")}/**/*.md`,
                `${path.resolve(__dirname).replace(/\\/g, "/")}/**/*.json`,
            ],
        },
    },
    css: {
        preprocessorOptions: {
            scss: {},
        },
    },
    plugins: [
        requireMuxBusClientId(),
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

    envDir: path.resolve(__dirname, "frontend"),
    envPrefix: ["VITE_"],
});
