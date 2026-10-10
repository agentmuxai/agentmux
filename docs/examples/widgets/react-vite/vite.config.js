// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Builds the widget package into dist/: index.html, its bundle, and
// widget.json (copied from public/). Install dist/widget.json in
// Settings → Widgets. `npm run dev` rebuilds on every change; install
// dist/widget.json again (Replace) to try a new build.
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

export default defineConfig({
    plugins: [react()],
    // Relative URLs: the package is served from its own folder.
    base: "./",
    build: {
        outDir: "dist",
        emptyOutDir: true,
        // AgentMux serves the SDK itself; leave the import as it is.
        rollupOptions: { external: ["/agentmux/widget-sdk/v1.js"] },
    },
});
