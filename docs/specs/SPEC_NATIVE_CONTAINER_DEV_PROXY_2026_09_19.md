# SPEC: a native reverse proxy in agentmux-srv for friendly dev-server hostnames — not Traefik, not a sidecar

**Date:** 2026-09-19
**Status:** implemented in #3439 — see "Implementation notes (2026-09-19)" at
the end of this document for what shipped, verification depth, and where the
actual implementation diverged (or didn't) from what's proposed below.
**Author:** Camper
**Repo state:** main @ `bdd3270aa` (v0.56.8+)
**Related:** `docs/reports/REPORT_HOST_AGENT_SANDBOX_RESTART_CONTROL_GAP_2026_09_19.md`
(the container-lifecycle investigation this grew out of), `a5af/claw`'s
`specs/DOCKER_NETWORK_ALIASES.md` (the precedent this replaces — archived,
independent of `agentmux` by `container.rs`'s own doc comment), issue #2939
(container agents generally)

---

## 0. TL;DR

**Build a small reverse proxy directly into `agentmux-srv`, not by compiling
or bundling Traefik.** Traefik is Go — there is no practical way to compile
it *into* a Rust binary, only to ship and manage it as a second process, the
way DDEV/Lando do. `agentmux-srv` already depends on `axum`, `hyper` (via
axum), `tower`, `tower-http`, and `bollard` (`agentmux-srv/Cargo.toml`) — a
label-polling sidecar proxy would be solving a discovery problem AgentMux
doesn't have, since it is the process that calls `docker run` for every
container in the first place and already holds that state in memory. Route
via `<project>-<agent>.localhost:<proxy-port>` — `*.localhost` resolves to
`127.0.0.1` in every evergreen browser with zero DNS/hosts configuration,
per RFC 6761 (§2). HTTP only; no TLS/ACME (§4) — the single most commonly
cited source of real-world Traefik pain (§1) simply doesn't apply to pure
loopback traffic.

## 1. What the prior art actually does, and why none of it fits as-is

Researched before deciding, not assumed:

| Tool | Approach | Relevant to us? |
|---|---|---|
| **DDEV / Lando** | Traefik runs as a global sidecar container listening on 80/443. Project containers self-register via Docker labels (`traefik.http.routers.*`). Wildcard DNS (`*.ddev.site`/`*.lndo.site`) → `127.0.0.1`. | Proves the pattern works, but both are Go/PHP-hosting-shaped tools managing many *independent* project stacks — they need Traefik's label-discovery because nothing else in their architecture already knows what's running. We're not in that position (§0). |
| **OrbStack** | Native domain-per-container convention built into the runtime itself, no separate reverse-proxy container shipped. | Closest architectural match to what AgentMux should do: the app that owns the container lifecycle also owns the routing, in-process. |
| **caddy-docker-proxy / caddy-dev-local** | Caddy + Docker-label discovery, terser config than Traefik, automatic on-demand TLS. | Same category as Traefik for our purposes — still a second process, still label-polling, still solving a discovery problem we don't have. Caddy's on-demand TLS is a real strength but irrelevant to loopback-only traffic (§4). |
| **Raw Traefik** | As above, industry-standard for exactly this (host-based routing to dynamic containers). | Real-world friction is well documented, not hypothetical: ACME/TLS automation causing "numerous problems," "documentation... all over," breaking changes across major versions, and label typos silently breaking routing with no compile-time check. None of that risk is worth taking on for a single-tenant, loopback-only, one-proxy-port use case. |

**Rust-native alternatives considered and rejected as overkill:** Cloudflare's
`pingora` (via `pingap`) and `sozu` are proven, production-grade dynamic
reverse proxies with hot-reload, service discovery, TLS termination, and
metrics — built for infrastructure serving real internet traffic at scale.
Pulling either in would mean learning and operating a second framework's
config/extension model for a routing table that, in practice, is a
`HashMap<String, SocketAddr>` guarded by a mutex, updated by code that
already runs in the same process as the Docker client.

## 2. Hostname scheme: `.localhost`, not `.local` or `.test`

Confirmed via current documentation (Microsoft Learn's ASP.NET Core docs on
`.localhost` TLD support, RFC 2606, RFC 6761): Chrome, Edge, and Firefox all
resolve **any** `*.localhost` name to `127.0.0.1`/`::1` with **no hosts-file
edit and no DNS configuration** — it's reserved specifically for this by
RFC 6761, the same RFC that reserves plain `localhost`.

This is a real improvement over what claw did, not just a rename:

- Claw's `.test` origins required its own `dns` service
  (`jpillora/dnsmasq`) as a fourth always-running container just to resolve
  `*.test` inside the Docker network (`claw/docker/docker-compose.yml`
  lines 74–83). `.localhost` needs nothing — the browser handles it.
- `.local` (which is what the user remembered, and what claw's
  `DOCKER_NETWORK_ALIASES.md` used for *inter-container* resolution) is
  reserved by RFC 6762 for mDNS and can conflict with a real mDNS resolver
  on the host (relevant here specifically: AgentMux's own LAN discovery
  already uses mDNS for cross-host agent discovery — see the Roles Anywhere
  migration spec's `starpower` resolution story). `.localhost` has no such
  collision risk.

**Caveat, stated plainly:** this guarantee is browser-level, not universal.
A non-browser HTTP client (`curl`, a test runner, `playwright`'s own browser
contexts should be fine since they're real browsers, but a raw script
isn't) will not automatically resolve `*.localhost` unless its underlying
resolver also special-cases it (most modern OS resolvers now do, per the
same RFC, but this should be verified per-platform before relying on it in,
e.g., CI). Where that matters, a hosts-file entry remains the fallback —
same fallback claw would have needed for `.test` too, just less often
needed here.

## 3. What's actually missing before routing can work at all

Two gaps, found while investigating this, not assumed:

### 3.1 Containers aren't on a shared, addressable network today

`agentmux-srv/src/backend/container.rs` currently sets
`network_mode: Some("bridge".to_string())` (line 1037) — Docker's default
bridge, not a user-defined one. Containers on the default bridge get no
automatic DNS resolution by name and no alias support; only a user-defined
network provides that (this is standard Docker behavior, not specific to
AgentMux). Claw's `docker-compose.yml` used exactly this distinction —
its `agent-network` was user-defined, with `aliases:` per service.

**Needed:** create (idempotently, at srv startup) a user-defined bridge
network — `bollard`, already a dependency, exposes `create_network` and
`connect_network` — and attach every agent container to it at creation
time. This is a prerequisite for *any* proxy design here, native or
sidecar; it isn't specific to the native-proxy decision in §0.

### 3.2 Routing needs to be dynamic — claw's was hand-authored

Claw's Traefik labels were static, one block per fixed agent (`agent1`
through `agent5`) times one block per known project (`pulse`, `stratum`,
`app`, …) — a human edited `docker-compose.yml` by hand to add a project.
AgentMux's agents are not a fixed roster: names, projects, and even
whether an agent is container-type at all are all dynamic, created and
destroyed on demand (this is the same point `SPEC_IAM_ROLES_ANYWHERE_MIGRATION_2026_09_18.md`
§2 already had to grapple with for certificate issuance — "identities
aren't fixed to hosts," same shape of problem, different resource).

**Needed:** a registration path, not a config file. A project's dev server
running inside a container needs to tell `agentmux-srv` "I'm listening on
port N," and that mapping needs to disappear again when the container
stops. Options, roughly in order of how much they ask of the agent running
inside the container:

1. **Convention + polling.** srv periodically inspects each container's
   exposed/listening ports (already has a Docker client) and guesses at a
   project name from, e.g., a `package.json`/`Taskfile.yml` it can read via
   the same bind mount used for `/workspace`. Zero agent cooperation
   required, but "guessing" is doing a lot of work in that sentence — this
   is the least reliable option and should be the fallback, not the
   primary mechanism.
2. **Explicit registration call.** A new App API verb / MCP tool (e.g.
   `RegisterDevServer { project: string, port: u16 }`) an agent calls once
   its dev server is up. Matches the existing pattern of agent-facing App
   API verbs already documented in
   `docs/reports/REPORT_AGENT_OPEN_API_GAP_2026_09_06.md` and this
   session's own restart-control-gap report — both already establish that
   agent-facing control surface as the right place for this kind of thing.
   Requires the agent (or a convention baked into `CLAUDE.md`/a skill) to
   actually call it.
3. **A well-known env var the agent's dev server reads at startup**
   (e.g. `AGENTMUX_DEV_PORT_HINT`), with the *tooling* (whatever starts
   `npm run dev`, `task dev`, etc.) responsible for calling back with the
   real bound port once known — closer to how Node/Vite dev servers often
   already print "Local: http://localhost:PORT" and could be scraped, but
   scraping stdout for a URL is fragile compared to (2).

**Recommendation: (2), with (1) as an unregistered fallback that shows
"no dev server registered yet" rather than a broken proxy response** —
explicit is more debuggable than inferred, and this is squarely in the
class of capability the two prior reports already flagged AgentMux's
agent-facing API as needing more of, not less.

**Resolved (2026-09-20): implement (2) as an MCP tool, not a CLI.** The
concern that motivated considering a CLI instead — that every MCP tool's
schema sits in context for the whole session whether or not it's called —
doesn't apply here: this environment already defers most `agentmux`-family
MCP tool schemas behind `ToolSearch` (observed directly, not assumed —
most `mcp__agentmux__*` tools arrive in the system prompt as name-only
until explicitly searched for), so a new `RegisterDevServer` tool costs
essentially nothing in context unless an agent actually calls it. That
removes the token-cost argument for a CLI and leaves only MCP's advantage:
an explicit schema the model can't get the invocation syntax wrong on,
versus a CLI's flag-guessing/retry risk. Build it as a normal MCP tool.

## 4. Explicitly out of scope

- **TLS/ACME.** Pure loopback HTTP is sufficient — nothing here ever
  leaves `127.0.0.1`. This sidesteps the single most commonly cited source
  of real-world Traefik operational pain (§1) entirely, not by solving it
  but by not having the problem.
- **A general-purpose rule/middleware engine.** No path-based routing, no
  header rewriting, no rate limiting, no auth. Host-header → backend
  address is the entire feature. If a real need for more shows up later,
  that's a new spec, not a reason to reach for Traefik's middleware system
  now.
- **Multi-host / remote access.** Scoped to one local AgentMux instance
  proxying to its own containers, same boundary as everything else in
  `container.rs`.
- **Non-container (host-type) agents.** They already run directly on the
  host; a dev server they start is already reachable at `localhost:PORT`
  with no proxy needed. This spec is container-agent-specific.

## 5. Open questions requiring a decision before implementation

1. ~~Where does the proxy listen, and is one port enough?~~ — **resolved**:
   `:8090`, a single fixed port, Host-header routing to every project
   across every agent. Confirmed free in this codebase before use (see
   "Implementation notes" below).
2. ~~Registration mechanism (§3.2)~~ — **resolved**: option 2, implemented
   as an MCP tool (`RegisterDevServer`). See §3.2's resolution note for why
   the token-cost objection to MCP doesn't hold in this environment.
3. ~~Does the proxy need to survive `agentmux-srv` restarts independent of
   any single agent's container?~~ — **resolved, as anticipated**: the
   routing table is a plain in-memory `HashMap` with no persistence: an
   `agentmux-srv` restart (or a container recreate on drift) loses every
   registration for that agent, by design, matching
   `REPORT_HOST_AGENT_SANDBOX_RESTART_CONTROL_GAP_2026_09_19.md`'s finding
   that the container itself doesn't survive either. An agent must call
   `RegisterDevServer` again after either kind of restart — not automated
   in this PR (see "Implementation notes" for why).
4. **Naming collisions.** Two different agents both running a project
   named `pulse` would collide on `pulse-<agent>.localhost` only if agent
   names themselves collide — which `AGENTMUX_AGENT_ID` is already assumed
   unique for elsewhere in this codebase (jekt trust, container naming).
   Worth stating as a stated assumption here too, not a new one. Unaffected
   by implementation — still just a stated assumption, not newly verified.

## 6. Non-claim

This spec does not claim the native approach is *always* superior to a
sidecar proxy — it's the better fit for AgentMux's specific shape (single
process already owns full container lifecycle state, single local tenant,
no TLS requirement, small fixed feature surface). A tool built the way
DDEV or Lando are — many independent, loosely-coupled project stacks with
no single process that already knows about all of them — would reasonably
make the opposite call.

## Implementation notes (2026-09-19)

Shipped as proposed, with the following concrete choices and one deviation:

- **Network name:** `agentmux-agents` (§3.1) — `AGENTMUX_DOCKER_NETWORK` in
  `agentmux-srv/src/backend/container.rs`. Created idempotently (tolerates
  a 409/"already exists" from `bollard::Docker::create_network`) and every
  container is attached to it via `connect_network` on every
  `ensure_running`, not just at creation — additive to the existing
  `network_mode: "bridge"` / `extra_hosts: host.docker.internal:host-gateway`
  setup, which is untouched.
- **Proxy port:** `:8090` (§5.1) — `DEV_PROXY_PORT` in the new
  `agentmux-srv/src/backend/dev_proxy.rs`. Confirmed free before use (no
  other `TcpListener::bind`/config default in this crate claims it).
- **Registration route:** `POST /api/v1/agent/dev_server/register`,
  backing the `RegisterDevServer` MCP tool
  (`agentmux-mcp/src/tool_schemas.rs` + `main.rs`). Identity is verified
  via the same `verified_block_id`/`UiAutomationAuth` mechanism
  `ClosePane`/`UIClick`/`UIQuery` already use — no client-supplied agent id
  or backend address; the container's dev-proxy-network IP is resolved
  server-side from `ContainerManager::agent_network_ip`, keyed off the
  CALLER's own verified identity.
- **Cleanup on container stop:** wired into `ContainerManager::stop`/
  `remove` themselves (§3.2's "needed" list didn't specify which), via a
  registry reference (`ContainerRuntimeHandle::set_dev_proxy_registry`,
  re-attached on every Docker reconnect since a reconnect creates a fresh
  `ContainerManager`) rather than a separate close-pane-saga hook — this
  keeps cleanup correct for the drift-recreate path inside
  `ensure_running` too, not just an explicit future stop/restart feature.
- **One deviation from "streamed... hyper's client, the natural tool"
  (§0/proxy handler description):** the proxy forwards requests via
  `reqwest` (already a workspace dependency, given the `stream` feature)
  rather than a raw `hyper` client — same streaming behavior (request and
  response bodies are streamed via `Body::from_stream`/`wrap_stream`, never
  buffered whole), less code than hand-rolling connection pooling and
  header plumbing on top of bare `hyper`. No functional difference for
  this spec's HTTP-only, no-websocket-upgrade scope.

**Verification depth:** `cargo build`/`cargo test` pass across
`agentmux-common`, `agentmux-srv`, and `agentmux-mcp` (4048 srv tests, 215
common, 26 mcp — zero regressions). Beyond that:
- `backend::dev_proxy::tests` includes a real end-to-end test: an actual
  bound TCP listener standing in for a dev server, the real proxy router in
  front of it, and a real `reqwest` client request carrying a `Host`
  header — confirms the full path (Host parsing → lookup → forward →
  stream response back) and that an unregistered Host 404s cleanly.
- `backend::container::tests::itest_dev_proxy_network_attach_and_ip_resolution`
  (Docker-gated, `#[ignore]` by default, run manually against this
  session's real Docker Desktop daemon) confirms against REAL Docker: the
  shared network is created, a real container is attached to it,
  `agent_network_ip` resolves a real routable IPv4 address, and `remove`
  clears that agent's registrations from a live registry.
- `server::tests::register_dev_server_*` (4 tests) exercise the actual
  HTTP route (`POST /api/v1/agent/dev_server/register`) through
  `build_router` + `tower::oneshot`: unsigned request → 401, empty
  project / zero port → 400 (before Docker is even consulted), and
  Docker-unavailable → 503, all against the real handler code, not a
  mock of it.
- **What did NOT run:** a full click-through-the-actual-MCP-stdio-tool
  call (`agentmux-mcp`'s `RegisterDevServer` invoking the live route over
  a real `AGENTMUX_LOCAL_URL`/`AGENTMUX_AUTH_KEY`/`AGENTMUX_JEKT_KEY`
  triple from inside a real container agent's own process) — driving that
  requires a full agent spawn this pass didn't attempt. The HTTP route
  itself, the Docker networking, and the MCP tool's JSON-RPC
  request/response shape were each verified independently instead (above);
  the seam between "agentmux-mcp sends this exact request" and "the route
  accepts it" is covered by matching Rust types
  (`RegisterDevServerRequest`/`Response` in `agentmux-common`) shared by
  both sides, not by an observed live call.
