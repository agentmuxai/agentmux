// A tiny static server for the demo pages: 127.0.0.1:8200 only, files from /app/demo-pages, nothing else.
const http = require("node:http");
const fs = require("node:fs");
const path = require("node:path");
const root = "/app/demo-pages";
http
    .createServer((req, res) => {
        const name = path.basename(new URL(req.url, "http://x").pathname) || "index.html";
        const file = path.join(root, name);
        fs.readFile(file, (err, data) => {
            if (err || !file.startsWith(root + path.sep)) return res.writeHead(404).end("not found");
            res.writeHead(200, { "content-type": "text/html; charset=utf-8" }).end(data);
        });
    })
    .listen(8200, "127.0.0.1");
