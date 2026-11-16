// A tiny static server for the demo pages: 127.0.0.1:8200 only, files from /app/demo-pages, nothing else.
// The pages report what they receive (a click, a key, text, a scroll, a resize) to /e, and /log lists the last events as JSON,
// so a scripted pass can check each step against what the page itself saw.
const http = require("node:http");
const fs = require("node:fs");
const path = require("node:path");
const root = "/app/demo-pages";
const events = [];
http
    .createServer((req, res) => {
        const url = new URL(req.url, "http://x");
        if (url.pathname === "/e") {
            events.push({ k: url.searchParams.get("k"), v: url.searchParams.get("v"), t: Date.now() });
            if (events.length > 500) events.shift();
            return res.writeHead(204).end();
        }
        if (url.pathname === "/log") return res.writeHead(200, { "content-type": "application/json" }).end(JSON.stringify(events));
        const name = path.basename(url.pathname) || "index.html";
        const file = path.join(root, name);
        fs.readFile(file, (err, data) => {
            if (err || !file.startsWith(root + path.sep)) return res.writeHead(404).end("not found");
            const type = name.endsWith(".js") ? "text/javascript" : "text/html; charset=utf-8";
            res.writeHead(200, { "content-type": type }).end(data);
        });
    })
    .listen(8200, "127.0.0.1");
