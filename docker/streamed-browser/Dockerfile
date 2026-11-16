# The streamed Browser pane's probe, in a container with no screen, mouse or keyboard.
# docs/specs/SPEC_STREAMED_BROWSER_PANE_DEMO_2026_11_15.md §9.
#
# trixie, because a srv built on a current Linux host needs its glibc. Build context is a folder holding: agentmux-srv (a Linux build), probe-entry.sh, streamed-browser-probe.mjs and
# node_modules/ws (the `ws` package: `npm pack ws`, unpacked). Run with no published port:
#   docker run --rm --cpus 1 agentmux-streamed-probe
# `--no-sandbox` is for this probe and the demo only (spec §2); the container is the boundary.
FROM debian:trixie-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends chromium nodejs ca-certificates fonts-liberation \
 && rm -rf /var/lib/apt/lists/* \
 && useradd -m amx
COPY agentmux-srv /usr/local/bin/agentmux-srv
COPY probe-entry.sh /app/probe-entry.sh
COPY streamed-browser-probe.mjs /app/probe.mjs
COPY node_modules /app/node_modules
RUN chmod -R a+rX /app && chmod 755 /usr/local/bin/agentmux-srv /app/probe-entry.sh
USER amx
ENV AGENTMUX_STREAMED_BROWSER=1 \
    AGENTMUX_STREAMED_BROWSER_NO_SANDBOX=1 \
    AGENTMUX_STREAMED_BROWSER_BIN=/usr/bin/chromium
ENTRYPOINT ["/app/probe-entry.sh"]
