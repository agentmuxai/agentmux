# Spec: Preloaded MCP Server Connectors for Creative Apps (Ableton Live, TouchDesigner, and Others)

**Date:** 2026-07-10
**Author:** AgentY
**Type:** Research + proposal. No code shipped yet.
**Purpose:** Give the Armory's **MCP Servers** tab a curated, one-click "preloaded servers" gallery for popular creative/production software — flagship cases: **Ableton Live** and **TouchDesigner** — instead of the generic name/transport/JSON-textarea form it has today. Generalizes the single-app `AppConnector` catalog schema already proposed in `docs/specs/SPEC_EXTERNAL_APP_DRIVING_BLENDER_2026_07_03.md` §4.7 into an actual multi-app catalog, and scopes what it takes to ship Ableton Live + TouchDesigner as the first two real entries.

---

## 0. Status update (2026-07-10, post-write): reconciled with Agent1's governing spec

While implementing this, discovered `docs/specs/SPEC_MCP_INTEGRATION_PARITY_ABLETON_PILOT_2026_07_08.md` (Agent1, 2026-07-08) — a broader, earlier spec whose **Phase A already shipped** (`mcp.probe`/`mcp.catalog.probe(id)` protocol-level probe + `mcp-capabilities.ts`, PR #2030) and whose **Phase B is this exact feature** (catalog/one-click-install UI), with an explicit recommendation: ship Ableton MCP alone first, expand only after the pattern validates (§7/§8 Q1).

Coordinated with Agent1 over muxbus — confirmed Phase B was open, no collision, took it. **The actual implementation diverges from §3's design below in three ways**, in favor of Agent1's simpler, already-partially-shipped shape:

1. **No new probe RPC.** §3.1's `liveCheck`/TCP-probe-by-port design was replaced entirely by the already-shipped `mcp.catalog.probe(id)` — a real MCP protocol handshake (`initialize` + `tools/list`) against a server id, materially better than a raw TCP connect (works for both stdio and http transports, confirms the process actually speaks MCP, not just "something is listening on a port"). My first draft duplicated the RPC name with an incompatible signature before this was caught in a merge-conflict review.
2. **No Tier A/B / `AppConnector` checklist UI.** Replaced by Agent1's flat `{name, transport, config, prereqNote, docsUrl}` catalog-entry shape (`mcp-preload-catalog.ts`) — a single static remediation string shown next to the live probe status, not a multi-step guided checklist with per-step manual/TCP verification.
3. **Scope cut to Ableton Live only for this PR.** TouchDesigner and ComfyUI (§2.2/§2.3 below) remain real, researched candidates for the *next* catalog entries once the Ableton pattern is validated in production — exactly per the governing spec's own phasing — but are not shipped in this PR.

§2's research (Ableton, TouchDesigner, ComfyUI ecosystem findings) and §4's supply-chain/version-pinning policy discussion remain accurate and useful for those follow-on entries. §3's design section and §5/§7's phased-plan/open-questions are **historical** — they describe the first draft's approach, superseded by the above. See the implementation itself (`frontend/app/view/mcp/mcp-preload-catalog.ts`, `McpCatalogPicker.tsx`, `mcp-model.ts`, `mcp-manager.tsx`) for what actually shipped.

---

## 1. Problem statement

The Armory's MCP Servers tab (`frontend/app/view/mcp/mcp-manager.tsx`, backed by `McpCatalogModel` in `mcp-model.ts`) is live and functional — a user can already create/edit/delete **global** `McpServer` catalog rows (`mcp.catalog.upsert`/`mcp.catalog.delete`, `agentmux-srv/src/server/app_api/mcp.rs`). But it is pure generic CRUD: three raw fields — Name, Transport, and a freeform Config JSON textarea (`mcp-manager.tsx:94-118`). Registering any real-world server today means hand-typing something like:

```json
{"command": "uvx", "args": ["ableton-mcp"], "env": {}}
```

with no guidance on what `command`/`args` should actually be, no indication that the target app has to already be running, and no warning if the server carries real code-execution risk. This is the same gap `SPEC_EXTERNAL_APP_DRIVING_BLENDER_2026_07_03.md` §5 Phase 1 flagged for Blender specifically ("the actual remaining Phase 1 UX gap, not the plumbing itself") and §4.7 designed a general `AppConnector`/`ConnectorSetupStep` schema to close — but deferred building for a second connector, calling it open question #3 ("does Phase 1 build to that schema from day one... or is a hardcoded single-purpose UI acceptable to ship faster").

The Accounts tab already proves the "curated tile gallery in front of generic CRUD" shape works well in this codebase: `frontend/app/view/accounts/accounts-catalog.ts` defines a static `SERVICE_CATALOG: ServiceTile[]` (GitHub, Google, AWS, Slack, etc.) rendered as clickable tiles in `AccountsGallery.tsx`; picking a tile pre-fills the underlying add-account form instead of leaving the user to construct a raw credential record by hand. This spec asks the MCP Servers tab to get the equivalent: a data-driven preload catalog, seeded with real, verified entries for Ableton Live and TouchDesigner, with a short list of other creative apps as the immediate follow-on.

---

## 2. Research: what actually exists for Ableton Live and TouchDesigner (verified 2026-07-10)

### 2.1 Ableton Live

The ecosystem is mature and, notably, **fragmented** — five actively maintained projects surfaced in a single search:

| Project | Shape | Notes |
|---|---|---|
| [ahujasid/ableton-mcp](https://github.com/ahujasid/ableton-mcp) | Socket-based (TCP, default port **9877**) MCP server ↔ a Remote Script running inside Live | The original and most-starred (~2,500★). Same author and architecture family as `ahujasid/blender-mcp`, the reference implementation cited in the Blender spec §3.1 — expect the same "socket bridge + must marshal onto the app's own thread" shape. |
| [jpoindexter/ableton-mcp](https://github.com/jpoindexter/ableton-mcp) | Same family, expanded | 200+ tools, near-complete Ableton Live Object Model (LOM) coverage; adds a REST API and a Max for Live device; multi-provider (Claude, Ollama, OpenAI, Groq). |
| [uisato/ableton-mcp-extended](https://github.com/uisato/ableton-mcp-extended) | Fork | Broader LOM coverage; explicitly compatible with Claude Desktop, Cursor, Gemini CLI. |
| `hidingwill/AbletonBridge` | Newer (2026-03) | 322 tools; installable via `claude mcp add`. |
| [nozomi-koborinai/ableton-osc-mcp](https://github.com/nozomi-koborinai/ableton-osc-mcp) | **OSC-based**, via the community AbletonOSC add-on | Different transport family entirely — real-time-performance-oriented (OSC favored for low latency), not LOM/socket-JSON. |

**Setup shape (ahujasid original, the de facto reference)**: copy a Remote Script folder into Live's `MIDI Remote Scripts` directory (path varies by OS/version — typically `.../Contents/App-Resources/MIDI Remote Scripts/` or a per-version `User Remote Scripts` path), then in Live's **Preferences → MIDI**, set the Control Surface dropdown to `AbletonMCP`. The MCP bridge process (what AgentMux's `command`/`args` would spawn) then connects to the Remote Script's socket. This is a **multi-step, GUI-driven, manual setup dance** structurally identical to Blender's add-on install (Blender spec §3.3) — not a `pip install` and done.

**Operational shape — same caveat class as Blender**: Live must already be running with the Remote Script loaded and the Control Surface preference set *before* the bridge can connect; there is no way to "launch Live for the user" as part of registering the MCP server. Expect the same "window must stay open, first command sometimes doesn't go through" class of symptom Blender's `bpy.app.timers` polling has (Blender spec §3.3) — not independently verified for Ableton in this pass, but the socket-bridge-into-a-live-GUI-app architecture is the same pattern, so the risk should be assumed and called out rather than assumed absent.

### 2.2 TouchDesigner

Also mature, with a clearer "canonical" implementation:

| Project | Shape | Notes |
|---|---|---|
| [8beeeaaat/touchdesigner-mcp](https://github.com/8beeeaaat/touchdesigner-mcp) | An MCP server (npm package `touchdesigner-mcp-server`) talking HTTP to a **WebServer DAT** component (`mcp_webserver_base.tox`) imported into the user's TD project, listening on `127.0.0.1:9981` by default | MIT license; v1.4.9 as of June 2026. Exposes: create/delete node, call a Python method on a node, execute arbitrary Python in TD, plus documentation-lookup tools (`op.help()`-style). Claude Desktop also has a one-double-click `.mcpb` bundle. |
| [johnsabath/touchdesigner-mcp](https://github.com/johnsabath/touchdesigner-mcp) | Runs *inside* TD itself | General-purpose: execute Python, inspect operators, read/write DATs, capture visual output. |
| Community: `ClaudeBridge.tox` (Derivative forum) | Simpler, prompt-driven | e.g. "create a red box and spin it." |
| Companion skill: "TouchDesigner Guide" | Claude Code Skill, not an MCP server | Enforces a `op.TDAPI` convention to reduce hallucinated operator names/params — worth pairing with the MCP server as a companion Skill, exactly the pattern Blender spec §4.2 recommends (a Skill encoding safety/accuracy guidance alongside the MCP server, not baked into it). |

**Setup shape**: import a `.tox` component into the TD project (drag-and-drop or a documented import step), which starts a WebServer DAT listening locally. The MCP server (`npx`-launched) then talks HTTP to that DAT. Same category of manual, GUI-driven setup as Ableton and Blender: **the project must already be open in TouchDesigner with the component loaded** before the bridge is useful.

**Risk note, sharper than Ableton's**: the 8beeeaaat server's tool list explicitly includes "execute an arbitrary Python script in TouchDesigner" as a first-class tool, not an opt-in extra — closer to Blender's "no guardrails" raw-exec posture (Blender spec §3.2) than to Ableton's LOM-scoped tool surface. Any TouchDesigner catalog entry should carry the same unmissable risk callout Blender's does.

### 2.3 ComfyUI — the interesting counter-example: official server exists, but not for local-first (verified 2026-07-10)

ComfyUI is the sharpest illustration of §4's vendoring-policy problem, because it has **two different official-vs-community stories depending on cloud vs. local**:

- **Comfy Cloud MCP** (official, public beta since 2026-06-29) — `url`/SSE transport straight to `cloud.comfy.org/mcp`, OAuth or API-key auth, workflows run on Comfy's own GPUs. This is a genuinely clean **Tier A** entry: no add-on to install, no "app must already be running" caveat, no local GPU required. First-party, so none of §4's fork-selection problem applies. Good candidate for Phase 0's initial cheap Tier-A validation (§6).
- **`comfy-local-mcp`** (official, wraps `comfy-cli`, validated end-to-end against a live local ComfyUI) — this is the one that would actually matter for a "local-first" catalog entry (drives *your* install, *your* nodes/custom-nodes/models). **Currently in private testing, not publicly available** — cannot be shipped as a catalog template today.
- Because the official local option isn't public, **local-first ComfyUI today means picking a community server**, reproducing the exact multi-fork problem §4 raises for Ableton: [artokun/comfyui-mcp](https://github.com/artokun/comfyui-mcp) (most complete — 108 tools, auto-detects the local install/port, also reaches LAN/VPS/Comfy Cloud from one config), [joenorton/comfyui-mcp-server](https://github.com/joenorton/comfyui-mcp-server) (lightweight), [shawnrushefsky/comfyui-mcp](https://github.com/shawnrushefsky/comfyui-mcp) (notes ComfyUI Desktop uses port 8000 vs. a manual install's 8188 — a platform/install-variant detail any template's `liveCheck` needs to account for, not hardcode).

**Recommendation**: ship *two* ComfyUI catalog entries, not one — "ComfyUI Cloud" (Tier A, official, template points at `cloud.comfy.org/mcp`) and "ComfyUI (local)" (Tier A-ish — no GUI-driven add-on install like Ableton/TD, but does need a running local ComfyUI server, so it's a lighter version of Tier B's `liveCheck` without the multi-step `setupSteps` checklist — default template to `artokun/comfyui-mcp` per §4's "most-starred, pin the version" policy, with a tracked follow-up to swap the local entry over to `comfy-local-mcp` once it exits private testing).

### 2.4 Others (candidate shortlist, not yet deep-researched to the same depth)

Grounded in the same research pass, roughly in priority order for a v1 shortlist:

- **REAPER** (DAW) — three competing integration strategies exist (OSC: low-latency/real-time; ReaScript via `reapy`: deep API, higher latency; file-based bridge: high latency, high reliability, good for batch/async). Good Tier-A candidate (see §4) since REAPER's own OSC support is a stable, first-party protocol, not a third-party add-on to install.
- **SuperCollider** (audio synthesis) — OSC-native, matches AgentMux's existing OSC exposure well.
- **ETC Eos** (lighting console) — `MaybeItsAdam/eos-mcp`, OSC-based. Different category (live show control) but same "professional creative tool with a real-time control surface" shape.
- **Logic Pro** — `koltyj/logic-pro-mcp`. **macOS-only** by definition; flag platform gating explicitly in the catalog schema (§4).
- **Pro Tools** — `skrul/protools-mcp-server`, notable for using Avid's *official* PTSL gRPC API rather than a reverse-engineered bridge — lower supply-chain risk than most of the Ableton forks (§5).
- **Blender** — already fully speced in `SPEC_EXTERNAL_APP_DRIVING_BLENDER_2026_07_03.md`; this spec's catalog mechanism should absorb Blender as its first Tier-B entry rather than duplicate that work.
- Explicitly **out of scope for v1**: narrow hardware bridges (e.g. a single-synth MIDI CC bridge like the MicroFreak MCP found in research) — long-tail, low reuse, better served by the existing generic "+ New MCP server" form than by a maintained catalog entry.

---

## 3. Design: extend the existing catalog pattern, don't invent a new one

No backend/schema change is required. Every preloaded entry still resolves to exactly one `McpServer` row (`id, name, transport, config, is_global`) via the existing `mcp.catalog.upsert` RPC — the catalog is a **frontend curation layer** in front of a primitive that already exists and already works, precisely as the Blender spec's §4.1 established for Blender alone.

### 3.1 Two tiers, reusing Blender spec §4.7's schema for the harder one

Not every app needs the full guided-checklist treatment. Splitting into two tiers keeps the common case cheap:

**Tier A — "one-click template."** For servers that are just a `stdio` command/args pair with no external multi-step setup beyond "have the target app or its CLI installed" (e.g. a REAPER OSC bridge that's a single `pip install && run` with no add-on to import). Clicking the tile pre-fills the *existing* `McpDraft` form (`mcp-model.ts:17-26`) — name, transport, config — and drops the user straight into the same edit view `mcp-manager.tsx` already renders (`draftAtom`/`startEdit` path). No new UI surface, just a new entry point into `startNew()` that seeds the draft from a template instead of `emptyMcpDraft()`.

**Tier B — full "Connector."** For apps needing genuine multi-step manual setup with real installed/running state to verify — Ableton Live (Remote Script copy + Preferences dropdown) and TouchDesigner (`.tox` import) both belong here, as does Blender. Reuse the `AppConnector`/`ConnectorSetupStep` interface exactly as defined in the Blender spec §4.7 (numbered checklist, auto-verified steps wherever a real probe exists, persistent risk callout, troubleshooting accordion, `verifiedAgainst` staleness marker) — do not define a second, slightly-different schema. Ableton and TouchDesigner become the 2nd and 3rd `AppConnector` entries; Blender becomes the 1st once its own Phase 1 lands.

```ts
// Extends the McpDraft shape used by mcp-model.ts's emptyMcpDraft()/startNew().
interface McpPreloadTemplate {
    id: string;                        // "ableton-live", "touchdesigner"
    displayName: string;
    icon: string;
    category: "daw" | "vj-visual" | "lighting" | "3d" | "design";
    blurb: string;                     // one line, tile subtitle
    tier: "A" | "B";
    platforms: ("windows" | "macos" | "linux")[];   // e.g. Logic Pro: ["macos"]
    riskNote?: string;                 // rendered as an unmissable callout, Tier A or B
    verifiedAgainst: string;           // "Ableton Live 12.x + ableton-mcp (ahujasid), checked 2026-07-10"
    sourceUrl: string;                 // the upstream project this template points at

    // Tier A only:
    draftTemplate?: { transport: string; config: string };  // pre-filled McpDraft, name left blank for user

    // Tier B only — identical shape to Blender spec §4.7's AppConnector:
    prerequisites?: string[];
    setupSteps?: ConnectorSetupStep[];
    liveCheck?: { kind: "tcp" | "http"; port: number; path?: string };
    mcpTemplate?: { transport: "stdio" | "url"; command?: string; args?: string[]; urlTemplate?: string };
    troubleshooting?: { symptom: string; explanation: string }[];
}
```

### 3.2 Where it surfaces in the UI

`mcp-manager.tsx` gains a gallery entry point above (or replacing, for the empty state) the current "+ New MCP server" button — e.g. "Browse preloaded servers," opening a tile grid grouped by `category`, mirroring `AccountsGallery.tsx`'s tile layout. Tier A tiles jump straight into the existing draft-edit form (§3.1). Tier B tiles open the guided checklist.

**Open placement question, inherited from the Blender spec:** §4.6/§4.7 of that spec put the *Toolchain* pane's proposed "Connectors" section forward as the natural home for Tier-B checklists (it already does install-detection/live-health-check for other tools). This spec's ask is explicitly scoped to the **Armory's MCP Servers tab** (per the request that started this spec), which is the *global-catalog* surface, not a per-machine tool-detection surface. Recommendation: the Armory MCP Servers tab is the entry point and owns the tile gallery + Tier-A flow; Tier-B's guided checklist can either render inline in the Armory (simpler, keeps everything in one place) or deep-link to the Toolchain pane's Connectors section once that ships (reuses Blender's own build-out, avoids two checklist implementations). **Don't build both** — pick one when Blender's Phase 1 and this spec's Phase 1 are sequenced against each other (see §6).

### 3.3 Global vs. per-agent

The Armory MCP Servers tab is explicitly the **global catalog** view today (`armory.md`: "Global servers — visible to and bindable by any agent," created via `mcp.catalog.upsert`) and has no agent context (`mcp.rs` comment: "no `check_s1`, so there is no agent context to scope by"). Preloaded-server adds from this tab should therefore always create a **global** row via `mcp.catalog.upsert` — consistent with existing semantics, no special-casing needed. An agent then binds it individually via the existing per-agent Agent-setup modal, same as any other global server today.

---

## 4. Risk and supply-chain considerations (compounds per app — new relative to the Blender spec)

The Blender spec's open question #1 ("fork/vendor an existing OSS server, or write a minimal AgentMux-maintained one?") was scoped to a single app. A multi-app catalog makes this decision **N times**, and §2.1 alone surfaces the sharp edge: **five actively competing Ableton Live MCP forks**, with materially different tool counts (from a curated ~50 to 322) and no single obvious "official" choice (there is no first-party Ableton MCP server — unlike Pro Tools' PTSL-based one, §2.3). Deciding this ad hoc per catalog entry means five different trust postures for five different apps, which is hard to reason about and harder to keep current.

**Recommendation:** adopt one documented policy for the whole catalog, not per-entry judgment calls:
1. Prefer the most-starred/longest-maintained implementation as the default template, but pin `command`/`args` to a **specific tagged release or pinned version** (e.g. `uvx ableton-mcp@1.2.0`, not `uvx ableton-mcp@latest`) so an upstream update can't silently change tool behavior or security posture under an already-registered catalog entry.
2. Every Tier B entry (real code-exec risk, per §2.1/§2.2) carries a `riskNote` rendered as a persistent callout — same unmissable treatment as Blender's, not weakened because it's "just a DAW."
3. `verifiedAgainst` is mandatory on every entry and reviewed on a cadence (proposed: whenever a catalog entry's upstream project has a major version bump, or at minimum once per AgentMux release cycle) — the alternative is a catalog that quietly drifts stale, which is worse than no catalog (a stale "verified" template is more misleading than an empty form).
4. Longer-term, revisit whether AgentMux should vendor/fork the highest-priority entries (Ableton, TouchDesigner) into an AgentMux-maintained bridge, the same open question Blender's spec left open (§7.1) — out of scope to decide here, but the same tradeoff applies at greater multiplier.

---

## 5. Phased plan

**Phase 0 — catalog mechanism only, cheapest possible validation.** Ship the `McpPreloadTemplate` data structure, the tile gallery UI, and Tier A wiring (pre-fill → existing form) in `mcp-manager.tsx`, seeded with 1-2 genuinely Tier-A entries — **ComfyUI Cloud (§2.3) is the best available candidate**: official, `url`-transport, no add-on install, no "app must already be running" caveat — to prove the mechanism before investing in Tier B's heavier checklist UI.

**Phase 1 — Ableton Live and TouchDesigner as the first Tier B connectors (the explicit ask).** Build each to the `AppConnector` schema from Blender spec §4.7:
- Ableton: setup steps = copy Remote Script → set Preferences → MIDI → Control Surface; `liveCheck` = TCP probe on port 9877; `mcpTemplate` = `{transport: "stdio", command: "uvx", args: ["ableton-mcp"]}` (exact package/version pinned per §4); troubleshooting seeded from §2.1's "window must stay open" caveat class once independently verified (flagged as unverified in this pass — verify before shipping the troubleshooting copy as fact, don't just port Blender's wording by assumption).
- TouchDesigner: setup steps = import `mcp_webserver_base.tox` into the project; `liveCheck` = TCP/HTTP probe on port 9981; `mcpTemplate` = `{transport: "stdio", command: "npx", args: ["touchdesigner-mcp-server"]}`; `riskNote` covering the raw-Python-exec tool per §2.2, at least as strongly worded as Blender's.
- Both ship a companion Skill (Blender spec §4.2 pattern) encoding the "app must already be open with the bridge component loaded" instruction and macro-tool-first guidance where the upstream server distinguishes macro vs. raw tools.

**Phase 2 — expand the shortlist.** REAPER (full OSC + ReaScript modes, not just Phase 0's minimal cut), SuperCollider, ETC Eos, Logic Pro (with explicit macOS-only platform gating), Pro Tools (lower-risk template given its official PTSL basis, §2.4), ComfyUI local (swap from `artokun/comfyui-mcp` to `comfy-local-mcp` once it exits private testing, §2.3). Blender's own entry migrates into this same catalog once its Phase 1 ships, so the two specs converge on one gallery instead of parallel UIs.

**Phase 3 — future, not committed scope.** Community-submitted or remotely-fetched catalog entries instead of a hardcoded frontend list, once the curated set proves the mechanism and the maintenance model in §4 point 3 is validated at small scale.

---

## 6. Sequencing against the Blender spec

This spec and `SPEC_EXTERNAL_APP_DRIVING_BLENDER_2026_07_03.md` both propose the same `AppConnector` schema and both want to own where Tier-B checklists render (§3.2). They should not ship independently without reconciling:
- If Blender's Phase 1 (Toolchain "Connectors" section) lands first, this spec's Tier B entries should render there too, and the Armory MCP Servers tab becomes purely the discovery/tile-picking surface that deep-links into Toolchain.
- If this spec's Armory-inline approach lands first, Blender's Phase 1 should adopt the same location rather than building a second checklist UI in Toolchain.
- Either order, the `AppConnector`/`ConnectorSetupStep` TypeScript interface should be defined exactly once and imported by both, not redefined in parallel.

---

## 7. Open questions

1. **Checklist UI location** (§3.2, §6) — Armory-inline vs. Toolchain "Connectors" deep-link. Needs a product decision before Phase 1 starts, ideally made jointly with whoever picks up the Blender spec's Phase 1.
2. **Version-pinning policy enforcement** (§4.1) — hand-maintained per entry, or a lightweight CI check that flags when a pinned template version falls N releases behind upstream?
3. **Ableton's operational caveats** (§2.1) — the "window must stay visible" class of symptom is assumed-by-architecture-similarity to Blender, not independently verified. Needs real hands-on testing against `ahujasid/ableton-mcp` before Phase 1 troubleshooting copy ships as fact.
4. **Fork choice for Ableton** (§2.1, §4) — ahujasid original (most popular, ~50 tools) vs. a higher-tool-count fork (200+/322 tools). More tools is more surface area and more risk; recommend defaulting the catalog entry to the original and treating forks as advanced/manual (still reachable via the generic form), not a second preloaded tile.
5. **Should Tier A exist at all, or should everything just be Tier B with an empty `setupSteps`?** A single schema is simpler to build; the two-tier split exists to keep the *UI* cheap for the common case. Worth revisiting once Phase 0 shows how many real candidates are actually Tier-A-shaped (§2.4 suggests most creative-app integrations are Tier B by nature — the GUI-app-must-already-be-running pattern recurs everywhere in this research; ComfyUI Cloud, §2.3, is the cleanest Tier-A counter-example found).
6. **ComfyUI local entry staleness** (§2.3) — shipping a community-server default (`artokun/comfyui-mcp`) for a slot that's really meant for the official `comfy-local-mcp` needs an explicit tracked follow-up so the swap actually happens once that exits private testing, rather than the community default silently becoming permanent.

## 8. References

- Internal: `docs/specs/SPEC_EXTERNAL_APP_DRIVING_BLENDER_2026_07_03.md` (source of the `AppConnector`/`ConnectorSetupStep` schema this spec reuses), `agentmux-srv/src/backend/storage/mcp_servers.rs`, `agentmux-srv/src/server/app_api/mcp.rs`, `frontend/app/view/mcp/mcp-manager.tsx`, `frontend/app/view/mcp/mcp-model.ts`, `frontend/app/view/accounts/accounts-catalog.ts` (the tile-gallery-in-front-of-CRUD precedent), `src/content/docs/armory.md` (Armory MCP Servers tab description, agentmux-docs repo).
- [ahujasid/ableton-mcp](https://github.com/ahujasid/ableton-mcp)
- [jpoindexter/ableton-mcp](https://github.com/jpoindexter/ableton-mcp)
- [uisato/ableton-mcp-extended](https://github.com/uisato/ableton-mcp-extended)
- [nozomi-koborinai/ableton-osc-mcp](https://github.com/nozomi-koborinai/ableton-osc-mcp)
- [8beeeaaat/touchdesigner-mcp](https://github.com/8beeeaaat/touchdesigner-mcp)
- [johnsabath/touchdesigner-mcp](https://github.com/johnsabath/touchdesigner-mcp)
- [MaybeItsAdam/eos-mcp](https://github.com/MaybeItsAdam/eos-mcp)
- [ChatForest: Music & Audio Production MCP Servers](https://chatforest.com/reviews/music-audio-production-mcp-servers/)
- [MCP.Directory: TouchDesigner MCP Complete Guide (2026)](https://mcp.directory/blog/touchdesigner-mcp-complete-guide-2026)
- [Skywork: REAPER DAW MCP Server](https://skywork.ai/skypage/en/reaper-daw-mcp-server-ai-engineer/1981641441844236288)
