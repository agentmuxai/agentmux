#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Builds the frontend for a browser tab: the same app as the desktop bundle,
// with the browser host (frontend/web-host) chosen as its host module through
// AGENTMUX_HOST_MODULE (docs/specs/SPEC_EXTERNAL_HOST_BUILD_2026_10_09.md).
// The desktop bundle (dist/frontend) is not touched.
//
//   node scripts/build-web.mjs [--out <dir>]     default: dist/web
//
// Serve the result from a headless srv (`agentmux-srv --headless --frontend-dir <dir>`)
// behind a proxy on the page's own origin that adds srv's key.

import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const outFlag = process.argv.indexOf("--out");
const outDir = path.resolve(root, outFlag > 0 ? process.argv[outFlag + 1] : "dist/web");

// Vite's own entry run with this node: no shell, so the same on every platform.
const vite = path.join(root, "node_modules", "vite", "bin", "vite.js");
const result = spawnSync(process.execPath, [vite, "build", "--mode", "production", "--config", "vite.config.ts", "--outDir", outDir, "--emptyOutDir"], {
    cwd: root,
    stdio: "inherit",
    env: {
        ...process.env,
        // The bundle needs more heap than node's default.
        NODE_OPTIONS: process.env.NODE_OPTIONS ?? "--max-old-space-size=6144",
        AGENTMUX_HOST_MODULE: path.join(root, "frontend", "web-host", "web-host-module.ts"),
        // The production build requires a MuxBus client id; the browser host has no MuxBus sign-in yet.
        VITE_MUXBUS_CLIENT_ID: process.env.VITE_MUXBUS_CLIENT_ID ?? "agentmux-web-unset",
    },
});
if (result.status !== 0) process.exit(result.status ?? 1);
console.log(`[build-web] built ${outDir}`);
