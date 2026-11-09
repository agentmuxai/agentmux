// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Serves a page whose buttons open file choosers, for trying set_files with chooser: true.
// It answers on the port given and on the next one, so a page on one can embed a frame from the other.
// See README.md.

import fs from "node:fs";
import http from "node:http";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const port = Number(process.argv[2] || 8767);
const pages = { "/": "index.html", "/index.html": "index.html", "/frame.html": "frame.html" };

for (const p of [port, port + 1]) {
    http.createServer((req, res) => {
        const file = pages[(req.url ?? "/").split("?")[0]];
        if (!file) {
            res.writeHead(404);
            res.end();
            return;
        }
        res.writeHead(200, { "content-type": "text/html", "cache-control": "no-store" });
        fs.createReadStream(path.join(here, file)).pipe(res);
    }).listen(p, "127.0.0.1", () => console.log(`fake upload on http://127.0.0.1:${p}/`));
}
