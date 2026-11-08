// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Serves a page of forms and controls for trying Enter and Space as the click they are.
// See README.md.

import fs from "node:fs";
import http from "node:http";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const port = Number(process.argv[2] || 8766);

http.createServer((req, res) => {
    const url = (req.url ?? "/").split("?")[0];
    if (url !== "/" && url !== "/index.html") {
        res.writeHead(404);
        res.end();
        return;
    }
    res.writeHead(200, { "content-type": "text/html", "cache-control": "no-store" });
    fs.createReadStream(path.join(here, "index.html")).pipe(res);
}).listen(port, "127.0.0.1", () => console.log(`fake form on http://127.0.0.1:${port}/`));
