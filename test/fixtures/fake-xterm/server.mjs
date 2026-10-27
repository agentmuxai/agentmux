// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Serves a page with real xterm.js (screen-reader mode) and a tiny shell that
// records what ran. See README.md.

import fs from "node:fs";
import http from "node:http";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const xterm = path.resolve(here, "../../../node_modules/@xterm/xterm");
const port = Number(process.argv[2] || 8765);

const assets = {
    "/": ["index.html", "text/html"],
    "/index.html": ["index.html", "text/html"],
};
const vendored = {
    "/xterm.js": [path.join(xterm, "lib/xterm.js"), "text/javascript"],
    "/xterm.css": [path.join(xterm, "css/xterm.css"), "text/css"],
};

http.createServer((req, res) => {
    const url = (req.url ?? "/").split("?")[0];
    const hit = assets[url] ? [path.join(here, assets[url][0]), assets[url][1]] : vendored[url];
    if (!hit) {
        res.writeHead(404);
        res.end();
        return;
    }
    res.writeHead(200, { "content-type": hit[1], "cache-control": "no-store" });
    fs.createReadStream(hit[0]).pipe(res);
}).listen(port, "127.0.0.1", () => console.log(`fake xterm on http://127.0.0.1:${port}/`));
