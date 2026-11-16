# The streamed Browser pane: probe and demo

For our own hosts only. Chromium runs with `--no-sandbox` (the container is the boundary), pages are ones we choose, and nothing is
published except on the host's own loopback. Spec: `docs/specs/SPEC_STREAMED_BROWSER_PANE_DEMO_2026_11_15.md`.

## The probe (no UI)

`Dockerfile` and `probe-entry.sh` run `scripts/streamed-browser-probe.mjs` against a headless srv in a container with no screen, mouse or
keyboard, and print one JSON of timings. Build context: `agentmux-srv`, `Dockerfile`, `probe-entry.sh`,
`streamed-browser-probe.mjs` and the `ws` package as `node_modules/ws`. `docker run --rm --cpus 1 <image>`.

## The demo (the pane in the web UI)

A headless srv with its own web UI, the streamed browser on, and a few demo pages (motion, a form, a long page, keys) served inside the container.

Build, on a Linux machine with Docker:

1. `cargo build -p agentmux-srv --bin agentmux-srv`
2. `node scripts/build-web.mjs --out <context>/web`
3. Put `agentmux-srv`, `docker/streamed-browser/Dockerfile.demo`, `demo-pages/`, `pages-server.js` and `demo-entry.sh` in `<context>` beside `web/`.
4. `docker build -f Dockerfile.demo -t agentmux-streamed-demo <context>`
5. `docker run -d --name streamed-demo --cpus 2 -p 127.0.0.1:18190:18190 agentmux-streamed-demo`

The one published port is on the host's loopback, so the demo is reachable only through the host (an `ssh -L` tunnel), never from the network.

Open it, from your own machine, with the same port at both ends (srv accepts only the address `127.0.0.1:<its port>`):

1. `ssh -L 8190:127.0.0.1:18190 <the host>`
2. On the host, in a terminal: `docker exec -it streamed-demo agentmux-srv open --print`. It prints a one-time sign-in line and then waits.
3. Paste the line into a browser on your machine. The page shows a short phrase; the terminal asks `[y/N]`: answer `y` only if the phrases match.
4. In the app, open a Browser pane and type `http://127.0.0.1:8200/` (the demo pages), or any address you like.

Stop it: `docker rm -f streamed-demo`.
