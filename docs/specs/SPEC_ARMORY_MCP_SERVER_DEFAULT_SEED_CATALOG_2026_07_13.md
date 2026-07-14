# Spec: Default Seed Catalog for the Armory's MCP Servers Tab

**Date:** 2026-07-13
**Author:** AgentX
**Type:** Research + proposal. No code shipped yet.
**Purpose:** The Armory's **MCP Servers** tab is empty on every fresh install and stays empty until a user hand-types a server or clicks through the one-entry "Browse catalog" picker. This spec researches the current seeding gap, surveys the external MCP server ecosystem for what's worth seeding, and proposes a two-tier architecture — real DB-seeded rows for safe/local servers, catalog-picker entries for anything needing a credential — driven by a hard constraint discovered during this research: **every `is_global` MCP server is auto-injected into every agent's `.mcp.json` at launch, with no per-agent opt-in and no "disabled" flag.**

---

## 1. Problem statement

`frontend/app/view/mcp/mcp-manager.tsx:70` renders `"No MCP servers yet"` on a fresh install, and stays that way indefinitely — `db_mcp_servers` (`agentmux-srv/src/backend/storage/migrations.rs:437-446`) is created empty by the v10 migration and **no code path anywhere in the backend ever inserts a seed row into it.** Confirmed by grep: no `INSERT INTO db_mcp_servers` exists outside the user-driven `mcp_server_upsert*` methods (`agentmux-srv/src/backend/storage/mcp_servers.rs:149-282`), and none of `agent_seed.rs`, `identity/migration.rs`, or any `m00XX_*.rs` migration touches this table.

The one thing in the codebase that looks like a seed list is not one:

- `frontend/app/view/mcp/mcp-preload-catalog.ts` — `MCP_PRELOAD_CATALOG: McpPreloadEntry[]`, currently **one entry** (Ableton Live, `:40-51`). This is a UI picker: selecting a tile calls `McpCatalogModel.startFromCatalog()` (`mcp-model.ts:140`), which only **pre-fills the add-server draft form** — the user still has to review and click Save (`mcp.catalog.upsert`) before anything is written. Its own doc comment is explicit that this is deliberately a frontend-only source of truth, not a DB seed (`mcp-preload-catalog.ts:11-18`).
- A sibling proposal, `docs/specs/SPEC_ARMORY_PRELOADED_CREATIVE_MCP_CONNECTORS_2026_07_10.md`, expands this exact picker with niche creative-software connectors (Ableton, TouchDesigner, ComfyUI, REAPER…) and is explicit that "no backend/schema change is required" for that work (§3, line 97) — by design, it never makes the list non-empty by default; a user who wants none of those tools still opens the Armory to a blank page.

**What this spec adds that neither of those covers:** a small, curated set of general-purpose, credential-free, local dev-productivity servers (filesystem, git, fetch, sequential-thinking) that are useful to essentially every coding-agent user, seeded as real rows so the Armory is non-empty and agents are immediately more capable on first launch — the same "get productive immediately" bar the seeded agent-definition manifest (`agent-seed.json`) already sets for agent templates.

**Sibling work, same day:** `docs/specs/SPEC_ARMORY_PHASE5_CONSOLIDATION_AND_SKILL_SEEDING_2026_07_13.md` §4 independently researched the identical problem shape for the Armory's **Skills** tab (`db_skills WHERE is_global=1` is also empty on every fresh install, also has no seed path) and reached a simpler implementation shape than this spec's first draft did — see §5's revision below, which adopts it.

---

## 2. The constraint that shapes everything else: `is_global` means "on for every agent, unconditionally"

This was the single most important fact surfaced during codebase research, and it rules out the naive "just seed everything as global rows" approach.

`Store::mcp_server_list(agent_id)` (`agentmux-srv/src/backend/storage/mcp_servers.rs:54-83`) is the query every agent's `.mcp.json` is built from:

```sql
SELECT ... FROM db_mcp_servers s
WHERE s.is_global = 1
   OR s.id IN (SELECT mcp_id FROM db_agent_mcp_ref WHERE agent_id = ?1)
```

Its own doc comment says it plainly: *"A global server is always visible to every agent"* (`:30`). And the call site that actually builds the launch-time `.mcp.json`, `agentmux-srv/src/server/app_api/agent_open.rs:583-593`, confirms there is no filtering step in between:

> "v1 MCP: same rule. **Globals + synthetic `agentmux` are always emitted**; the legacy blob's user servers are merged in ONLY when the agent has no own ref-bound servers."

There is no `enabled`/`disabled` column on `db_mcp_servers`, and no per-agent "I haven't bound this yet, don't start it" gate for global rows — `is_global = true` and "every agent tries to spawn/probe this at every launch" are the same thing today. This is fine for the servers this spec calls **Tier A** (stdio, fully local, no credentials — a missing/misconfigured env var can't happen because there are no env vars to misconfigure). It is actively harmful for anything needing an API key or OAuth token: seeding, say, a GitHub or Postgres server as a global row would mean **every agent on every launch** attempts to start a process or reach a URL that fails immediately for lack of a credential, on every single agent — a novel error introduced at seed time, not something the user opted into.

This isn't an MCP-specific quirk — it's a shared property of every `is_global`-flagged Armory primitive. `SPEC_ARMORY_PHASE5_CONSOLIDATION_AND_SKILL_SEEDING_2026_07_13.md` §4.1 independently found the identical mechanism for `db_skills`: *"Global skills need no per-agent bind step to take effect... unions every agent's own `db_agent_skills_ref` rows with all `is_global = 1` rows automatically."* The difference is consequence, not mechanism: a global Skill is inert prompt text with no credential and no execution surface, so "on for everyone, unconditionally" is exactly what that spec wants and ships without a tiering split. A global MCP server can be an arbitrary local process or a networked call requiring a secret, so the same mechanism that's harmless for Skills is the reason this spec needs one.

This is why §4 below is a two-tier design rather than one seed list.

---

## 3. A seed idiom already in the codebase — and why this spec doesn't fully reuse it

`agentmux-srv/src/backend/agent_seed.rs` already solves "ship a curated default set, on first launch, without clobbering user edits" for agent definitions and memory bundles. It's worth understanding in full because it's the natural first instinct for this problem — this spec's own first draft proposed extending it directly — even though §5 ultimately adopts a lighter mechanism instead, for consistency with the sibling Skills-seeding spec written the same day. Understanding what `agent_seed.rs` does (and which parts of its complexity are and aren't earned) is what makes that simplification a deliberate choice rather than an oversight:

- **Embedded manifest:** `const SEED_MANIFEST: &str = include_str!("../../agent-seed.json")` (`:138`) — parsed once at startup, not a runtime fetch.
- **First-launch seed:** `auto_seed_on_startup` (`:299`) calls `seed_agents` when `agent_def_count() == 0` — a one-shot bulk insert, skipping any id already present (`:159-162`).
- **Reseed-on-manifest-change:** `reseed_if_needed` (`:359`) runs on every subsequent startup, but only actually mutates anything if it detects a real difference (new manifest id, or an identity field like `description` changed, `:374-386`). When it does mutate, it **preserves user-editable fields** on existing rows — provider, agent_type, environment, shell, auto_start, hide-state — updating only the seed-owned identity fields (`:445-467`). New ids always start visible (`user_hidden = 0`, `:439`); ids dropped from the manifest get deleted (`:475-482`).
- **Known, accepted tradeoff in that design:** deletion isn't tracked as a tombstone. If a user hard-deletes a seeded row and the manifest's identity fields later change for an unrelated reason, `reseed_if_needed`'s loop treats the (now-absent) id as "not yet created" and re-inserts it (`:469-472`, the `None => insert` branch). The codebase's answer for agent templates is `user_hidden` — a soft-hide the user can set instead of a hard delete, which *is* preserved across reseeds (`:466`, and both regression tests at `agent_seed.rs:553-606`).

The `is_seeded`/`user_hidden`/reseed-on-change machinery exists in `agent_seed.rs` because agent templates genuinely need it: templates get added and removed across releases, users customize provider/shell/environment on top of a seeded identity, and losing that customization on every restart would be a real regression. MCP servers don't have the equivalent "user customizes on top of a seed" case (§5) — a seeded Tier A row's command/args aren't something a user meaningfully tweaks in place, they either keep it, delete it, or replace it with their own. That's the concrete reason §5 lands on a lighter mechanism instead of porting this one wholesale: the extra machinery here is solving a problem MCP seeding doesn't actually have.

---

## 4. Design: two tiers, two different mechanisms

### Tier A — no credential, fully local, safe to seed as real `is_global = true` DB rows

Because §2 established that global = auto-on-for-everyone, Tier A is deliberately restricted to servers where "on for everyone, always" has no failure mode: no API key, no OAuth, no network dependency beyond `npx`/`uvx` resolving a package the first time. These become real `db_mcp_servers` rows via a new seed pass — mechanism detailed in §5.

| Server | Purpose | Command | Transport | Why safe to auto-enable |
|---|---|---|---|---|
| **Filesystem** | Sandboxed read/write/search within allowed dirs | `npx -y @modelcontextprotocol/server-filesystem <dir>` | stdio | No creds; dir allowlist is the only config, defaults to the agent's own working directory |
| **Git** | Local repo read/search/diff/log operations | `uvx mcp-server-git` | stdio | No creds; operates on whatever repo the agent's cwd is in |
| **Fetch** | Fetch + convert a web page to markdown | `npx -y @modelcontextprotocol/server-fetch` | stdio | No creds; outbound-only, no stored state |
| **Sequential Thinking** | Structured, revisable step-by-step reasoning scratchpad | `npx -y @modelcontextprotocol/server-sequential-thinking` | stdio | No creds, no I/O outside the process itself |
| **Memory** | Cross-session knowledge-graph memory | `npx -y @modelcontextprotocol/server-memory` | stdio | No creds; local file-backed state |
| **Playwright** | Browser automation via accessibility-tree snapshots (Microsoft-official) | `npx @playwright/mcp@latest` | stdio | No creds; downloads a browser binary on first use but needs no secret |
| **Context7** | Live, version-specific library docs — reduces hallucinated APIs (Upstash) | `npx -y @upstash/context7-mcp` | stdio | No key required for the free tier; the closest thing to a de-facto standard for coding-agent doc lookup found in this research |

All seven are `npx`/`uvx`-launched, meaning the *seed row* itself needs no network call — only the first real invocation triggers a package fetch, same cold-start cost every hand-typed stdio server already has today.

**Explicitly excluded from Tier A, with reasons:**
- **SQLite** (reference server) — has a known, unpatched SQL-injection / prompt-injection vector; Anthropic declined to fix it and archived the server rather than patch it. Never seed this by default.
- **Puppeteer** — archived/no-security-fixes; superseded by Playwright above. Don't seed a deprecated server when a maintained equivalent exists.
- **Everything** / **Time** (reference servers) — real, maintained, but demo/utility-grade rather than "every coding session benefits from this." Left as easy manual adds, not seeded.

### Tier B — needs an API key or OAuth: catalog-picker entries only, never auto-seeded as global rows

GitHub, Postgres, Brave Search, Linear, Sentry, and Notion are all high-value for a coding agent, and all need a secret this spec cannot supply at seed time (there is no account/credential-templating mechanism in `McpServer.config` today — confirmed by grep, the config field is a literal JSON blob, not a template referencing an identity account). Per §2, seeding these as `is_global = true` would break every agent's launch with a missing-credential error the user never asked for.

Instead, these are added to the **existing** `MCP_PRELOAD_CATALOG` picker (`mcp-preload-catalog.ts`) — the same one-click-prefill, user-reviews-and-saves flow already shipped for Ableton Live. No schema change, no seed engine involvement; the user pastes their own token into the pre-filled config before clicking Save, at which point it becomes a real (and, at that point, correctly configured) global row.

| Server | Purpose | Package / endpoint | Transport | Credential |
|---|---|---|---|---|
| **GitHub** | Repos, issues, PRs, code search | `ghcr.io/github/github-mcp-server` (Docker) or remote `https://api.githubcopilot.com/mcp/` | local stdio or remote http | `GITHUB_PERSONAL_ACCESS_TOKEN`, or OAuth for the remote endpoint |
| **Postgres MCP Pro** | Read/write Postgres, index tuning, explain plans | `crystaldba/postgres-mcp` (Docker) | stdio | `DATABASE_URI` connection string |
| **Brave Search** | Web/image/video/news search | `npx -y @brave/brave-search-mcp-server --transport http` | stdio or http | `BRAVE_API_KEY` (free tier: 2,000 queries/mo) |
| **Linear** | Issue creation/update, sprint search | remote `https://mcp.linear.app/mcp` via `npx -y mcp-remote ...` | streamable HTTP | OAuth 2.1 + PKCE |
| **Sentry** | Query/triage errors from the agent | `npx @sentry/mcp-server@latest --access-token=...` | remote-first, stdio supported | Access token or OAuth |
| **Notion** | Read/write pages and databases | `npx @notionhq/notion-mcp-server` (local) or OAuth remote | stdio or OAuth remote | `NOTION_TOKEN` or OAuth |

Six entries, matching the scale of Tier A. `Postgres` uses the actively-maintained `crystaldba/postgres-mcp` rather than Anthropic's own archived `server-postgres` (which is read-only and unmaintained). This list is deliberately smaller than the ~15-server survey in §7 — the full survey exists so a future pass can extend either tier without re-researching from scratch, not because every surveyed server belongs in v1.

---

## 5. Seed mechanism — revised to match the sibling Skills-seeding convention

**This section's first draft proposed a schema migration** (`is_seeded`/`user_hidden` columns on `db_mcp_servers`) plus a `reseed_if_needed`-style engine mirroring `agent_seed.rs` exactly. Revised after reading `SPEC_ARMORY_PHASE5_CONSOLIDATION_AND_SKILL_SEEDING_2026_07_13.md` §4.3, which independently solved the identical "empty global Armory catalog" problem for Skills the same day and landed on something lighter — two independent research passes converging on avoiding the heavier design is itself a signal worth taking. This spec adopts that shape rather than the original draft:

- **No schema migration.** No new columns on `db_mcp_servers`. A seeded row is, after the one-time seed runs, indistinguishable from a user-created one — same as the Skills design. This sidesteps the entire "don't resurrect a user-deleted seeded row" problem `agent_seed.rs`'s `is_seeded`/`user_hidden` pair exists to solve (§3): if there's no recurring reseed trigger, there's nothing that could resurrect anything.
- **A one-time seed, not a recurring reseed.** Gate on the same condition the Skills spec proposes: seed only if `db_mcp_servers WHERE is_global = 1` is empty **and** this is a fresh install (reuse whatever "fresh install" signal already exists elsewhere in the codebase — e.g. however first-run/default-channel state is detected today — rather than inventing a second one). A user who deletes a seeded server later just has one fewer server; nothing ever tries to put it back. This is a real behavior tradeoff versus the original draft's reseed-on-manifest-change (a future AgentMux release that improves the seeded `filesystem` command, say, won't retroactively reach existing installs) — accepted here for consistency with the sibling spec and because it's simpler and strictly safer; §9 keeps a path open to add versioned reseeding later if that tradeoff turns out to matter in practice.
- **Reuse the `config/*.json` static-file convention**, not a Rust-`include_str!` manifest: new `agentmux-srv/src/config/starter-mcp-servers.json`, an array of `{name, transport, config}` objects (no `id`/`is_global`/timestamps — assigned at insert time, mirroring the Skills spec's §4.3 point 1 exactly).
- **Insert via the existing validated path**, not hand-rolled SQL: `mcp_server_upsert_unique_global` (`mcp_servers.rs:255-282`) already enforces catalog-wide name uniqueness and sets `is_global = 1` — call it once per manifest entry, warn-and-skip on a name collision (the `seed_memories` precedent, `agent_seed.rs:278-291`) rather than aborting the whole batch, so a user who already hand-created a server named `filesystem` doesn't get it silently clobbered.

```json
[
  { "name": "filesystem", "transport": "stdio",
    "config": { "command": "npx", "args": ["-y", "@modelcontextprotocol/server-filesystem"] } }
]
```

**Open question, shared verbatim with the sibling spec's §4.3:** should this seed run unconditionally on every fresh install, or be opt-in (e.g. a "load starter servers" action in the Armory MCP tab's empty state)? Unconditional matches "pre-populate" and is simpler; an unconditional silent DB write on first launch is a bigger behavioral change than a button click. Flagging as a real product decision rather than assuming either default — and note this decision should almost certainly be made **once, for both catalogs together** (Skills and MCP servers), not independently per spec, since a user's expectation of "does the Armory auto-populate itself or not" should be consistent across tabs.

**Frontend:** no new fields needed on `McpServer`/`McpServerListItem`/`McpServerCatalogItem` under this revised design — a seeded row uses the exact same edit/delete affordances `mcp-manager.tsx` already has for any global row.

---

## 6. Explicit non-goals

- **No live catalog sync from an external registry.** The official MCP Registry (`registry.modelcontextprotocol.io`) is metadata-only and still preview/API-frozen; directories like Smithery/Glama/mcp.so trade curation for coverage and, in Smithery's case, add a hosting-infra trust surface (a real path-traversal incident occurred there in 2025, patched within days). A hardcoded, human-curated manifest — reviewed the same way `agent-seed.json` already is — is the right v1 shape; revisit only if the manifest becomes a maintenance burden at meaningfully larger scale.
- **No cross-provider `.mcp.json` translation.** `build_mcp_config_from_refs` (`agentmux-srv/src/backend/agent_config.rs:320-395`) writes exactly one `.mcp.json` per agent regardless of `agent.provider` — this is Claude Code's native config format. `cli-catalog.ts` claims `mcpSupport` for Codex/Gemini/Qwen/etc., but those CLIs' real MCP config lives elsewhere (e.g. Codex CLI reads `~/.codex/config.toml`, not a project `.mcp.json`) and nothing in this codebase special-cases that today. Seeding more servers into `.mcp.json` doesn't make this gap worse, but it doesn't fix it either — flagged here so it isn't mistaken for in-scope.
- **No secret-templating in `McpServer.config`.** Tier B staying catalog-only (§4) is a direct consequence of this — there is no mechanism today for a seeded row's `config` to reference an identity account's stored credential rather than a literal string. Building one is a real, separable feature (and would upgrade Tier B from "picker, user pastes a token" to "picker, user picks an already-connected account") but is out of scope for making the list non-empty.
- **Version pinning is out of scope for Tier A.** Unlike the creative-connector spec's `uvx ableton-mcp@1.2.0`-style pinning recommendation (relevant there because those servers carry real code-exec risk against a user's creative-app state), Tier A's seven servers are Anthropic/Microsoft/Upstash-maintained references with a low-risk, well-scoped tool surface; `npx -y <pkg>@latest`-style resolution is consistent with how every other hand-typed stdio server in this codebase already works. Worth revisiting only if a Tier A server ever ships a breaking change that surprises users.

---

## 7. Full ecosystem survey (reference for future tier expansion, not all shipped in v1)

Researched mid-2026 state of the MCP ecosystem, for anyone extending either tier later:

- **`modelcontextprotocol/servers`** now holds only steering-group-maintained reference servers: Everything, Fetch, Filesystem, Git, Memory, Sequential Thinking, Time. Latest tag `2026.7.4`.
- **`modelcontextprotocol/servers-archived`** — no-security-fixes archive of servers Anthropic no longer maintains as references: AWS KB Retrieval, (old) Brave Search, EverArt, (old) Git dup, (old) GitHub, GitLab, Google Drive, Google Maps, (old) PostgreSQL, Puppeteer, Redis, (old) Sentry, (old) Slack, **SQLite (unpatched SQLi vuln)**. The general pattern: most SaaS-integration categories were superseded by vendor-official servers outside Anthropic's monorepo (GitHub → `github/github-mcp-server`, Brave → `brave/brave-search-mcp-server`, Notion → `makenotion/notion-mcp-server`, Linear → `mcp.linear.app`, Sentry → `mcp.sentry.dev`).
- **Registries/directories surveyed** (see §6 for why none is used directly): official Registry (~9,652 servers, metadata-only), PulseMCP (11,840+, human-curated), Smithery (7,000+, hosted execution, had a patched security incident), Glama (21,000+, largest but least curated), mcp.so (19,700+, unvetted), Docker MCP Catalog (200+, strongest security posture — signed SBOMs, sandboxing, credential manager — worth a second look if AgentMux ever containerizes agent MCP execution by default).
- **Security baseline worth remembering when extending either tier:** a scan of 8,000+ public registry servers found 36.7% with SSRF issues, 43% with unsafe command-execution paths, 41% with zero authentication. Popularity alone isn't a safety signal — prefer official/vendor-maintained servers over community forks where one exists (the Ableton-connector spec's §2.1 "five competing forks, no obvious winner" problem is the cautionary example of what happens when no official option exists).
- Additional servers noted as high-value but deliberately left out of both tiers for now: **Supabase MCP** (official, OAuth-remote, strong candidate for a future Tier B slot if Supabase usage among AgentMux's users turns out to be common), **Slack** (comms-oriented rather than "get productive immediately" for a coding loop — good Tier B candidate on a later pass), **Google Drive** (archived reference, no confirmed current official replacement identified in this research pass — needs its own verification before it's trustworthy enough to catalog).

---

## 8. Phased plan

**Phase 1 — Tier A backend seed.** `agentmux-srv/src/config/starter-mcp-servers.json` with the 7 Tier A entries, a one-time seed step gated on "no global MCP servers exist yet + fresh install" per §5, inserting via `mcp_server_upsert_unique_global`. No schema change, no new frontend fields. This alone makes the Armory non-empty by default and every new agent launch immediately capable of local file/git/fetch/reasoning/memory/browser/doc-lookup tool use with zero user setup. Should land alongside (or immediately after) the sibling spec's Skills-seeding PR B, sharing whatever "fresh install" gate and unconditional-vs-opt-in decision (§5) that PR settles on, rather than each spec answering it independently.

**Phase 2 — Tier B catalog expansion.** Add the six §4 entries to `MCP_PRELOAD_CATALOG`, following the exact shape the Ableton entry already established (`prereqNote` explaining what credential is needed and where to get it, `docsUrl` pointing at the upstream project). No schema change needed for this phase — it's purely additive to an existing, already-shipped mechanism.

**Phase 3 — not committed scope.** Secret-templating for `McpServer.config` (§6) to let Tier B servers reference an already-connected identity account instead of a hand-pasted token; revisit registry/directory sync (§6) only if manual manifest maintenance becomes a real bottleneck at a meaningfully larger catalog size.

---

## 9. Open questions

1. **Is one-time-only seeding (§5) the right call long-term?** It's simpler and strictly safer than reseed-on-manifest-change today, but it also means a future fix to a Tier A entry's command/args never reaches an install that seeded before the fix shipped. If that turns out to matter in practice, the `is_seeded`/`user_hidden` + versioned-reseed design in §3 is still there to adopt later — not proposed for v1, but not thrown away either.
2. **Should Tier A's seed be skippable at first-run** (e.g. for a locked-down/offline environment where even `npx -y <pkg>` package resolution on first invocation is undesirable)? `agent_seed.rs` has no such escape hatch for agent templates today, so the precedent is "no," but MCP servers do outbound network I/O on first real use in a way agent templates don't — worth a explicit product decision rather than silently inheriting the agent-seed precedent.
3. **Filesystem server's default allowed-directory.** `@modelcontextprotocol/server-filesystem` takes an allowlisted directory as a CLI arg (§4's table shows `<dir>` unfilled) — seeding a literal path is environment-specific and can't be baked into a static manifest the way the other six Tier A commands can. Needs either a per-agent-cwd default computed at config-build time (extending `build_mcp_config_from_refs` to substitute a placeholder) or shipping this one entry with an empty/unset arg and a `prereqNote`-equivalent nudging the user to fill it in — decide before Phase 1 ships this specific entry.
4. **Filesystem's directory arg is the one Tier A entry that can't ship as a static manifest value without answering question 3 first** — the other six need no per-install customization, this one does. If question 3 lands on "ship it anyway with an empty arg," confirm the resulting server fails safely (a probe error the user can see, not a silent no-op) rather than, worse, defaulting to some unexpectedly broad directory.

---

## 10. References

- Internal: `agentmux-srv/src/backend/storage/mcp_servers.rs`, `agentmux-srv/src/backend/storage/migrations.rs:437-462`, `agentmux-srv/src/server/app_api/mcp.rs`, `agentmux-srv/src/backend/mcp_probe.rs`, `agentmux-srv/src/backend/agent_config.rs:252-395`, `agentmux-srv/src/server/app_api/agent_open.rs:575-615`, `agentmux-srv/src/backend/agent_seed.rs` (the seed idiom §3 evaluates and §5 partially departs from), `frontend/app/view/mcp/mcp-manager.tsx`, `frontend/app/view/mcp/mcp-model.ts`, `frontend/app/view/mcp/mcp-preload-catalog.ts`, `frontend/app/view/mcp/McpCatalogPicker.tsx`, `specs/SPEC_V1_MCP_SKILLS_PRIMITIVES_2026_06_30.md` (governing schema spec), `docs/specs/SPEC_MCP_INTEGRATION_PARITY_ABLETON_PILOT_2026_07_08.md` (probe + one-click-install groundwork this spec builds on), `docs/specs/SPEC_ARMORY_PRELOADED_CREATIVE_MCP_CONNECTORS_2026_07_10.md` (sibling proposal — niche creative connectors via the same picker mechanism Tier B reuses here), `docs/specs/SPEC_ARMORY_PHASE5_CONSOLIDATION_AND_SKILL_SEEDING_2026_07_13.md` (independent same-day sibling spec — identical problem shape for Skills, source of the lighter §5 seed mechanism and the shared §2 `is_global`-auto-inject finding), `docs/specs/archive/REPORT_ARMORY_FEATURE_STATUS_2026_07_07.md` (confirms seeding was not among the six tracked Armory/MCP gaps as of #1960 — genuinely unaddressed territory).
- [modelcontextprotocol/servers](https://github.com/modelcontextprotocol/servers)
- [modelcontextprotocol/servers-archived](https://github.com/modelcontextprotocol/servers-archived)
- [Official MCP Registry](https://registry.modelcontextprotocol.io/) / [registry preview announcement](https://blog.modelcontextprotocol.io/posts/2025-09-08-mcp-registry-preview/)
- [github/github-mcp-server](https://github.com/github/github-mcp-server)
- [microsoft/playwright-mcp](https://github.com/microsoft/playwright-mcp)
- [upstash/context7](https://github.com/upstash/context7)
- [brave/brave-search-mcp-server](https://github.com/brave/brave-search-mcp-server)
- [crystaldba/postgres-mcp](https://github.com/crystaldba/postgres-mcp)
- [makenotion/notion-mcp-server](https://github.com/makenotion/notion-mcp-server)
- [Linear MCP docs](https://linear.app/docs/mcp)
- [Sentry MCP docs](https://docs.sentry.io/) (`mcp.sentry.dev`, `@sentry/mcp-server`)
- [Supabase MCP docs](https://supabase.com/docs/guides/ai-tools/mcp)
- [Docker MCP Catalog & Toolkit](https://docs.docker.com/ai/mcp-catalog-and-toolkit/)
- [WorkOS: Everything your team needs to know about MCP in 2026](https://workos.com/blog/everything-your-team-needs-to-know-about-mcp-in-2026)
- [SSOJet: Best MCP Servers 2026](https://ssojet.com/blog/best-mcp-servers-2026)
- [TrueFoundry: MCP registries comparison](https://www.truefoundry.com/blog/best-mcp-registries)
